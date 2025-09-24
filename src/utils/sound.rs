use crate::utils::paths::resolve_resource;
use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

/// Play a sound asynchronously
pub fn play_sound_async(path: &str) {
    let path = path.to_string();
    std::thread::spawn(move || {
        if let Err(_e) = play_sound(&path) {
            // Log only on error (uncomment for debug)
            // eprintln!("⚠️ Sound playback error ({}): {:?}", path, e);
        }
    });
}

fn play_sound(path: &str) -> Result<()> {
    // Delegate to the sound worker (decode+playback in worker thread)
    let tx = get_or_start_worker();
    let _ = tx.send(SoundCmd::PlayPath(path.to_string()));
    Ok(())
}

// Preferred output device name (from UI setting)
static OUTPUT_DEVICE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
// Sound worker (keeps OutputStream in its thread)
static SOUND_TX: OnceLock<Mutex<Option<mpsc::Sender<SoundCmd>>>> = OnceLock::new();
// Whether sounds are enabled
static ENABLED: AtomicBool = AtomicBool::new(true);
// Volume percent (0..=100)
static VOLUME: AtomicU32 = AtomicU32::new(100);

enum SoundCmd {
    SetDevice(Option<String>),
    PlayPath(String),
    StartLoop {
        key: String,
        path: String,
        gap_ms: u64,
    },
    StopLoop {
        key: String,
    },
}

fn get_store() -> &'static Mutex<Option<String>> {
    OUTPUT_DEVICE.get_or_init(|| Mutex::new(None))
}

fn get_output_device_name() -> Option<String> {
    get_store().lock().ok().and_then(|g| g.clone())
}

pub fn set_output_device(name: Option<&str>) {
    if let Ok(mut g) = get_store().lock() {
        *g = name.map(|s| s.to_string());
    }
    // Update worker
    let tx = get_or_start_worker();
    let _ = tx.send(SoundCmd::SetDevice(name.map(|s| s.to_string())));
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
}

pub fn set_volume_percent(percent: f32) {
    let clamped = (percent.round() as u32).clamp(0, 100);
    VOLUME.store(clamped, Ordering::SeqCst);
}

fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

fn current_volume() -> f32 {
    VOLUME.load(Ordering::SeqCst) as f32 / 100.0
}

