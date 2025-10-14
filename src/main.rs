#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]
// On Windows hide console in release builds. Debug keeps console for diagnostics.

use anyhow::Result;
use eframe::NativeOptions;
// no path imports needed here

mod app;
mod audio;
mod core;
mod dictionary;
mod gui;
mod hotkey;
mod i18n;
mod llm;
mod transcription;
mod utils;
// removed updater module (unused)
#[cfg(target_os = "macos")]
mod macos;

use fs2::FileExt;
use gui::RootApp;
use std::fs::OpenOptions;
use std::io;
use std::sync::OnceLock;
use utils::app_config_dir;

const APP_NAME: &str = "HootVoice";

static INSTANCE_LOCK: OnceLock<std::fs::File> = OnceLock::new();

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    // Default filter suppresses noisy WGPU/eframe warnings (like surface timeouts)
    // Users can override fully via RUST_LOG if desired.
    let default_directives = "info,egui=error,epaint=error,eframe=error,egui_wgpu=error,wgpu=error,wgpu_core=error,wgpu_hal=error,naga=error";
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_directives));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn acquire_single_instance_lock() -> Result<AcquireResult, io::Error> {
    // Use a deterministic, writable per-user dir to place the lock file.
    // e.g. on macOS: ~/Library/Application Support/HootVoice
    let dir = app_config_dir();
    std::fs::create_dir_all(&dir)?;
    let lock_path = dir.join("instance.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    eprintln!("[single-instance] lock file: {}", lock_path.display());

    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = INSTANCE_LOCK.set(file);
            eprintln!("[single-instance] lock acquired");
            Ok(AcquireResult::Acquired)
        }
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
            eprintln!("[single-instance] already running (lock busy)");
            Ok(AcquireResult::AlreadyRunning)
        }
        Err(err) => {
            eprintln!("[single-instance] lock error: {}", err);
            Err(err)
        }
    }
}

#[allow(dead_code)]
enum AcquireResult {
    Acquired,
    AlreadyRunning,
}

fn main() -> Result<()> {
    // macOS: always capture stdout/stderr to persistent log so Finder launches are diagnosable
    #[cfg(target_os = "macos")]
    {
        init_persistent_logging_macos();
    }
    init_logging();
    tracing::info!("{} version {}", APP_NAME, env!("CARGO_PKG_VERSION"));
    // Single-instance guard: explicit file lock in a writable per-user dir
    match acquire_single_instance_lock() {
        Ok(AcquireResult::Acquired) => {
            // ok
        }
        Ok(AcquireResult::AlreadyRunning) => {
            eprintln!("{} is already running.", APP_NAME);
            #[cfg(target_os = "macos")]
            {
                macos::ui::show_already_running_alert();
                macos::ui::try_activate_existing_instance();
            }
            return Ok(());
        }
        Err(e) => {
            // On error, continue without exclusivity to avoid silent exit
            eprintln!("Failed to initialize single instance: {} — continuing", e);
        }
    }

    // System tray removed; RootApp manages initial setup and hotkeys

    // Update checks disabled for now; may restore on a separate thread later

    // Launch GUI application (Wayland-friendly)
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(APP_NAME) // Wayland app_id
            .with_title(APP_NAME) // Window title (also macOS menu)
            // macOS: avoid overriding Dock icon; always use Info.plist .icns
            .with_icon(egui::IconData::default())
            // Slightly larger default window so the first‑run wizard fits comfortably
            .with_inner_size(egui::vec2(880.0, 620.0))
            .with_resizable(true)
            .with_transparent(false)
            .with_maximized(false)
            .with_fullscreen(false)
            // Start visible to show the settings screen on first launch
            .with_visible(true),
        renderer: eframe::Renderer::Wgpu,
        // VSync configuration:
        // - On Linux, some drivers/compositors can cause frequent surface timeouts
        //   when waiting for vsync, which leads to repeated warnings like:
        //   "Dropped frame with error: A timeout was encountered while trying to acquire the next frame".
        //   To avoid this, default to disabling vsync on Linux (maps to AutoNoVsync).
        // - On other platforms keep vsync enabled by default.
        // - Allow override via env var: HOOTVOICE_VSYNC=0/1
        vsync: {
            let env = std::env::var("HOOTVOICE_VSYNC").ok();
            if let Some(v) = env.as_deref() {
                matches!(v, "1" | "true" | "TRUE" | "on" | "ON")
            } else {
                #[cfg(target_os = "linux")]
                {
                    false
                }
                #[cfg(not(target_os = "linux"))]
                {
                    true
                }
            }
        },
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        hardware_acceleration: eframe::HardwareAcceleration::Preferred,
        run_and_return: false,
        event_loop_builder: None,
        window_builder: None,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            #[cfg(target_os = "macos")]
            {
                // macOS: set menubar after NSApp is initialized
                macos::menu::setup_menubar(APP_NAME);
            }
            // Configure UI fonts
            gui::fonts::setup_custom_fonts(&cc.egui_ctx);
            // Slightly upscale UI for readability
            let ppp = cc.egui_ctx.pixels_per_point();
            cc.egui_ctx.set_pixels_per_point(ppp * 1.10);
            cc.egui_ctx
                .request_repaint_after(std::time::Duration::from_secs(5));
            Ok(Box::new(RootApp::new()))
        }),
    )
    .unwrap();

    Ok(())
}

#[cfg(target_os = "macos")]
fn init_persistent_logging_macos() {
    use std::fs::{create_dir_all, OpenOptions};
    use std::io::Write;
    use std::os::fd::AsRawFd;

    let home = std::env::var("HOME").unwrap_or_else(|_| String::from(""));
    if home.is_empty() {
        return;
    }
    let log_dir = format!("{}/Library/Logs/{}", home, APP_NAME);
    let _ = create_dir_all(&log_dir);
    let log_path = format!("{}/{}.log", log_dir, APP_NAME);

    // Append mode so multiple runs accumulate
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = writeln!(
            &std::io::stderr(),
            "\n===== Launch {} =====",
            chrono::Local::now().to_rfc3339()
        );
        // Duplicate file descriptor onto stdout(1) and stderr(2)
        unsafe {
            let fd = file.as_raw_fd();
            let _ = libc::dup2(fd, 1);
            let _ = libc::dup2(fd, 2);
        }
        // After dup2, writes to println!/eprintln! go to the log file
        eprintln!("[{}] persistent logging at: {}", APP_NAME, log_path);
    }
}
