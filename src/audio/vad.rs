use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Instant;

/// Voice Activity Detection (VAD) for automatic audio segmentation
/// Ported and refined from a Python version
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VadStrategy {
    Normal,
    Aggressive,
}

/// Voice Activity Detection (VAD) for automatic audio segmentation
/// Ported and refined from a Python version
pub struct VoiceActivityDetector {
    /// Sample rate
    sample_rate: u32,
    /// Buffer of recent RMS values for silence detection
    rms_buffer: VecDeque<f32>,
    /// Base silence threshold
    silence_threshold: f32,
    /// Base silence duration (seconds)
    base_silence_duration: f32,
    /// Minimum chunk duration (seconds)
    min_chunk_duration: f32,
    /// Maximum chunk duration (seconds)
    max_chunk_duration: f32,
    /// Splitting strategy
    strategy: VadStrategy,
    /// Time since recording started
    recording_started: Option<Instant>,
    /// Start time of current chunk
    chunk_started: Option<Instant>,
    /// Time when silence started
    silence_started: Option<Instant>,
    /// Total samples processed
    total_samples: usize,
    /// Samples processed in current chunk
    chunk_samples: usize,
    /// Speech frames in current chunk
    speech_frames: usize,
    /// Total frames in current chunk
    total_frames: usize,
}

impl VoiceActivityDetector {
    pub fn new_with_strategy(sample_rate: u32, strategy: VadStrategy) -> Self {
        // Start from defaults, then adjust by strategy
        let mut v = Self {
            sample_rate,
            rms_buffer: VecDeque::with_capacity(100),
            silence_threshold: 0.005,   // defaults for Normal
            base_silence_duration: 2.0, // defaults for Normal
            min_chunk_duration: 3.0,    // defaults for Normal
            max_chunk_duration: 30.0,   // defaults for Normal
            strategy,
            recording_started: None,
            chunk_started: None,
            silence_started: None,
            total_samples: 0,
            chunk_samples: 0,
            speech_frames: 0,
            total_frames: 0,
        };
        v.apply_strategy(strategy);
        v
    }

    fn apply_strategy(&mut self, strategy: VadStrategy) {
        match strategy {
            VadStrategy::Normal => {
                self.silence_threshold = 0.005;
                self.base_silence_duration = 2.0;
                self.min_chunk_duration = 3.0;
                self.max_chunk_duration = 30.0;
            }
            VadStrategy::Aggressive => {
                // Split earlier/finer: shorter required silence/min length, slightly higher threshold
                self.silence_threshold = 0.007;
                self.base_silence_duration = 1.0;
                self.min_chunk_duration = 1.5;
                self.max_chunk_duration = 25.0;
            }
        }
    }

    /// Mark the start of recording
    pub fn start_recording(&mut self) {
        self.recording_started = Some(Instant::now());
        self.chunk_started = Some(Instant::now());
        self.chunk_samples = 0;
        self.speech_frames = 0;
        self.total_frames = 0;
    }

    /// Return dynamic silence duration threshold (same logic as Python version)
    fn get_dynamic_silence_threshold(&self, current_duration: f32) -> f32 {
        match self.strategy {
            VadStrategy::Normal => {
                if current_duration < 8.0 {
                    self.base_silence_duration // 2.0s
                } else if current_duration < 15.0 {
                    1.0
                } else if current_duration < 25.0 {
                    0.5
                } else {
                    0.3
                }
            }
            VadStrategy::Aggressive => {
                if current_duration < 6.0 {
                    self.base_silence_duration // 1.0s
                } else if current_duration < 12.0 {
                    0.5
                } else if current_duration < 20.0 {
                    0.3
                } else {
                    0.2
                }
            }
        }
    }

    /// Check if the chunk contains speech
    fn contains_speech(&self) -> bool {
        if self.total_frames == 0 {
            return false;
        }

        // Compute ratio of speech frames
        let speech_ratio = self.speech_frames as f32 / self.total_frames as f32;

        // If >10% frames contain speech, consider the chunk to have speech (same as Python)
        let has_speech = speech_ratio > 0.1;

        if has_speech {
            println!(
                "  Speech present in chunk: {}/{} frames ({:.1}%)",
                self.speech_frames,
                self.total_frames,
                speech_ratio * 100.0
            );
        }

        has_speech
    }

    /// Process audio and decide whether to split the chunk
    pub fn process_audio(&mut self, samples: &[f32]) -> SplitDecision {
        if self.recording_started.is_none() {
            self.start_recording();
        }

        self.total_samples += samples.len();
        self.chunk_samples += samples.len();
        self.total_frames += 1;

        // Compute RMS (Root Mean Square)
        let rms = calculate_rms(samples);
        self.rms_buffer.push_back(rms);
        if self.rms_buffer.len() > 100 {
            self.rms_buffer.pop_front();
        }

        // Speech detection
        if rms > self.silence_threshold {
            self.speech_frames += 1;
        }

        // Elapsed time since chunk start (seconds)
        let chunk_duration = self.chunk_samples as f32 / self.sample_rate as f32;

        // Get dynamic required silence duration
        let required_silence_duration = self.get_dynamic_silence_threshold(chunk_duration);

        // Silence check
        let is_silent = rms < self.silence_threshold;

        if is_silent {
            // Record when silence started
            if self.silence_started.is_none() {
                self.silence_started = Some(Instant::now());
            }

            // Check how long silence has lasted
            if let Some(silence_start) = self.silence_started {
                let silence_duration = silence_start.elapsed().as_secs_f32();

                // Debug: print accumulated silence for long chunks
                if chunk_duration > 8.0 && silence_duration > 0.3 {
                    println!(
                        "  [silence: {:.1}s / required: {:.1}s @ {:.1}s]",
                        silence_duration, required_silence_duration, chunk_duration
                    );
                }

                // If min chunk length is met and required silence exceeded
                if chunk_duration >= self.min_chunk_duration
                    && silence_duration >= required_silence_duration
                {
                    // Ensure the chunk contains speech
                    if self.contains_speech() {
                        let reason = format!(
                            "detected {:.1}s silence after {:.1}s",
                            silence_duration, chunk_duration
                        );
                        self.reset_chunk();
                        return SplitDecision::Split { reason };
                    } else {
                        println!("  Skipping silence-only chunk");
                        self.reset_chunk();
                        return SplitDecision::Skip;
                    }
                }
            }
        } else {
            // Reset silence timer when speech is present
            self.silence_started = None;
        }

        // Force split at maximum chunk length
        if chunk_duration >= self.max_chunk_duration {
            println!(
                "Reached maximum duration ({} s); forced split",
                self.max_chunk_duration
            );
            let reason = format!("max {} s limit", self.max_chunk_duration);
            self.reset_chunk();
            return SplitDecision::Split { reason };
        }

        SplitDecision::Continue
    }

    /// Reset chunk tracking
    fn reset_chunk(&mut self) {
        self.chunk_started = Some(Instant::now());
        self.silence_started = None;
        self.chunk_samples = 0;
        self.speech_frames = 0;
        self.total_frames = 0;
        self.rms_buffer.clear();
    }

    // removed: unused getters and constructors
}

/// Result of chunk-splitting decision
#[derive(Debug, Clone)]
pub enum SplitDecision {
    /// Continue recording
    Continue,
    /// Split the chunk and process
    Split { reason: String },
    /// Skip silent chunk
    Skip,
}

/// Compute RMS (Root Mean Square)
fn calculate_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let sum_squares: f32 = samples.iter().map(|s| s * s).sum();
    (sum_squares / samples.len() as f32).sqrt()
}
