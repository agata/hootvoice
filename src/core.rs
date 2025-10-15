// Core facade for high-level control; detailed I/O/transcription/output logic
// lives in submodules under `core/`.

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use whisper_rs::{WhisperContext, WhisperContextParameters};

// use std::time::Instant; // not used in this module

mod audio_io;
mod output;
mod postprocess;
mod transcriber;
use crate::audio::VadStrategy;
use crate::dictionary::DictionaryEntry;
use crate::llm::{LlmPostProcessSettings, LlmPostProcessor};
use crate::transcription::ensure_model;
use crate::transcription::WhisperOptimizationParams;
use crate::utils::sound;
use hound::{SampleFormat as WavSampleFormat, WavSpec, WavWriter};
pub use output::BehaviorOptions;
use std::sync::atomic::AtomicU32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SimpleRecState {
    Idle,
    Recording,
    Processing,
    PostProcessing,
    Busy,
}

pub type LogCallback = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
pub struct WhisperCore {
    pub ctx: Arc<Mutex<Arc<WhisperContext>>>,
    pub state: Arc<Mutex<SimpleRecState>>,
    pub log_callback: Arc<Mutex<Option<LogCallback>>>,
    audio_buffer: Arc<Mutex<Vec<f32>>>,
    stop_flag: Arc<Mutex<bool>>,
    processing_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    current_model_path: Arc<Mutex<std::path::PathBuf>>,
    preferred_output_device: Arc<Mutex<Option<String>>>,
    llm_settings: Arc<Mutex<LlmPostProcessSettings>>,
    #[cfg(target_os = "macos")]
    front_app_before_paste: Arc<Mutex<Option<String>>>,

    // Components split by responsibility
    audio: audio_io::AudioIO,
    trans: transcriber::Transcriber,
    out: output::OutputBehavior,
}

impl WhisperCore {
    pub fn new(model_path: &Path) -> Result<Self> {
        whisper_rs::install_logging_hooks();
        ensure_model(model_path).context("download Whisper model")?;

        let model_path_str = model_path
            .to_str()
            .ok_or_else(|| anyhow!("invalid model path (non-UTF-8)"))?;
        let ctx =
            WhisperContext::new_with_params(model_path_str, WhisperContextParameters::default())
                .with_context(|| format!("load Whisper model: {}", model_path.display()))?;

        // Shared state (Arc/Mutex)
        let ctx_arc = Arc::new(Mutex::new(Arc::new(ctx)));
        let state = Arc::new(Mutex::new(SimpleRecState::Idle));
        let log_callback = Arc::new(Mutex::new(None));

        let recording_thread = Arc::new(Mutex::new(None));
        let audio_buffer = Arc::new(Mutex::new(Vec::new()));
        let stop_flag = Arc::new(Mutex::new(false));
        let processor = Arc::new(Mutex::new(None));
        let processing_thread = Arc::new(Mutex::new(None));
        let last_processed_len = Arc::new(Mutex::new(0));
        let record_started_at = Arc::new(Mutex::new(None));
        let behavior = Arc::new(Mutex::new(BehaviorOptions {
            use_clipboard: true,
            auto_paste: true,
        }));
        let current_model_path = Arc::new(Mutex::new(model_path.to_path_buf()));
        let language = Arc::new(Mutex::new(None));
        let preferred_input_device = Arc::new(Mutex::new(None));
        let preferred_input_device_index = Arc::new(Mutex::new(None));
        let preferred_input_host = Arc::new(Mutex::new(None));
        let preferred_output_device = Arc::new(Mutex::new(None));
        let input_gain = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let whisper_optimization = Arc::new(Mutex::new(WhisperOptimizationParams::default()));
        let chunk_strategy = Arc::new(Mutex::new(VadStrategy::Normal));
        let dictionary_entries = Arc::new(Mutex::new(Vec::new()));
        let current_session = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let auto_stop_silence_secs = Arc::new(Mutex::new(10.0));
        let max_record_secs = Arc::new(Mutex::new(600.0));
        let llm_settings = Arc::new(Mutex::new(LlmPostProcessSettings::default()));
        let llm_processor = Arc::new(LlmPostProcessor::new());
        let postprocess_engine = postprocess::PostProcessEngine::new(
            llm_settings.clone(),
            llm_processor.clone(),
            state.clone(),
        );
        #[cfg(target_os = "macos")]
        let front_app_before_paste = Arc::new(Mutex::new(None));

        // Components (share the same Arcs)
        let audio = audio_io::AudioIO::new(
            audio_buffer.clone(),
            stop_flag.clone(),
            recording_thread.clone(),
            preferred_input_device.clone(),
            preferred_input_device_index.clone(),
            preferred_input_host.clone(),
            input_gain.clone(),
            current_session.clone(),
        );
        let trans = transcriber::Transcriber::new(
            ctx_arc.clone(),
            processor.clone(),
            processing_thread.clone(),
            last_processed_len.clone(),
            record_started_at.clone(),
            language.clone(),
            whisper_optimization.clone(),
            chunk_strategy.clone(),
            dictionary_entries.clone(),
            auto_stop_silence_secs.clone(),
            max_record_secs.clone(),
            postprocess_engine.clone(),
            state.clone(),
        );
        let out = output::OutputBehavior::new(
            behavior.clone(),
            #[cfg(target_os = "macos")]
            front_app_before_paste.clone(),
            log_callback.clone(),
        );

        Ok(Self {
            ctx: ctx_arc,
            state,
            log_callback,
            audio_buffer,
            stop_flag,
            processing_thread,
            current_model_path,
            preferred_output_device,
            llm_settings,
            #[cfg(target_os = "macos")]
            front_app_before_paste,
            audio,
            trans,
            out,
        })
    }

