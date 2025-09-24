use std::path::{Path, PathBuf};
use std::process::Command;

/// Open a path in the OS default file manager
pub fn reveal_in_file_manager(path: &Path) {
    // Try best-effort to open a directory
    let target_dir: PathBuf = if path.exists() {
        if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(Path::new(".")).to_path_buf()
        }
    } else {
        // Create the expected directory before opening
        let _ = std::fs::create_dir_all(path);
        path.to_path_buf()
    };

    #[cfg(target_os = "linux")]
    {
        // Try fallbacks if xdg-open is missing or fails
        if Command::new("xdg-open").arg(&target_dir).spawn().is_err() {
            let _ = Command::new("gio")
                .args(["open", target_dir.to_string_lossy().as_ref()])
                .spawn()
                .or_else(|_| Command::new("nautilus").arg(&target_dir).spawn())
                .or_else(|_| Command::new("dolphin").arg(&target_dir).spawn())
                .or_else(|_| Command::new("thunar").arg(&target_dir).spawn());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(&target_dir).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        // explorer can open both directories and files
        let _ = Command::new("explorer").arg(&target_dir).spawn();
    }
}

/// Open a URL in the user's default web browser
pub fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    {
        if Command::new("xdg-open").arg(url).spawn().is_err() {
            let _ = Command::new("gio").args(["open", url]).spawn();
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        // Use cmd /C start to delegate to shell and default browser
        let _ = Command::new("cmd").args(["/C", "start", "", url]).spawn();
    }
}
