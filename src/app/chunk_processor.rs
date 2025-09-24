use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;
use unicode_categories::UnicodeCategories;
use whisper_rs::WhisperContext;

use crate::audio::{SplitDecision, VadStrategy, VoiceActivityDetector};
use crate::core::LogCallback;
use crate::transcription::{transcribe_with_state, WhisperOptimizationParams};

/// Audio chunk
#[derive(Clone)]
pub struct AudioChunk {
    pub id: usize,
    pub samples: Vec<f32>,
    pub start_time: f32,
    pub duration: f32,
}

/// Chunk processing result
#[derive(Debug, Clone)]
pub struct ChunkResult {
    pub id: usize,
    pub text: String,
    pub start_time: f32,
    pub end_time: f32,
    pub processing_time: f32,
}

/// Chunk-based audio processing (enhanced)
pub struct ChunkProcessor {
    ctx: Arc<WhisperContext>,
    vad: VoiceActivityDetector,
    chunks: Vec<AudioChunk>,
    results: Arc<Mutex<Vec<ChunkResult>>>,
    tx: Option<mpsc::Sender<AudioChunk>>,
    rx: Option<mpsc::Receiver<AudioChunk>>,
    worker_handle: Option<thread::JoinHandle<()>>,
    next_chunk_id: usize,
    current_buffer: Vec<f32>,
    chunk_start_time: f32,
    total_samples_processed: usize,
    sample_rate: u32,
    // Optional logger for forwarding logs to GUI
    logger: Option<LogCallback>,
    language: Option<String>,
    optimization_params: Option<WhisperOptimizationParams>,
}

impl ChunkProcessor {
    pub fn new(
        ctx: Arc<WhisperContext>,
        sample_rate: u32,
        language: Option<String>,
        optimization_params: Option<WhisperOptimizationParams>,
        vad_strategy: VadStrategy,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let results = Arc::new(Mutex::new(Vec::new()));

        Self {
            ctx,
            vad: VoiceActivityDetector::new_with_strategy(sample_rate, vad_strategy),
            chunks: Vec::new(),
            results,
            tx: Some(tx),
            rx: Some(rx),
            worker_handle: None,
            next_chunk_id: 0,
            current_buffer: Vec::new(),
            chunk_start_time: 0.0,
            total_samples_processed: 0,
            sample_rate,
            logger: None,
            language,
            optimization_params,
        }
    }

    /// Set logger (hook to stream logs to GUI)
    pub fn set_logger(&mut self, logger: LogCallback) {
        self.logger = Some(logger);
    }

    /// Log helper (GUI logger if available, else stdout)
    fn log_line(&self, line: &str) {
        if let Some(ref lg) = self.logger {
            lg(line);
        } else {
            println!("{}", line);
            // Force flush (console only)
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    }

    /// Start worker thread
    pub fn start_worker(&mut self) {
        let ctx = self.ctx.clone();
        let results = self.results.clone();
        let rx = self.rx.take().expect("Receiver already taken");
        let logger = self.logger.clone();

        let lang = self.language.clone();
        let opt_params = self.optimization_params.clone();
        let handle = thread::spawn(move || {
            // Create WhisperState once and reuse it in this worker
            let mut wstate = match ctx.create_state() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[Whisper] Failed to create state: {}", e);
                    return;
                }
            };

            while let Ok(chunk) = rx.recv() {
                let start_time = Instant::now();

                // Run Whisper inference
                if let Ok(result) = transcribe_with_state(
                    &mut wstate,
                    &chunk.samples,
                    lang.as_deref(),
                    opt_params.as_ref(),
                ) {
                    // Filter non-speech noise
                    let text = filter_noise_text(&result.text);

                    if !text.is_empty() {
                        let chunk_result = ChunkResult {
                            id: chunk.id,
                            text,
                            start_time: chunk.start_time,
                            end_time: chunk.start_time + chunk.duration,
                            processing_time: start_time.elapsed().as_secs_f32(),
                        };

                        // Save result
                        if let Ok(mut results) = results.lock() {
                            results.push(chunk_result.clone());
                            results.sort_by_key(|r| r.id);

                            // Realtime log output
                            let line = format!(
                                "\n  ✅ [Chunk{}] {:.1}s-{:.1}s: {} (proc: {:.2}s)",
                                chunk_result.id,
                                chunk_result.start_time,
                                chunk_result.end_time,
                                chunk_result.text,
                                chunk_result.processing_time
                            );
                            if let Some(ref lg) = logger {
                                lg(&line);
                            } else {
                                println!("{}", line);
                                use std::io::Write;
                                let _ = std::io::stdout().flush();
                            }
                        }
                    } else {
                        let line = format!("\n  ⏭️  [Chunk{}] No speech / noise only", chunk.id);
                        if let Some(ref lg) = logger {
                            lg(&line);
                        } else {
                            println!("{}", line);
                        }
                    }
                }
            }
        });

