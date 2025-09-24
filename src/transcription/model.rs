use anyhow::{anyhow, Context, Result};
use reqwest::blocking as http;
use reqwest::header::RANGE;
use reqwest::redirect::Policy as RedirectPolicy;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Whisper official ggml model filenames we support and their URLs/sizes.
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub filename: &'static str,
    pub url: &'static str,
    pub size_bytes: u64,         // approximate/declared size
    pub label_key: &'static str, // i18n key for short label
    pub speed_rating: f32,       // 1..=5 (5 fastest)
    pub quality_rating: f32,     // 1..=5 (5 best)
    pub notes_key: &'static str, // i18n key for guidance text
}

pub const SUPPORTED_MODELS: &[ModelInfo] = &[
    ModelInfo {
        filename: "ggml-tiny.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        size_bytes: 39 * 1_000_000,
        label_key: "model-label-tiny",
        speed_rating: 5.0,
        quality_rating: 2.0,
        notes_key: "model-note-tiny",
    },
    ModelInfo {
        filename: "ggml-base.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        size_bytes: 142 * 1_000_000,
        label_key: "model-label-base",
        speed_rating: 4.0,
        quality_rating: 3.0,
        notes_key: "model-note-base",
    },
    ModelInfo {
        filename: "ggml-small.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        size_bytes: 465 * 1_000_000,
        label_key: "model-label-small",
        speed_rating: 3.0,
        quality_rating: 4.0,
        notes_key: "model-note-small",
    },
    ModelInfo {
        filename: "ggml-medium.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
        size_bytes: 1_500 * 1_000_000,
        label_key: "model-label-medium",
        speed_rating: 2.0,
        quality_rating: 4.5,
        notes_key: "model-note-medium",
    },
    ModelInfo {
        filename: "ggml-large-v3.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
        size_bytes: 3_095 * 1_000_000, // ~2.9GB
        label_key: "model-label-large",
        speed_rating: 1.0,
        quality_rating: 5.0,
        notes_key: "model-note-large",
    },
];

pub fn model_info_for_filename(name: &str) -> Option<&'static ModelInfo> {
    SUPPORTED_MODELS.iter().find(|m| m.filename == name)
}

/// Auto-download Whisper model if missing
pub fn ensure_model(model_path: &Path) -> Result<()> {
    if model_path.exists() {
        return Ok(());
    }

    // choose URL based on filename if known, otherwise fall back to small
    let filename = model_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("ggml-small.bin");
    let model_info = model_info_for_filename(filename);
    let url = model_info
        .map(|m| m.url)
        .unwrap_or("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin");

    let expected_size = model_info.map(|m| m.size_bytes).unwrap_or(0);
    let size_mb = expected_size as f64 / 1_000_000.0;

    eprintln!("===================================================");
    eprintln!("📥 First‑time Whisper model download");
    eprintln!("===================================================");
    eprintln!("File: {}", filename);
    eprintln!("Size: {:.1} MB", size_mb);
    eprintln!("Destination: {}", model_path.display());
    eprintln!("URL: {}", url);
    eprintln!("---------------------------------------------------");
    eprintln!("Downloading...");

    // Download with progress callback
    download_with_progress(url, model_path, |downloaded, total| {
        let percent = if total > 0 {
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        let downloaded_mb = downloaded as f64 / 1_000_000.0;
        let total_mb = total as f64 / 1_000_000.0;

        // Render a simple progress bar
        let bar_width = 40;
        let filled = (bar_width as f64 * percent / 100.0) as usize;
        let bar: String = "█".repeat(filled) + &"░".repeat(bar_width - filled);

        eprint!(
            "\r[{}] {:.1}% ({:.1}/{:.1} MB)",
            bar, percent, downloaded_mb, total_mb
        );
        std::io::stderr().flush().ok();
    })?;

    eprintln!("\n===================================================");
    eprintln!("✅ Download completed!");
    eprintln!("===================================================");

    Ok(())
}

/// Stream download with progress callback. Writes to a temp file then atomically moves.
pub fn download_with_progress<F>(url: &str, dest: &Path, mut on_progress: F) -> Result<()>
where
    F: FnMut(u64, u64) + Send,
{
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create parent dir: {}", parent.display()))?;
    }

    // Build HTTP client with sensible timeouts and explicit UA to avoid silent hangs
    let client = http::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // Generous overall timeout for large files; connection/setup should fail fast
        .timeout(Duration::from_secs(60 * 60))
        // Avoid some HTTP/2 oddities seen with certain CDNs / proxies
        .http1_only()
        // Follow redirects from huggingface -> CDN endpoints
        .redirect(RedirectPolicy::limited(10))
        .user_agent(format!(
            "HootVoice/{} (+https://github.com/agata/hootvoice)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .context("build http client")?;
    eprintln!("[Download] Requesting: {}", url);
    let resp = client.get(url).send().context("request failed")?;
    eprintln!(
        "[Download] Response: {:?} {}",
        resp.version(),
        resp.status()
    );
    let total = resp
        .content_length()
        .or_else(|| {
            // fallback to known size
            dest.file_name()
                .and_then(|s| s.to_str())
                .and_then(model_info_for_filename)
                .map(|m| m.size_bytes)
        })
        .unwrap_or(0);

    if !resp.status().is_success() {
        return Err(anyhow!("download failed: {}", resp.status()));
    }

    let mut reader = resp;
    let mut downloaded: u64 = 0;

    // write to temporary file first
    let tmp_path = dest.with_extension("download");
    let mut file = File::create(&tmp_path).context("create temp file")?;

    let mut buf = [0u8; 1024 * 64];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(anyhow!("read error: {e}")),
        } as u64;
        file.write_all(&buf[..n as usize]).context("write file")?;
        downloaded = downloaded.saturating_add(n);
        on_progress(downloaded, total);
    }
    file.flush().ok();

    // atomically move into place
    std::fs::rename(&tmp_path, dest).context("rename downloaded file")?;
    Ok(())
}