    /// Reload the Whisper model (prefer idle state)
    pub fn reload_model(&self, model_path: &Path) -> Result<()> {
        self.log("[Whisper] Loading new model...");
        ensure_model(model_path).context("download Whisper model")?;
        let model_path_str = model_path
            .to_str()
            .ok_or_else(|| anyhow!("invalid model path (non-UTF-8)"))?;
        let new_ctx =
            WhisperContext::new_with_params(model_path_str, WhisperContextParameters::default())
                .with_context(|| format!("load Whisper model: {}", model_path.display()))?;
        let mut guard = self.ctx.lock().unwrap();
        *guard = Arc::new(new_ctx);
        *self.current_model_path.lock().unwrap() = model_path.to_path_buf();
        self.log(&format!(
            "[Whisper] Model switched: {}",
            model_path.display()
        ));
        Ok(())
    }

    pub fn get_model_path(&self) -> std::path::PathBuf {
        self.current_model_path.lock().unwrap().clone()
    }

    pub fn get_state(&self) -> SimpleRecState {
        *self.state.lock().unwrap()
    }

    pub fn toggle_recording(&self) -> SimpleRecState {
        // Use try_lock for non-blocking access (do not block UI thread)
        let mut state = match self.state.try_lock() {
            Ok(s) => s,
            Err(_) => {
                self.log("[Warning] Failed to acquire state lock (busy?)");
                return SimpleRecState::Busy;
            }
        };

        let _old_state = *state;
        let new_state = match *state {
            SimpleRecState::Idle => {
                self.log("[Record] Start recording");
                // Log current model
                let mp = self.current_model_path.lock().unwrap().clone();
                self.log(&format!("[Whisper] Using model: {}", mp.display()));
                // Stop processing loop sound if playing (avoid conflicts on resume)
                crate::utils::sound::stop_loop("processing");
                // Start recording on a separate thread
                // Remember current front app for auto‑paste on macOS
                self.out.remember_front_app();
                let core = self.clone();
                thread::spawn(move || core.start_recording_internal());
                SimpleRecState::Recording
            }
            SimpleRecState::Recording => {
                self.log("[Record] Stop recording and start transcription");
                // Play processing loop sound (1s gap; stop on completion)
                crate::utils::sound::start_loop("processing", "sounds/processing.mp3", 1000);
                // Stop recording on a separate thread too
                let core = self.clone();
                thread::spawn(move || core.stop_recording_internal());
                SimpleRecState::Processing
            }
            SimpleRecState::Processing => {
                self.log("[Warning] Already processing");
                SimpleRecState::Processing
            }
            SimpleRecState::PostProcessing => {
                self.log("[Warning] Already processing");
                SimpleRecState::PostProcessing
            }
            SimpleRecState::Busy => {
                self.log("[Warning] State busy");
                SimpleRecState::Busy
            }
        };
        *state = new_state;
        new_state
    }

    pub fn set_log_callback(&self, callback: LogCallback) {
        *self.log_callback.lock().unwrap() = Some(callback);
    }

    // Language (None for auto-detect)
    pub fn set_language(&self, lang: Option<&str>) {
        self.trans.set_language(lang);
    }