fn get_or_start_worker() -> mpsc::Sender<SoundCmd> {
    // Return existing worker if present
    if let Some(tx) = SOUND_TX
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|o| o.clone())
    {
        return tx;
    }
    // Initialize
    let (tx, rx) = mpsc::channel::<SoundCmd>();
    let tx_clone = tx.clone();
    let _ = SOUND_TX.get_or_init(|| Mutex::new(Some(tx_clone)));
    // Spawn worker thread
    std::thread::spawn(move || {
        let mut current_name: Option<String> = get_output_device_name();
        // Lazily-created output stream context. Keep None while idle to avoid
        // holding the device when no sound is playing.
        let mut ctx: Option<(OutputStream, OutputStreamHandle)> = None;
        let mut loops: HashMap<String, Arc<AtomicBool>> = HashMap::new();

        // Helper: rebuild context for the current device name
        fn rebuild_ctx(name: &Option<String>) -> Option<(OutputStream, OutputStreamHandle)> {
            if let Some(w) = name.as_ref() {
                let host = cpal::default_host();
                let mut sel = None;
                if let Ok(devs) = host.output_devices() {
                    for d in devs {
                        if let Ok(n) = d.name() {
                            if n == *w {
                                sel = Some(d);
                                break;
                            }
                        }
                    }
                }
                if let Some(dev) = sel {
                    if let Ok(pair) = OutputStream::try_from_device(&dev) {
                        // Small delay to give backend a moment to settle (avoids occasional glitches)
                        std::thread::sleep(Duration::from_millis(80));
                        return Some(pair);
                    }
                }
                None
            } else {
                // Avoid eager default stream open which can print ALSA errors on systems
                // without a configured default. Only open when playback is requested.
                None
            }
        }

        // Do not open a stream up-front to avoid ALSA stderr noise on headless/CI.
        // We'll (re)build on-demand when a sound is actually requested.

        while let Ok(cmd) = rx.recv() {
            match cmd {
                SoundCmd::SetDevice(name) => {
                    // Update selection and stop any loops. Do not immediately
                    // open the output stream; defer until actual playback.
                    if name != current_name {
                        for (_k, flag) in loops.drain() {
                            flag.store(true, Ordering::SeqCst);
                        }
                        current_name = name;
                        // Drop existing context to release the device while idle
                        ctx = None;
                    }
                }
                SoundCmd::PlayPath(p) => {
                    if !is_enabled() {
                        continue;
                    }
                    // Rebuild if needed just in case
                    if ctx.is_none() {
                        ctx = rebuild_ctx(&current_name);
                        // If still none (no device), skip silently
                        if ctx.is_none() {
                            continue;
                        }
                    }
                    if let Some((_, ref handle)) = ctx {
                        let resolved =
                            resolve_resource(&p).unwrap_or_else(|| std::path::PathBuf::from(&p));
                        if let Ok(file) = File::open(&resolved) {
                            let reader = BufReader::new(file);
                            if let Ok(decoder) = Decoder::new(reader) {
                                if let Ok(sink) = Sink::try_new(handle) {
                                    sink.set_volume(current_volume());
                                    sink.append(decoder);
                                    sink.sleep_until_end();
                                }
                            }
                        }
                    }
                    // After a one-shot playback, release the device to avoid
                    // occupying the output when idle. Keep it while loops are running.
                    if loops.is_empty() {
                        ctx = None; // drop OutputStream/Handle
                    }
                }
                SoundCmd::StartLoop { key, path, gap_ms } => {
                    // Stop existing loop if any
                    if let Some(flag) = loops.remove(&key) {
                        flag.store(true, Ordering::SeqCst);
                    }
                    let stop = Arc::new(AtomicBool::new(false));
                    loops.insert(key.clone(), stop.clone());
                    // Loop playback thread (using current handle)
                    // Ensure we have a handle before entering the loop to avoid ALSA spam
                    if ctx.is_none() {
                        ctx = rebuild_ctx(&current_name);
                    }
                    let handle_for_loop = ctx.as_ref().map(|(_, h)| h.clone());
                    std::thread::spawn(move || loop {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        if let Some(ref h) = handle_for_loop {
                            if !is_enabled() {
                                if gap_ms > 0 {
                                    std::thread::sleep(std::time::Duration::from_millis(gap_ms));
                                }
                                continue;
                            }
                            let resolved = resolve_resource(&path)
                                .unwrap_or_else(|| std::path::PathBuf::from(&path));
                            if let Ok(file) = File::open(&resolved) {
                                let reader = BufReader::new(file);
                                if let Ok(decoder) = Decoder::new(reader) {
                                    if let Ok(sink) = Sink::try_new(h) {
                                        sink.set_volume(current_volume());
                                        sink.append(decoder);
                                        sink.sleep_until_end();
                                    }
                                }
                            }
                        }
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        if gap_ms > 0 {
                            std::thread::sleep(std::time::Duration::from_millis(gap_ms));
                        }
                    });
                }
                SoundCmd::StopLoop { key } => {
                    if let Some(flag) = loops.remove(&key) {
                        flag.store(true, Ordering::SeqCst);
                    }
                    // If no more loops remain, we can drop the context to
                    // release the output device while idle.
                    if loops.is_empty() {
                        ctx = None;
                    }
                }
            }
        }
    });
    tx
}

pub fn start_loop(key: &str, path: &str, gap_ms: u64) {
    let tx = get_or_start_worker();
    let _ = tx.send(SoundCmd::StartLoop {
        key: key.to_string(),
        path: path.to_string(),
        gap_ms,
    });
}

pub fn stop_loop(key: &str) {
    let tx = get_or_start_worker();
    let _ = tx.send(SoundCmd::StopLoop {
        key: key.to_string(),
    });
}

// Direct synchronous playback was removed; only prewarmed worker mode is used
