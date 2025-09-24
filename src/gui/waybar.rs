use crate::utils::app_config_dir;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::core::SimpleRecState;
use crate::i18n;

fn status_path() -> PathBuf {
    app_config_dir().join("status.json")
}

fn ensure_parent_dir(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
}

pub fn write_status(state: SimpleRecState) {
    let (text, tooltip, color, class, alt) = match state {
        SimpleRecState::Idle => ("○", &i18n::tr("status-idle"), "#22aa22", "idle", "idle"),
        SimpleRecState::Recording => (
            "●",
            &i18n::tr("status-recording"),
            "#dd3333",
            "recording",
            "rec",
        ),
        SimpleRecState::Processing => (
            "●",
            &i18n::tr("status-processing"),
            "#d0c000",
            "processing",
            "proc",
        ),
        SimpleRecState::Busy => ("●", &i18n::tr("status-busy"), "#6c757d", "busy", "busy"),
    };
    let json = format!(
        "{{\"text\":\"{}\",\"tooltip\":\"{}\",\"class\":\"{}\",\"alt\":\"{}\",\"color\":\"{}\"}}",
        text, tooltip, class, alt, color
    );
    let path = status_path();
    ensure_parent_dir(&path);
    // Atomic-ish write: write to temp then rename
    let tmp = path.with_extension("json.tmp");
    if let Ok(mut f) = fs::File::create(&tmp) {
        let _ = f.write_all(json.as_bytes());
        let _ = f.flush();
        let _ = fs::rename(tmp, path);
    }
}
