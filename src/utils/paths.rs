use directories::BaseDirs;
use std::path::PathBuf;

/// Application config directory (OS standard)
/// Linux: ~/.config/HootVoice
/// macOS: ~/Library/Application Support/HootVoice
/// Windows: %APPDATA%\\HootVoice
pub fn app_config_dir() -> PathBuf {
    if let Some(base) = BaseDirs::new() {
        return base.config_dir().join("HootVoice");
    }
    // Fallback: current working directory
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Directory that contains the current executable
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// Resolve a resource path using this precedence:
/// config dir -> executable dir -> current working dir
pub fn resolve_resource(filename: &str) -> Option<PathBuf> {
    // 1) config dir
    let cfg = app_config_dir().join(filename);
    if cfg.exists() {
        return Some(cfg);
    }

    // 2) executable dir
    if let Some(exe) = exe_dir() {
        let p = exe.join(filename);
        if p.exists() {
            return Some(p);
        }
    }

    // 3) current working dir
    let cwd = PathBuf::from(filename);
    if cwd.exists() {
        return Some(cwd);
    }

    None
}

// removed: preferred_location (unused)
