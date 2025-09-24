use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Append a log line to an in-memory ring buffer (max 1000 lines) and persist to file.
/// Runtime behavior is append-only for performance; no file rewrite.
pub fn push_log_and_persist(
    logs_arc: &Arc<Mutex<VecDeque<String>>>,
    log_path: &Path,
    log_line: &str,
) {
    if let Ok(mut logs) = logs_arc.lock() {
        if logs.len() >= 1000 {
            // Keep only the newest 1000 lines in memory
            logs.pop_front();
        }
        logs.push_back(log_line.to_string());
    }

    // Append-only during runtime for performance
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(file, "{}", log_line);
    }
}

/// Startup-only: trim the existing log file to the last `max_lines` lines.
pub fn trim_log_file_startup(log_path: &Path, max_lines: usize) {
    if max_lines == 0 {
        return;
    }
    let file = match File::open(log_path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let mut ring: VecDeque<String> = VecDeque::with_capacity(max_lines);
    let mut total = 0usize;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        match line {
            Ok(l) => {
                total += 1;
                if ring.len() == max_lines {
                    ring.pop_front();
                }
                ring.push_back(l);
            }
            Err(_) => {
                // Ignore malformed lines
            }
        }
    }
    if total > max_lines {
        if let Ok(mut out) = File::create(log_path) {
            for l in ring.iter() {
                let _ = writeln!(out, "{}", l);
            }
        }
    }
}
