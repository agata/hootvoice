use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::core::LogCallback;

thread_local! {
    static MONO_BUFFER: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    static RESAMPLE_BUFFER: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

// Debug/safety: track active input streams in AudioIO (detect double starts)
static ACTIVE_CORE_INPUT_STREAMS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Minimal device settings and capture lifecycle grouped here
#[derive(Clone)]
pub struct AudioIO {
    pub audio_buffer: Arc<Mutex<Vec<f32>>>,
    pub stop_flag: Arc<Mutex<bool>>,
    pub recording_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,

    pub preferred_input_device: Arc<Mutex<Option<String>>>,
    pub preferred_input_device_index: Arc<Mutex<Option<usize>>>,
    pub preferred_input_host: Arc<Mutex<Option<String>>>,
    pub input_gain: Arc<AtomicU32>, // f32 bits

    pub current_session: Arc<AtomicU64>,
}

impl AudioIO {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        audio_buffer: Arc<Mutex<Vec<f32>>>,
        stop_flag: Arc<Mutex<bool>>,
        recording_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
        preferred_input_device: Arc<Mutex<Option<String>>>,
        preferred_input_device_index: Arc<Mutex<Option<usize>>>,
        preferred_input_host: Arc<Mutex<Option<String>>>,
        input_gain: Arc<AtomicU32>,
        current_session: Arc<AtomicU64>,
    ) -> Self {
        Self {
            audio_buffer,
            stop_flag,
            recording_thread,
            preferred_input_device,
            preferred_input_device_index,
            preferred_input_host,
            input_gain,
            current_session,
        }
    }

    pub fn set_audio_devices(&self, input: Option<&str>) {
        *self.preferred_input_device.lock().unwrap() = input.map(|s| s.to_string());
    }

    pub fn set_input_device_index(&self, idx: Option<usize>) {
        *self.preferred_input_device_index.lock().unwrap() = idx;
    }

    pub fn set_input_device_host_and_index(&self, host: Option<&str>, idx: Option<usize>) {
        *self.preferred_input_host.lock().unwrap() = host.map(|s| s.to_string());
        *self.preferred_input_device_index.lock().unwrap() = idx;
    }

    pub fn set_input_gain(&self, gain: f32) {
        self.input_gain
            .store(gain.max(0.0).to_bits(), Ordering::Relaxed);
    }

    // Start CPAL input stream and push 16k mono f32 into audio_buffer
    pub fn start_capture(&self, log_callback: Arc<Mutex<Option<LogCallback>>>) {
        use cpal::{
            traits::{DeviceTrait, HostTrait, StreamTrait},
            StreamConfig,
        };

        // Clear audio buffer and flags
        self.audio_buffer.lock().unwrap().clear();
        *self.stop_flag.lock().unwrap() = false;

        let buffer = self.audio_buffer.clone();
        let stop_flag = self.stop_flag.clone();
        let preferred_in = self.preferred_input_device.lock().unwrap().clone();
        let preferred_in_idx = *self.preferred_input_device_index.lock().unwrap();
        let preferred_host = self.preferred_input_host.lock().unwrap().clone();
        let input_gain_for_thread = self.input_gain.clone();
        // Bump session ID
        let session_id = self.current_session.fetch_add(1, Ordering::SeqCst) + 1;
        let session_guard = self.current_session.clone();

        let handle = thread::spawn(move || {
            // Get default host and device
            // Try preferred host first (if any)
            let host = if let Some(ref h) = preferred_host {
                // Match preferred host by name (lowercased)
                let mut chosen = None;
                for id in cpal::available_hosts() {
                    let name = format!("{:?}", id).to_lowercase();
                    if name == h.to_lowercase() {
                        chosen = Some(id);
                        break;
                    }
                }
                if let Some(id) = chosen {
                    cpal::host_from_id(id).unwrap_or_else(|_| cpal::default_host())
                } else {
                    cpal::default_host()
                }
            } else {
                cpal::default_host()
            };

            // Enumerate current devices (debug)
            let device_list: Vec<cpal::Device> = host
                .input_devices()
                .map(|it| it.collect())
                .unwrap_or_default();
            let mut names: Vec<String> = Vec::new();
            for d in &device_list {
                names.push(d.name().unwrap_or_else(|_| "(unknown)".to_string()));
            }
            Self::log_with_callback(
                &log_callback,
                &format!(
                    "[Record] Desired input: host={:?}, name={:?}, index={:?}",
                    preferred_host, preferred_in, preferred_in_idx
                ),
            );
            if names.is_empty() {
                Self::log_with_callback(&log_callback, "[Record] Input devices: none");
            } else {
                Self::log_with_callback(&log_callback, "[Record] Input devices:");
                for (i, n) in names.iter().enumerate() {
                    Self::log_with_callback(&log_callback, &format!("  [{}] {}", i, n));
                }
            }

            // Selection order: host index, then name match, then default
            let mut chosen: Option<cpal::Device> = None;
            if let Some(idx) = preferred_in_idx {
                if let Some(d) = device_list.get(idx).cloned() {
                    chosen = Some(d);
                    Self::log_with_callback(
                        &log_callback,
                        &format!("[Record] Selected by index: host index={}", idx),
                    );
                } else {
                    Self::log_with_callback(
                        &log_callback,
                        &format!(
                            "[Record] Index out of range; ignored: host index={} / devices={}",
                            idx,
                            device_list.len()
                        ),
                    );
                }
            }
            if chosen.is_none() {
                if let Some(ref want) = preferred_in {
                    if let Some(pos) = names.iter().position(|n| n == want) {
                        chosen = device_list.get(pos).cloned();
                        Self::log_with_callback(
                            &log_callback,
                            &format!("[Record] Selected by name: '{}'", want),
                        );
                    } else {
                        Self::log_with_callback(
                            &log_callback,
                            &format!("[Record] Name not found: '{}'", want),
                        );
                    }
                }
            }
            if chosen.is_none() {
                chosen = host.default_input_device();
                Self::log_with_callback(&log_callback, "[Record] Fallback to default device");
            }
            let device = match chosen {
                Some(d) => d,
                None => {
                    Self::log_with_callback(&log_callback, "[Error] No input device found");
                    return;
                }
            };
            let dev_name = device.name().unwrap_or_else(|_| "(unknown)".to_string());
            Self::log_with_callback(
                &log_callback,
                &format!("[Record] Selected device: {}", dev_name),
            );

            // Get supported default config
            let supported_config = match device.default_input_config() {
                Ok(config) => config,
                Err(e) => {
                    Self::log_with_callback(
                        &log_callback,
                        &format!("[Error] Failed to get device default config: {}", e),
                    );
                    return;
                }
            };

            let config: StreamConfig = supported_config.clone().into();
            let sample_format = supported_config.sample_format();
            let err_fn = |err| eprintln!("[Error] Stream error: {}", err);

            let sr_for_cb = config.sample_rate.0;
            let ch_for_cb = config.channels as usize;
            let in_frames_counter = Arc::new(AtomicU64::new(0));
            let out_frames_counter = Arc::new(AtomicU64::new(0));
            let debug = std::env::var("HOOTVOICE_DEBUG_AUDIO").ok().as_deref() == Some("1");

            // Safety: wait briefly if a previous input stream remains (max 2s)
            let wait_start = std::time::Instant::now();
            while ACTIVE_CORE_INPUT_STREAMS.load(Ordering::SeqCst) > 0 {
                Self::log_with_callback(
                    &log_callback,
                    "[Record] Waiting for previous input stream to finish...",
                );
                if wait_start.elapsed() > std::time::Duration::from_secs(2) {
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(50));
            }

            let stream_res = match sample_format {
                cpal::SampleFormat::I16 => {
                    let buffer_clone = buffer.clone();
                    let input_gain_for_cb = input_gain_for_thread.clone();
                    let in_frames_counter_cb = in_frames_counter.clone();
                    let out_frames_counter_cb = out_frames_counter.clone();
                    let session_guard_cb = session_guard.clone();
                    device.build_input_stream(
                        &config,
                        move |data: &[i16], _: &_| {
                            if session_guard_cb.load(Ordering::Relaxed) != session_id {
                                return;
                            }
                            MONO_BUFFER.with(|mono_buf| {
                                let mut mono = mono_buf.borrow_mut();
                                mono.clear();
                                if ch_for_cb > 1 {
                                    mono.reserve(data.len() / ch_for_cb);
                                    for chunk in data.chunks(ch_for_cb) {
                                        let mut sum = 0f32;
                                        for &s in chunk {
                                            sum += s as f32 / 32768.0;
                                        }
                                        mono.push(sum / ch_for_cb as f32);
                                    }
                                } else {
                                    mono.reserve(data.len());
                                    for &s in data {
                                        mono.push(s as f32 / 32768.0);
                                    }
                                }
                                if debug {
                                    in_frames_counter_cb
                                        .fetch_add(mono.len() as u64, Ordering::Relaxed);
                                }
                                RESAMPLE_BUFFER.with(|res_buf| {
                                    let mut resampled = res_buf.borrow_mut();
                                    if sr_for_cb != 16_000 {
                                        Self::resample_into(
                                            &mono,
                                            sr_for_cb,
                                            16_000,
                                            &mut resampled,
                                        );
                                    } else {
                                        resampled.clear();
                                        resampled.extend_from_slice(&mono);
                                    }
                                    if debug {
                                        out_frames_counter_cb
                                            .fetch_add(resampled.len() as u64, Ordering::Relaxed);
                                    }
                                    let g =
                                        f32::from_bits(input_gain_for_cb.load(Ordering::Relaxed));
                                    if (g - 1.0).abs() > f32::EPSILON {
                                        for s in &mut *resampled {
                                            *s *= g;
                                        }
                                    }
                                    if let Ok(mut buf) = buffer_clone.lock() {
                                        buf.extend_from_slice(&resampled);
                                    }
                                });
                            });
                        },
                        err_fn,
                        None,
                    )
                }
                cpal::SampleFormat::U16 => {
                    let buffer_clone = buffer.clone();
                    let input_gain_for_cb = input_gain_for_thread.clone();
                    let in_frames_counter_cb = in_frames_counter.clone();
                    let out_frames_counter_cb = out_frames_counter.clone();
                    let session_guard_cb = session_guard.clone();
                    device.build_input_stream(
                        &config,
                        move |data: &[u16], _: &_| {
                            if session_guard_cb.load(Ordering::Relaxed) != session_id {
                                return;
                            }
                            MONO_BUFFER.with(|mono_buf| {
                                let mut mono = mono_buf.borrow_mut();
                                mono.clear();
                                if ch_for_cb > 1 {
                                    mono.reserve(data.len() / ch_for_cb);
                                    for chunk in data.chunks(ch_for_cb) {
                                        let mut sum = 0f32;
                                        for &s in chunk {
                                            sum += (s as f32 - 32768.0) / 32768.0;
                                        }
                                        mono.push(sum / ch_for_cb as f32);
                                    }
                                } else {
                                    mono.reserve(data.len());
                                    for &s in data {
                                        mono.push((s as f32 - 32768.0) / 32768.0);
                                    }
                                }
                                if debug {
                                    in_frames_counter_cb
                                        .fetch_add(mono.len() as u64, Ordering::Relaxed);
                                }
                                RESAMPLE_BUFFER.with(|res_buf| {
                                    let mut resampled = res_buf.borrow_mut();
                                    if sr_for_cb != 16_000 {
                                        Self::resample_into(
                                            &mono,
                                            sr_for_cb,
                                            16_000,
                                            &mut resampled,
                                        );
                                    } else {
                                        resampled.clear();
                                        resampled.extend_from_slice(&mono);
                                    }
                                    if debug {
                                        out_frames_counter_cb
                                            .fetch_add(resampled.len() as u64, Ordering::Relaxed);
                                    }
                                    let g =
                                        f32::from_bits(input_gain_for_cb.load(Ordering::Relaxed));
                                    if (g - 1.0).abs() > f32::EPSILON {
                                        for s in &mut *resampled {
                                            *s *= g;
                                        }
                                    }
                                    if let Ok(mut buf) = buffer_clone.lock() {
                                        buf.extend_from_slice(&resampled);
                                    }
                                });
                            });
                        },
                        err_fn,
                        None,
                    )
                }
                _ => {
                    let buffer_clone = buffer.clone();
                    let input_gain_for_cb = input_gain_for_thread.clone();
                    let in_frames_counter_cb = in_frames_counter.clone();
                    let out_frames_counter_cb = out_frames_counter.clone();
                    let session_guard_cb = session_guard.clone();
                    device.build_input_stream(
                        &config,
                        move |data: &[f32], _: &_| {
                            if session_guard_cb.load(Ordering::Relaxed) != session_id {
                                return;
                            }
                            MONO_BUFFER.with(|mono_buf| {
                                let mut mono = mono_buf.borrow_mut();
                                mono.clear();
                                if ch_for_cb > 1 {
                                    mono.reserve(data.len() / ch_for_cb);
                                    for chunk in data.chunks(ch_for_cb) {
                                        mono.push(chunk.iter().sum::<f32>() / ch_for_cb as f32);
                                    }
                                } else {
                                    mono.extend_from_slice(data);
                                }
                                if debug {
                                    in_frames_counter_cb
                                        .fetch_add(mono.len() as u64, Ordering::Relaxed);
                                }
                                RESAMPLE_BUFFER.with(|res_buf| {
                                    let mut resampled = res_buf.borrow_mut();
                                    if sr_for_cb != 16_000 {
                                        Self::resample_into(
                                            &mono,
                                            sr_for_cb,
                                            16_000,
                                            &mut resampled,
                                        );
                                    } else {
                                        resampled.clear();
                                        resampled.extend_from_slice(&mono);
                                    }
                                    if debug {
                                        out_frames_counter_cb
                                            .fetch_add(resampled.len() as u64, Ordering::Relaxed);
                                    }
                                    let g =
                                        f32::from_bits(input_gain_for_cb.load(Ordering::Relaxed));
                                    if (g - 1.0).abs() > f32::EPSILON {
                                        for s in &mut *resampled {
                                            *s *= g;
                                        }
                                    }
                                    if let Ok(mut buf) = buffer_clone.lock() {
                                        buf.extend_from_slice(&resampled);
                                    }
                                });
                            });
                        },
                        err_fn,
                        None,
                    )
                }
            };

            match stream_res {
                Ok(stream) => {
                    let active = ACTIVE_CORE_INPUT_STREAMS.fetch_add(1, Ordering::SeqCst) + 1;
                    if debug {
                        Self::log_with_callback(
                            &log_callback,
                            &format!(
                                "[Record] Input stream started: sr={}Hz ch={} (core active={})",
                                sr_for_cb, ch_for_cb, active
                            ),
                        );
                    } else {
                        Self::log_with_callback(
                            &log_callback,
                            &format!(
                                "[Record] Input stream started: sr={}Hz ch={}",
                                sr_for_cb, ch_for_cb
                            ),
                        );
                    }
                    Self::log_with_callback(
                        &log_callback,
                        "[Record] Using default config (soft convert to 16k/mono)",
                    );
                    if let Err(e) = stream.play() {
                        Self::log_with_callback(
                            &log_callback,
                            &format!("[Error] Failed to start stream: {}", e),
                        );
                        return;
                    }
                    Self::log_with_callback(&log_callback, "[Record] Recording started");
                    crate::utils::sound::play_sound_async("sounds/start.mp3");

                    // Busy-wait loop to keep the thread alive until stop_flag becomes true
                    while !*stop_flag.lock().unwrap() {
                        thread::sleep(std::time::Duration::from_millis(50));
                        if debug {
                            let in_frames = in_frames_counter.load(Ordering::Relaxed);
                            let out_frames = out_frames_counter.load(Ordering::Relaxed);
                            if in_frames > 0 || out_frames > 0 {
                                // Lightweight periodic debug (every ~1s)
                            }
                        }
                    }
                    drop(stream);
                    let active = ACTIVE_CORE_INPUT_STREAMS.fetch_sub(1, Ordering::SeqCst) - 1;
                    Self::log_with_callback(
                        &log_callback,
                        &format!("[Record] Input stream stopped (core active={})", active),
                    );
                }
                Err(e) => {
                    Self::log_with_callback(
                        &log_callback,
                        &format!("[Error] Failed to build input stream: {}", e),
                    );
                }
            }
        });

        *self.recording_thread.lock().unwrap() = Some(handle);
    }

    pub fn stop_capture(&self) {
        *self.stop_flag.lock().unwrap() = true;
        let _ = self.current_session.fetch_add(1, Ordering::SeqCst);
        if let Some(h) = self.recording_thread.lock().unwrap().take() {
            let _ = h.join();
        }
    }

    // TODO: Consider replacing with a low-cost FIR resampler (e.g., rubato / speexdsp).
    // Evaluate trade-offs in binary size and extra dependencies.
    fn resample_into(input: &[f32], src_rate: u32, dst_rate: u32, output: &mut Vec<f32>) {
        output.clear();
        if src_rate == dst_rate {
            output.extend_from_slice(input);
            return;
        }
        let ratio = src_rate as f32 / dst_rate as f32;
        let output_len = (input.len() as f32 / ratio) as usize;
        output.reserve(output_len);
        for i in 0..output_len {
            let src_idx = i as f32 * ratio;
            let idx = src_idx as usize;
            let frac = src_idx - idx as f32;
            if idx + 1 < input.len() {
                let sample = input[idx] * (1.0 - frac) + input[idx + 1] * frac;
                output.push(sample);
            } else if idx < input.len() {
                output.push(input[idx]);
            }
        }
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
