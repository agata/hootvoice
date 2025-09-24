use anyhow::{Context, Result};
use std::time::Instant;
use unicode_categories::UnicodeCategories;
use whisper_rs::{FullParams, SamplingStrategy, WhisperState};

#[allow(dead_code)]
pub struct TranscriptionResult {
    pub text: String,
    pub segments: Vec<Segment>,
    pub duration_sec: f32,
    pub rtf: f32,
}

#[allow(dead_code)]
pub struct Segment {
    pub start: f32,
    pub end: f32,
    pub text: String,
    pub no_speech_prob: f32,
}

#[derive(Clone, Debug)]
pub struct WhisperOptimizationParams {
    pub no_timestamps: bool,
    pub token_timestamps: bool,
    pub use_physical_cores: bool,
    // New: decoding + context controls
    pub enable_beam_search: bool,
    pub beam_size: i32,
    pub temperature: f32,
    pub n_max_text_ctx: i32,
    pub no_context: bool,
}

impl Default for WhisperOptimizationParams {
    fn default() -> Self {
        Self {
            no_timestamps: true,
            token_timestamps: false,
            use_physical_cores: true,
            // Prefer small beam search for stable punctuation
            enable_beam_search: true,
            beam_size: 3,
            temperature: 0.0,
            // Keep long text context by default (but we disable context across chunks below)
            n_max_text_ctx: 16384,
            no_context: false,
        }
    }
}

/// Reusable-state variant: call this repeatedly with the same `state` to avoid init overhead.
pub fn transcribe_with_state(
    state: &mut WhisperState,
    pcm: &[f32],
    language: Option<&str>,
    optimization: Option<&WhisperOptimizationParams>,
) -> Result<TranscriptionResult> {
    // Optimization settings (or defaults)
    let default_opt = WhisperOptimizationParams::default();
    let opt = optimization.unwrap_or(&default_opt);

    // Recommendation: Greedy or small beam
    let mut params = if opt.enable_beam_search {
        FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: opt.beam_size.max(1),
            patience: 1.0,
        })
    } else {
        FullParams::new(SamplingStrategy::Greedy { best_of: 1 })
    };

    // Language, threading and features (auto-detect if None)
    params.set_language(language);

    // Threads
    let mut n_threads = if opt.use_physical_cores {
        // Prefer physical cores (HT often doesn't help)
        num_cpus::get_physical() as i32
    } else {
        // Or all logical cores
        num_cpus::get() as i32
    };
    // Avoid too many threads (cap at 4)
    if n_threads > 4 {
        n_threads = 4;
    }
    params.set_n_threads(n_threads.max(1));
    params.set_translate(false);

    // Timestamp options (toggle for diagnostics)
    params.set_no_timestamps(opt.no_timestamps);
    params.set_token_timestamps(opt.token_timestamps);

    // Decoding / context controls
    params.set_temperature(opt.temperature);
    // Enable temperature fallback to mitigate decoding loops/repetitions
    params.set_temperature_inc(0.2);
    params.set_n_max_text_ctx(opt.n_max_text_ctx);
    // IMPORTANT: We reuse WhisperState across VAD chunks. To avoid repeated
    // prefixes leaking from prior chunks, force no_context for each call.
    // Users can still override via settings later if we expose it.
    params.set_no_context(true);
    // Mark field as used to satisfy Clippy when we keep forcing true
    let _ = opt.no_context;
    params.set_single_segment(true);
    params.set_token_timestamps(false);
    // Reduce blank token influence
    params.set_suppress_blank(true);
    // Non-speech token suppression + confidence filter
    params.set_suppress_nst(true);
    params.set_logprob_thold(-0.7);
    params.set_entropy_thold(2.4);
    // no-speech threshold (combine with downstream filter)
    params.set_no_speech_thold(0.90);

    // Light initial prompt depending on language; skip when auto
    if let Some(lang) = language {
        match lang {
            "ja" => {
                params.set_initial_prompt("This input is Japanese. Please add proper punctuation and quotation marks where appropriate.");
            }
            "en" => {
                params.set_initial_prompt(
                    "The following is English. Use proper punctuation like commas and periods.",
                );
            }
            _ => {}
        }
    }

    let start = Instant::now();
    state
        .full(params, pcm)
        .context("whisper inference failed")?;
    let duration = start.elapsed();

    let audio_sec = pcm.len() as f32 / 16_000.0;
    let rtf = if audio_sec > 0.0 {
        duration.as_secs_f32() / audio_sec
    } else {
        0.0
    };

    let mut full_text = String::new();
    let mut segments = Vec::new();

    let mut last_ns: f32 = 0.0;
    for seg in state.as_iter() {
        // Timestamps are disabled but the iterator is safe to use
        let start = seg.start_timestamp() as f32 / 100.0;
        let end = seg.end_timestamp() as f32 / 100.0;
        let text = seg.to_string();
        let ns = seg.no_speech_probability();

        full_text.push_str(&text);
        full_text.push(' ');

        segments.push(Segment {
            start,
            end,
            text,
            no_speech_prob: ns,
        });
        last_ns = ns;
    }

    // Post-output filter
    let mut text_out = full_text.trim().to_string();
    // Collapse obvious immediate repeated phrases (decoder loops) before final filtering
    text_out = collapse_repetitions(&text_out);
    if should_suppress_output(&text_out, last_ns) {
        text_out.clear();
    }

    Ok(TranscriptionResult {
        text: text_out,
        segments,
        duration_sec: duration.as_secs_f32(),
        rtf,
    })
}