        self.worker_handle = Some(handle);
        self.vad.start_recording();
    }

    /// Process audio data (expected ~every 100 ms)
    pub fn process_audio(&mut self, samples: &[f32], sample_rate: u32) {
        // Append to buffer
        self.current_buffer.extend_from_slice(samples);
        self.total_samples_processed += samples.len();

        // Silence detection by VAD
        let decision = self.vad.process_audio(samples);

        // Decide chunk split
        match decision {
            SplitDecision::Split { reason } => {
                self.create_and_send_chunk(sample_rate, &reason);
            }
            SplitDecision::Skip => {
                // Skip silent chunk and reset
                let log_line = "  ⏭️  Skip silent chunk".to_string();
                self.log_line(&log_line);
                self.reset_buffer();
            }
            SplitDecision::Continue => {
                // Continue
            }
        }
    }

    /// Create a chunk and send it
    fn create_and_send_chunk(&mut self, sample_rate: u32, reason: &str) {
        if self.current_buffer.is_empty() {
            return;
        }

        let chunk = AudioChunk {
            id: self.next_chunk_id,
            samples: self.current_buffer.clone(),
            start_time: self.chunk_start_time,
            duration: self.current_buffer.len() as f32 / sample_rate as f32,
        };

        // Realtime log output
        let log_line = format!(
            "\n  🎯 Chunk{} created: {:.1}s ({}) → Start Whisper",
            chunk.id, chunk.duration, reason
        );
        self.log_line(&log_line);

        // Send to worker thread
        if let Some(tx) = &self.tx {
            let _ = tx.send(chunk.clone());
        }

        self.chunks.push(chunk);
        self.next_chunk_id += 1;

        // Reset buffer
        self.reset_buffer();
    }

    /// Reset buffer
    fn reset_buffer(&mut self) {
        self.chunk_start_time = self.total_samples_processed as f32 / self.sample_rate as f32;
        self.current_buffer.clear();
    }

    /// Finalize when recording ends
    pub fn finish(&mut self, sample_rate: u32) -> Vec<ChunkResult> {
        // Send remaining buffer as a chunk (if it has audio)
        if !self.current_buffer.is_empty() {
            // Quick audio check
            let rms = calculate_rms(&self.current_buffer);
            if rms > 0.005 {
                self.create_and_send_chunk(sample_rate, "end-of-recording");
            } else {
                let log_line = "  ⏭️  Final chunk skipped (silence)".to_string();
                self.log_line(&log_line);
            }
        }

        // Stop worker thread
        drop(self.tx.take());

        // Wait for worker thread to finish
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }

        // Collect results
        self.results.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Concatenate all text
    pub fn combine_results(results: &[ChunkResult]) -> String {
        // Remove adjacent overlap while concatenating (respect UTF‑8 boundaries)
        fn merge_with_overlap(mut acc: String, next: &str) -> String {
            let next_trim = next.trim();
            if next_trim.is_empty() {
                return acc;
            }
            // If `next` is already fully contained in `acc`, do nothing
            if acc.contains(next_trim) {
                return acc;
            }
            // Limit overlap search to up to 64 bytes; respect UTF‑8 boundaries
            let max_k = acc.len().min(next_trim.len()).min(64);
            // Collect byte offsets of character boundaries from the start of `next` (include full length)
            let mut cuts: Vec<usize> = next_trim.char_indices().map(|(i, _)| i).collect();
            if *cuts.last().unwrap_or(&0) != next_trim.len() {
                cuts.push(next_trim.len());
            }
            let mut best_k = 0usize;
            for &i in cuts.iter().rev() {
                if i == 0 || i > max_k {
                    continue;
                }
                if acc.ends_with(&next_trim[..i]) {
                    best_k = i;
                    break;
                }
            }
            acc.push_str(&next_trim[best_k..]);
            acc
        }

        let mut acc = String::new();
        for r in results.iter() {
            let t = r.text.trim();
            if t.is_empty() {
                continue;
            }
            acc = merge_with_overlap(acc, t);
        }
        acc
    }

    // removed: unused helpers (progress, current silence seconds)
}

/// RMS calculation
fn calculate_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_squares: f32 = samples.iter().map(|s| s * s).sum();
    (sum_squares / samples.len() as f32).sqrt()
}

/// Filter noisy text
fn is_punct_or_space_only(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_whitespace() || c.is_ascii_punctuation() || c.is_punctuation())
}

fn filter_noise_text(text: &str) -> String {
    // Remove common noise patterns
    let noise_patterns = [
        "(music)",
        "(sound)",
        "(applause)",
        "(laugh)",
        "(cough)",
        "[music]",
        "[sound]",
        "[applause]",
        "[laugh]",
        "[cough]",
        "♪",
    ];

    let mut filtered = text.to_string();
    for pattern in &noise_patterns {
        filtered = filtered.replace(pattern, "");
    }

    // Trim surrounding whitespace
    filtered = filtered.trim().to_string();

    let trimmed = filtered.trim();
    // Discard punctuation/space-only text
    if is_punct_or_space_only(trimmed) {
        String::new()
    } else {
        trimmed.to_string()
    }
}
