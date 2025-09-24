use super::SettingsWindow;
use crate::audio::stream::build_input_stream_f32;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{SampleFormat as WavSampleFormat, WavSpec, WavWriter};
use std::sync::{Arc, Mutex};

impl SettingsWindow {
    // Test recording: start recording with the currently selected input device
    pub(super) fn start_test_recording_for_selected_input(&mut self) -> Result<(), String> {
        if self.is_test_recording {
            return Ok(());
        }
        let host_opt = self.settings.input_host.clone();
        let idx_opt = self.settings.input_device_index_in_host;
        match (host_opt, idx_opt) {
            (Some(h), Some(i)) => self.start_test_recording_host_index(Some(h.as_str()), Some(i)),
            _ => self.start_test_recording_host_index(None, None),
        }
    }

    pub(super) fn start_test_recording_host_index(
        &mut self,
        host: Option<&str>,
        idx: Option<usize>,
    ) -> Result<(), String> {
        let host_sel = if let Some(h) = host {
            let mut chosen = None;
            for id in cpal::available_hosts() {
                if format!("{:?}", id).to_lowercase() == h.to_lowercase() {
                    chosen = Some(id);
                    break;
                }
            }
            if let Some(id) = chosen {
                cpal::host_from_id(id).unwrap_or_else(|_| cpal::default_host())
            } else {
                cpal::default_host()
            }
        } else {
            cpal::default_host()
        };
        let device = if let Some(i) = idx {
            if let Ok(devs) = host_sel.input_devices() {
                devs.into_iter().nth(i)
            } else {
                None
            }
        } else {
            host_sel.default_input_device()
        };
        let device = device.ok_or_else(|| "Input device not found".to_string())?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.config();
        let sr = config.sample_rate.0;
        let ch = config.channels as u16;
        let buf: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(
            sr as usize * ch as usize * 60,
        )));
        let stream = build_input_stream_f32(&device, config.clone(), sample_format, buf.clone())
            .map_err(|e| format!("{:?}", e))?;
        stream.play().map_err(|e| e.to_string())?;
        self.test_stream = Some(stream);
        self.test_buffer = Some(buf);
        self.test_sample_rate = sr;
        self.test_channels = ch;
        Ok(())
    }

    pub(super) fn stop_and_save_test_recording(&mut self) -> Result<(), String> {
        self.test_started_at = None;
        self.is_test_recording = false;
        if let Some(s) = self.test_stream.take() {
            // Pause first so OS releases the device promptly
            let _ = s.pause();
        }
        let data = if let Some(buf) = self.test_buffer.take() {
            let guard = buf.lock().map_err(|_| "Failed to lock recording buffer")?;
            guard.clone()
        } else {
            Vec::new()
        };
        let sr = self.test_sample_rate;
        let ch = self.test_channels;
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let default_name = format!("test-recording-{}.wav", ts);
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("WAV", &["wav"])
            .set_file_name(&default_name)
            .save_file()
        {
            self.save_buffer_as_wav(&path, &data, sr, ch)?;
        }
        Ok(())
    }

    pub(super) fn save_buffer_as_wav(
        &self,
        path: &std::path::Path,
        samples: &[f32],
        sample_rate: u32,
        channels: u16,
    ) -> Result<(), String> {
        let spec = WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 32,
            sample_format: WavSampleFormat::Float,
        };
        let mut writer = WavWriter::create(path, spec).map_err(|e| e.to_string())?;
        for &s in samples {
            writer.write_sample::<f32>(s).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())?;
        Ok(())
    }
}