/// Cancelable variant of download_with_progress
pub fn download_with_progress_cancelable<F>(
    url: &str,
    dest: &Path,
    cancel: Arc<AtomicBool>,
    mut on_progress: F,
) -> Result<()>
where
    F: FnMut(u64, u64) + Send,
{
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create parent dir: {}", parent.display()))?;
    }

    // Support resume from temp file
    let tmp_path = dest.with_extension("download");
    let mut start: u64 = 0;
    if let Ok(meta) = std::fs::metadata(&tmp_path) {
        start = meta.len();
    }

    // HTTP client with timeouts and UA so failures surface promptly
    let client = http::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60 * 60))
        .http1_only()
        .redirect(RedirectPolicy::limited(10))
        .user_agent(format!(
            "HootVoice/{} (+https://github.com/agata/hootvoice)",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .context("build http client")?;
    let mut req = client.get(url);
    if start > 0 {
        req = req.header(RANGE, format!("bytes={}-", start));
    }
    eprintln!(
        "[Download] {} request: {} (resume from {} bytes)",
        if start > 0 { "Resuming" } else { "Starting" },
        url,
        start
    );
    let resp = req.send().context("request failed")?;
    eprintln!(
        "[Download] Response: {:?} {}",
        resp.version(),
        resp.status()
    );
    if !(resp.status().is_success() || resp.status().as_u16() == 206) {
        return Err(anyhow!("download failed: {}", resp.status()));
    }
    // total expected size
    let remaining = resp.content_length().unwrap_or(0);
    let mut total = start + remaining;
    if total == 0 {
        total = dest
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(model_info_for_filename)
            .map(|m| m.size_bytes)
            .unwrap_or(0);
    }

    // If resume was requested but server ignored Range (returned 200), restart from scratch
    let resume_ok = start > 0 && resp.status().as_u16() == 206;
    // open file (append if resuming and server accepted range, otherwise create new)
    let mut file: File = if resume_ok {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&tmp_path)?
    } else {
        // Fresh download
        File::create(&tmp_path).context("create temp file")?
    };

    // initial progress callback for resumed portion
    if resume_ok && start > 0 {
        on_progress(start, total);
    }

    let mut reader = resp;
    let mut downloaded: u64 = 0;
    let mut buf = [0u8; 1024 * 64];
    loop {
        if cancel.load(Ordering::SeqCst) {
            // keep tmp file for resume
            return Err(anyhow!("download canceled"));
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(anyhow!("read error: {e}")),
        } as u64;
        file.write_all(&buf[..n as usize]).context("write file")?;
        downloaded = downloaded.saturating_add(n);
        // If we restarted fresh, don't include prior start offset
        let base = if resume_ok { start } else { 0 };
        on_progress(base + downloaded, total);
    }
    file.flush().ok();
    std::fs::rename(&tmp_path, dest).context("rename downloaded file")?;
    Ok(())
}

// removed: default_model_path, supported_model_labels (unused)