// Whether the string consists only of punctuation/whitespace
fn is_punct_or_space_only(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_whitespace() || c.is_ascii_punctuation() || c.is_punctuation())
}

fn should_suppress_output(text: &str, last_no_speech_prob: f32) -> bool {
    // Filter by no-speech prob and punctuation-only text (let anything non-trivial pass)
    if last_no_speech_prob >= 0.85 {
        return true;
    }
    let t = text.trim();
    if is_punct_or_space_only(t) {
        return true;
    }
    false
}

/// Collapse contiguous repeated phrases to mitigate rare decoder loops.
/// Two stages:
/// 1) Sentence-level immediate dedup (exact match)
/// 2) Char-level block collapse for repeated blocks (>=12 chars)
fn collapse_repetitions(s: &str) -> String {
    // 1) Sentence-level immediate dedup
    let mut out_sentences: Vec<String> = Vec::new();
    let mut buf = String::new();
    for ch in s.chars() {
        buf.push(ch);
        if matches!(ch, '。' | '！' | '？' | '!' | '?' | '\n') {
            let cur = buf.trim().to_string();
            if let Some(prev) = out_sentences.last() {
                if prev == &cur {
                    // skip exact immediate duplicate
                    buf.clear();
                    continue;
                }
            }
            out_sentences.push(cur);
            buf.clear();
        }
    }
    if !buf.trim().is_empty() {
        let cur = buf.trim().to_string();
        if out_sentences.last().map(|p| p != &cur).unwrap_or(true) {
            out_sentences.push(cur);
        }
    }
    let out = out_sentences.join(" ");

    // 2) Char-level block collapse near any position
    let chars: Vec<char> = out.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    let mut collapsed = String::new();
    while i < n {
        let mut collapsed_here = false;
        // Check block sizes from large to small to collapse longer repeats first
        let max_block = 128usize.min(n.saturating_sub(i) / 2);
        let min_block = 12usize;
        let mut w = max_block;
        while w >= min_block {
            if i + 2 * w <= n && chars[i..i + w] == chars[i + w..i + 2 * w] {
                // Found at least two repeats; count how many and keep only one
                let mut j = i + w;
                while j + w <= n && chars[i..i + w] == chars[j..j + w] {
                    j += w;
                }
                for c in &chars[i..i + w] {
                    collapsed.push(*c);
                }
                i = j;
                collapsed_here = true;
                break;
            }
            if w == min_block {
                break;
            }
            w -= 1;
        }
        if !collapsed_here {
            collapsed.push(chars[i]);
            i += 1;
        }
    }
    collapsed.trim().to_string()
}