    // I/O device settings (output used for sound effects)
    pub fn set_audio_devices(&self, input: Option<&str>, output: Option<&str>) {
        self.audio.set_audio_devices(input);
        let mut out_guard = self.preferred_output_device.lock().unwrap();
        let new_out = output.map(|s| s.to_string());
        if *out_guard != new_out {
            *out_guard = new_out;
            sound::set_output_device(output);
        }
    }

    pub fn set_input_device_index(&self, idx: Option<usize>) {
        self.audio.set_input_device_index(idx);
    }

    pub fn set_input_device_host_and_index(&self, host: Option<&str>, idx: Option<usize>) {
        self.audio.set_input_device_host_and_index(host, idx);
    }

    pub fn set_input_gain(&self, gain: f32) {
        self.audio.set_input_gain(gain);
    }

    pub fn set_whisper_optimization(&self, params: WhisperOptimizationParams) {
        self.trans.set_whisper_optimization(params);
    }

    // User dictionary settings (pass YAML-loaded entries)
    pub fn set_dictionary_entries(&self, entries: Vec<DictionaryEntry>) {
        self.trans.set_dictionary_entries(entries);
        self.log("[Dictionary] Updated user dictionary");
    }

    // Chunk splitting strategy (VAD)
    pub fn set_chunk_split_strategy(&self, strategy: VadStrategy) {
        self.trans.set_chunk_split_strategy(strategy);
    }

    // Auto-stop by silence/max duration (0 disables each)
    pub fn set_auto_stop_params(&self, silence_secs: f32, max_secs: f32) {
        self.trans.set_auto_stop_params(silence_secs, max_secs);
    }

    pub fn set_llm_postprocess_settings(&self, settings: LlmPostProcessSettings) {
        *self.llm_settings.lock().unwrap() = settings.clone();
        self.trans.set_llm_settings(settings);
    }

    // Behavior options reflected from GUI settings
    pub fn set_behavior_options(&self, use_clipboard: bool, auto_paste: bool) {
        self.out.set_behavior_options(use_clipboard, auto_paste);
    }

    pub fn log(&self, message: &str) {
        if let Some(ref callback) = *self.log_callback.lock().unwrap() {
            callback(message);
        }
        if let Some(rest) = message.strip_prefix("[Error]") {
            tracing::error!("{}", rest.trim());
        } else if let Some(rest) = message.strip_prefix("[Warning]") {
            tracing::warn!("{}", rest.trim());
        } else if let Some(rest) = message.strip_prefix("[Info]") {
            tracing::info!("{}", rest.trim());
        } else {
            tracing::info!("{}", message);
        }
    }

    // Output behavior lives in `core/output.rs`

    fn start_recording_internal(&self) {
        // Delegate to components
        self.audio.start_capture(self.log_callback.clone());
        // Start chunk processing loop
        let on_auto_stop: Arc<dyn Fn() + Send + Sync> = {
            let core = self.clone();
            Arc::new(move || {
                let core2 = core.clone();
                std::thread::spawn(move || core2.stop_recording_internal());
            })
        };
        self.trans.start_processing(
            self.audio_buffer.clone(),
            self.stop_flag.clone(),
            self.log_callback.clone(),
            on_auto_stop,
        );
    }

    fn stop_recording_internal(&self) {
        // Stop audio capture and join thread
        self.audio.stop_capture();
        if let Some(handle) = self.processing_thread.lock().unwrap().take() {
            let _ = handle.join();
        }

        // Debug: dump recorded audio to WAV (16k/mono) if requested
        if std::env::var("HOOTVOICE_DEBUG_DUMP_AUDIO").ok().as_deref() == Some("1") {
            if let Ok(buf) = self.audio_buffer.lock() {
                let path = std::path::Path::new("debug_last.wav");
                if let Ok(mut writer) = WavWriter::create(
                    path,
                    WavSpec {
                        channels: 1,
                        sample_rate: 16_000,
                        bits_per_sample: 32,
                        sample_format: WavSampleFormat::Float,
                    },
                ) {
                    for &s in buf.iter() {
                        let _ = writer.write_sample::<f32>(s);
                    }
                    let _ = writer.finalize();
                    self.log(&format!(
                        "[Debug] Saved recorded audio: {} ({} samples)",
                        path.display(),
                        buf.len()
                    ));
                }
            }
        }
        // Finalize transcription and apply output behavior
        self.trans
            .finalize_and_output(self.audio_buffer.clone(), &self.log_callback, &self.out);
        *self.state.lock().unwrap() = SimpleRecState::Idle;
    }

    // removed: old helper `log_with_callback` (unused)
}
