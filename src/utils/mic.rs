#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Try to open the microphone briefly to trigger OS permission prompt (macOS TCC).
/// - Selects device by host name + per-host index if provided; otherwise uses defaults.
/// - Starts the input stream, waits a short time, then pauses and drops it.
/// Returns Ok(()) on successful start; Err with message otherwise.
#[cfg(target_os = "macos")]
pub fn preflight_mic_access(
    host_pref: Option<&str>,
    index_in_host: Option<usize>,
) -> Result<(), String> {
    let host = if let Some(h) = host_pref {
        // Find matching host by Debug name (lowercased), e.g., "coreaudio", "pipewire", etc.
        let mut chosen = None;
        for id in cpal::available_hosts() {
            let name = format!("{:?}", id).to_lowercase();
            if name == h.to_lowercase() {
                chosen = Some(id);
                break;
            }
        }
        if let Some(id) = chosen {
            cpal::host_from_id(id).map_err(|e| e.to_string())?
        } else {
            cpal::default_host()
        }
    } else {
        cpal::default_host()
    };

    // Decide input device
    let device = if let Some(idx) = index_in_host {
        let mut iter = host.input_devices().map_err(|e| e.to_string())?;
        iter.nth(idx)
            .ok_or_else(|| "Selected input device not found".to_string())?
    } else {
        host.default_input_device()
            .ok_or_else(|| "Default input device not found".to_string())?
    };

    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.config();

    // No-op input callback; just touching the device is enough to raise TCC prompt.
    let err_fn = |_e: cpal::StreamError| {};

    let stream = match sample_format {
        cpal::SampleFormat::I16 => device
            .build_input_stream(&config, |_data: &[i16], _| {}, err_fn, None)
            .map_err(|e| e.to_string())?,
        cpal::SampleFormat::U16 => device
            .build_input_stream(&config, |_data: &[u16], _| {}, err_fn, None)
            .map_err(|e| e.to_string())?,
        _ => device
            .build_input_stream(&config, |_data: &[f32], _| {}, err_fn, None)
            .map_err(|e| e.to_string())?,
    };

    stream.play().map_err(|e| e.to_string())?;
    std::thread::sleep(Duration::from_millis(300));
    let _ = stream.pause();
    drop(stream);
    Ok(())
}
