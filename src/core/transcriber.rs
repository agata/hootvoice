use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use whisper_rs::WhisperContext;

use super::{
    postprocess::{PostProcessEngine, PostProcessResult},
    SimpleRecState,
};
use crate::app::chunk_processor::ChunkProcessor;
use crate::audio::VadStrategy;
use crate::core::LogCallback;
use crate::dictionary::{apply_pairs, flatten_sorted_with_context, DictionaryEntry};
use crate::llm::LlmPostProcessSettings;
use crate::transcription::WhisperOptimizationParams;

#[derive(Clone)]
pub struct Transcriber {
    pub ctx: Arc<Mutex<Arc<WhisperContext>>>,

    pub processor: Arc<Mutex<Option<Arc<Mutex<ChunkProcessor>>>>>,
    pub processing_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    pub last_processed_len: Arc<Mutex<usize>>,
    pub record_started_at: Arc<Mutex<Option<Instant>>>,

    pub language: Arc<Mutex<Option<String>>>,
    pub whisper_optimization: Arc<Mutex<WhisperOptimizationParams>>,
    pub chunk_strategy: Arc<Mutex<VadStrategy>>,
    pub dictionary_entries: Arc<Mutex<Vec<DictionaryEntry>>>,

    pub auto_stop_silence_secs: Arc<Mutex<f32>>, // 0 disables
    pub max_record_secs: Arc<Mutex<f32>>,        // 0 disables
    pub postprocess: PostProcessEngine,
    pub state: Arc<Mutex<SimpleRecState>>,
}

impl Transcriber {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ctx: Arc<Mutex<Arc<WhisperContext>>>,
        processor: Arc<Mutex<Option<Arc<Mutex<ChunkProcessor>>>>>,
        processing_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
        last_processed_len: Arc<Mutex<usize>>,
        record_started_at: Arc<Mutex<Option<Instant>>>,
        language: Arc<Mutex<Option<String>>>,
        whisper_optimization: Arc<Mutex<WhisperOptimizationParams>>,
        chunk_strategy: Arc<Mutex<VadStrategy>>,
        dictionary_entries: Arc<Mutex<Vec<DictionaryEntry>>>,
        auto_stop_silence_secs: Arc<Mutex<f32>>,
        max_record_secs: Arc<Mutex<f32>>,
        postprocess: PostProcessEngine,
        state: Arc<Mutex<SimpleRecState>>,
    ) -> Self {
        Self {
            ctx,
            processor,
            processing_thread,
            last_processed_len,
            record_started_at,
            language,
            whisper_optimization,
            chunk_strategy,
            dictionary_entries,
            auto_stop_silence_secs,
            max_record_secs,
            postprocess,
            state,
        }
    }

    pub fn set_language(&self, lang: Option<&str>) {
        *self.language.lock().unwrap() = lang.map(|s| s.to_string());
    }

    pub fn set_whisper_optimization(&self, params: WhisperOptimizationParams) {
        *self.whisper_optimization.lock().unwrap() = params;
    }

    pub fn set_chunk_split_strategy(&self, strategy: VadStrategy) {
        *self.chunk_strategy.lock().unwrap() = strategy;
    }

    pub fn set_dictionary_entries(&self, entries: Vec<DictionaryEntry>) {
        *self.dictionary_entries.lock().unwrap() = entries;
    }

    pub fn set_auto_stop_params(&self, silence_secs: f32, max_secs: f32) {
        *self.auto_stop_silence_secs.lock().unwrap() = silence_secs.max(0.0);
        *self.max_record_secs.lock().unwrap() = max_secs.max(0.0);
    }

    pub fn start_processing(
        &self,
        audio_buffer: Arc<Mutex<Vec<f32>>>,
        stop_flag: Arc<Mutex<bool>>,
        log_callback: Arc<Mutex<Option<LogCallback>>>,
        on_auto_stop: Arc<dyn Fn() + Send + Sync + 'static>,
    ) {
        let ctx = self.ctx.lock().unwrap().clone();
        let lang_opt = self.language.lock().unwrap().clone();
        let opt_params = self.whisper_optimization.lock().unwrap().clone();
        let vad = *self.chunk_strategy.lock().unwrap();
        let auto_stop_silence_secs = *self.auto_stop_silence_secs.lock().unwrap();
        let max_record_secs = *self.max_record_secs.lock().unwrap();

        *self.last_processed_len.lock().unwrap() = 0;
        *self.record_started_at.lock().unwrap() = Some(Instant::now());

        let proc = Arc::new(Mutex::new(ChunkProcessor::new(
            ctx,
            16_000,
            lang_opt.clone(),
            Some(opt_params.clone()),
            vad,
        )));

        // Forward logs to GUI
        {
            let log_cb = log_callback.clone();
            let gui_logger: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |msg: &str| {
                if let Some(ref cb) = *log_cb.lock().unwrap() {
                    cb(msg);
                }
            });
            if let Ok(mut p) = proc.lock() {
                p.set_logger(gui_logger);
            }
        }
        proc.lock().unwrap().start_worker();
        *self.processor.lock().unwrap() = Some(proc.clone());

        let buffer_for_proc = audio_buffer.clone();
        let stop_for_proc = stop_flag.clone();
        let last_len_for_proc = self.last_processed_len.clone();
        let processor_holder = self.processor.clone();
        let log_cb_for_proc = log_callback.clone();
        let record_started_for_proc = self.record_started_at.clone();
        let chunk_strategy_for_proc = self.chunk_strategy.clone();
        let on_auto_stop_cb = on_auto_stop.clone();

        let proc_thread = thread::spawn(move || {
            use std::time::Duration;
            let mut last_tick = std::time::Instant::now();
            let process_interval = Duration::from_millis(100);
            let mut auto_stop_triggered = false;
            let mut global_silence_started: Option<std::time::Instant> = None;
            let silence_threshold = match *chunk_strategy_for_proc.lock().unwrap() {
                VadStrategy::Aggressive => 0.007,
                VadStrategy::Normal => 0.005,
            };
            loop {
                if *stop_for_proc.lock().unwrap() {
                    break;
                }
                if last_tick.elapsed() >= process_interval {
                    let maybe_slice = {
                        let buf = buffer_for_proc.lock().unwrap();
                        let mut last_len = last_len_for_proc.lock().unwrap();
                        let new_len = buf.len();
                        if new_len > *last_len {
                            let slice = buf[*last_len..new_len].to_vec();
                            *last_len = new_len;
                            Some(slice)
                        } else {
                            None
                        }
                    };
                    if let Some(pcm) = maybe_slice {
                        let proc_opt = processor_holder.lock().unwrap().clone();
                        if let Some(proc_arc) = proc_opt {
                            if let Ok(mut p) = proc_arc.lock() {
                                p.process_audio(&pcm, 16_000);
                                if !auto_stop_triggered && auto_stop_silence_secs > 0.0 {
                                    let rms = {
                                        let sum_sq: f32 = pcm.iter().map(|s| s * s).sum();
                                        ((sum_sq / (pcm.len().max(1) as f32)).max(0.0)).sqrt()
                                    };
                                    let now = std::time::Instant::now();
                                    if rms < silence_threshold {
                                        if global_silence_started.is_none() {
                                            global_silence_started = Some(now);
                                        }
                                    } else {
                                        global_silence_started = None;
                                    }
                                    if let Some(st) = global_silence_started {
                                        let sil = st.elapsed().as_secs_f32();
                                        if sil >= auto_stop_silence_secs {
                                            auto_stop_triggered = true;
                                            if let Some(ref cb) = *log_cb_for_proc.lock().unwrap() {
                                                cb(&format!(
                                                    "[Record] ⏹ Auto stop: silence for {:.1}s",
                                                    sil
                                                ));
                                            }
                                            *stop_for_proc.lock().unwrap() = true;
                                            (on_auto_stop_cb)();
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Check max recording time
                    if !auto_stop_triggered && max_record_secs > 0.0 {
                        let elapsed = if let Some(start) = *record_started_for_proc.lock().unwrap()
                        {
                            start.elapsed().as_secs_f32()
                        } else {
                            0.0
                        };
                        if elapsed >= max_record_secs {
                            auto_stop_triggered = true;
                            if let Some(ref cb) = *log_cb_for_proc.lock().unwrap() {
                                cb(&format!(
                                    "[Record] ⏹ Auto stop: max duration {:.0}s reached",
                                    max_record_secs
                                ));
                            }
                            *stop_for_proc.lock().unwrap() = true;
                            (on_auto_stop_cb)();
                        }
                    }
                    last_tick = std::time::Instant::now();
                }
                thread::sleep(std::time::Duration::from_millis(50));
            }
        });

        *self.processing_thread.lock().unwrap() = Some(proc_thread);
    }

    pub fn finalize_and_output(
        &self,
        audio_buffer: Arc<Mutex<Vec<f32>>>,
        log: &Arc<Mutex<Option<LogCallback>>>,
        output: &crate::core::output::OutputBehavior,
    ) {
        // Push remaining samples
        let final_slice = {
            let buf = audio_buffer.lock().unwrap();
            let mut last_len = self.last_processed_len.lock().unwrap();
            if buf.len() > *last_len {
                let slice = buf[*last_len..].to_vec();
                *last_len = buf.len();
                Some(slice)
            } else {
                None
            }
        };
        if let Some(pcm) = final_slice {
            if let Some(proc_arc) = self.processor.lock().unwrap().clone() {
                if let Ok(mut p) = proc_arc.lock() {
                    p.process_audio(&pcm, 16_000);
                }
            }
        }

        // Finish
        let whisper_start_time = Instant::now();
        let chunk_results = if let Some(proc_arc) = self.processor.lock().unwrap().take() {
            if let Ok(mut p) = proc_arc.lock() {
                p.finish(16_000)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let whisper_processing_time = whisper_start_time.elapsed().as_secs_f32();

        if chunk_results.is_empty() {
            Self::log_with_callback(log, "[Whisper] No speech detected");
            crate::utils::sound::stop_loop("processing");
            crate::utils::sound::play_sound_async("sounds/fail.mp3");
            return;
        }

        for r in &chunk_results {
            Self::log_with_callback(
                log,
                &format!("[{:>5.1}–{:>5.1}] {}", r.start_time, r.end_time, r.text),
            );
        }
        let full_text = ChunkProcessor::combine_results(&chunk_results);
        Self::log_with_callback(log, &format!("[Whisper] Combined result: {}", full_text));

        // Dictionary
        let dictionary_snapshot = self.dictionary_entries.lock().unwrap().clone();
        let pairs = flatten_sorted_with_context(&dictionary_snapshot, &full_text);
        let corrected_text = if pairs.is_empty() {
            full_text.clone()
        } else {
            apply_pairs(&full_text, &pairs)
        };
        if corrected_text != full_text {
            Self::log_with_callback(
                log,
                &format!("[Dictionary] Applied replacements: {}", corrected_text),
            );
        } else {
            Self::log_with_callback(log, "[Dictionary] No change (no matches)");
        }
        let dictionary_prompt = Self::dictionary_prompt_text(&dictionary_snapshot);

        let language_setting = self.language.lock().unwrap().clone();
        let language_hint = language_setting.as_deref();
        let PostProcessResult {
            final_text,
            llm_latency_secs,
        } = self.postprocess.process(
            &corrected_text,
            dictionary_prompt.as_deref(),
            language_hint,
            log,
        );

        output.apply_output(&final_text);
        crate::utils::sound::stop_loop("processing");

        // Performance info
        let recording_duration = {
            if let Some(start) = self.record_started_at.lock().unwrap().take() {
                start.elapsed().as_secs_f32()
            } else {
                audio_buffer.lock().unwrap().len() as f32 / 16000.0
            }
        };
        Self::log_with_callback(log, "\n📊 Performance metrics:");
        Self::log_with_callback(
            log,
            &format!("  🎙️  Recording time: {:.2}s", recording_duration),
        );
        Self::log_with_callback(
            log,
            &format!("  🔄 Whisper processing: {:.2}s", whisper_processing_time),
        );
        if llm_latency_secs > 0.0 {
            Self::log_with_callback(
                log,
                &format!("  🤖 LLM processing: {:.2}s", llm_latency_secs),
            );
        }
        let total = whisper_processing_time + llm_latency_secs;
        Self::log_with_callback(log, &format!("  ⏱️  Total processing time: {:.2}s", total));
        if recording_duration > 0.0 {
            Self::log_with_callback(
                log,
                &format!(
                    "  ⚡ RTF (Real Time Factor): {:.2}x\n",
                    total / recording_duration
                ),
            );
        } else {
            Self::log_with_callback(log, "  ⚡ RTF (Real Time Factor): N/A\n");
        }

        if let Ok(mut state) = self.state.lock() {
            *state = SimpleRecState::Idle;
        }
    }

    fn dictionary_prompt_text(entries: &[DictionaryEntry]) -> Option<String> {
        const MAX_LINES: usize = 40;
        let mut lines = Vec::new();
        for entry in entries {
            if entry.aliases.is_empty() {
                continue;
            }
            let mut line = format!("- {}: {}", entry.canonical, entry.aliases.join(", "));
            if !entry.include.is_empty() {
                line.push_str(" (context: ");
                line.push_str(&entry.include.join(", "));
                line.push(')');
            }
            lines.push(line);
            if lines.len() >= MAX_LINES {
                break;
            }
        }
        if lines.is_empty() {
            None
        } else {
            let mut prompt = String::from("User dictionary replacements:\n");
            prompt.push_str(&lines.join("\n"));
            Some(prompt)
        }
    }

    pub fn set_llm_settings(&self, settings: LlmPostProcessSettings) {
        self.postprocess.set_settings(settings);
    }

    fn log_with_callback(log_callback: &Arc<Mutex<Option<LogCallback>>>, message: &str) {
        if let Some(ref callback) = *log_callback.lock().unwrap() {
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
}
