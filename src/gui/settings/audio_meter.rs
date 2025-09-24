// no extra sync imports needed here

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::SettingsWindow;

impl SettingsWindow {
    pub(super) fn refresh_device_lists(&mut self) {
        use cpal::traits::{DeviceTrait, HostTrait};
        self.input_devices.clear();
        self.input_map.clear();
        self.input_names.clear();
        self.input_hosts.clear();
        // Collect available hosts
        let mut hosts: Vec<String> = Vec::new();
        for host_id in cpal::available_hosts() {
            hosts.push(format!("{:?}", host_id).to_lowercase());
        }
        self.input_hosts = hosts.clone();
        println!("[DEV-LIST] available_hosts = {:?}", self.input_hosts);
        // Flatten devices across hosts
        for host_id in cpal::available_hosts() {
            if let Ok(host) = cpal::host_from_id(host_id) {
                if let Ok(devs) = host.input_devices() {
                    for (i, d) in devs.enumerate() {
                        let host_name = format!("{:?}", host_id).to_lowercase();

                        // Quick capability check: can we get a default input config?
                        let can_record = d.default_input_config().is_ok();
                        if !can_record {
                            if let Ok(n) = d.name() {
                                println!("[DEV-LIST] skip (no default input config): host={} idx={} name={}", host_name, i, n);
                            } else {
                                println!("[DEV-LIST] skip (no default input config): host={} idx={} name=(unknown)", host_name, i);
                            }
                            continue;
                        }

                        // Decide display name (avoid unused_mut lint on non-Linux)
                        let base_name = d.name().unwrap_or_else(|_| "(unknown)".to_string());
                        let name = {
                            #[cfg(target_os = "linux")]
                            {
                                if host_name == "pipewire" {
                                    println!(
                                        "[DEV-LIST] cpal pipewire device raw: host={} idx={} name={}",
                                        host_name, i, base_name
                                    );
                                    if let Some(pretty) = enrich_with_pipewire_name(&base_name) {
                                        println!("[DEV-LIST] enrich pipewire -> {}", pretty);
                                        pretty
                                    } else {
                                        base_name.clone()
                                    }
                                } else {
                                    base_name.clone()
                                }
                            }
                            #[cfg(not(target_os = "linux"))]
                            {
                                base_name.clone()
                            }
                        };

                        self.input_devices.push(format!("{}: {}", host_name, name));
                        self.input_map.push((host_name.clone(), i));
                        self.input_names.push(name);
                        println!(
                            "[DEV-LIST] added: host={} idx={} name={}",
                            host_name,
                            i,
                            self.input_names.last().unwrap()
                        );
                    }
                }
            }
        }
        if self.input_devices.is_empty() {
            self.input_devices.push("(system default)".into());
            self.input_map.push(("default".into(), 0));
        }
        // Output devices (default host only)
        self.output_devices.clear();
        let host_out = cpal::default_host();
        if let Ok(mut devs) = host_out.output_devices() {
            for d in devs.by_ref() {
                if let Ok(name) = d.name() {
                    self.output_devices.push(name);
                }
            }
        }
        self.output_devices.sort();
        self.output_devices.dedup();
    }

    pub(super) fn restart_meter(&mut self) {
        if self.is_meter_active {
            self.stop_meter();
            self.ensure_meter_for_selected_input();
        }
    }

    pub(super) fn stop_meter(&mut self) {
        if let Some(s) = self.meter_stream.take() {
            // Explicitly pause to release mic immediately (macOS privacy indicator)
            let _ = s.pause();
        }
        self.meter_device_name = None;
        *self.meter_level.lock().unwrap() = 0.0;
    }

    // Allow stopping the meter from outside the UI safely
    pub fn stop_input_meter(&mut self) {
        if self.is_meter_active {
            self.stop_meter();
            self.is_meter_active = false;
        }
    }

    pub(super) fn ensure_meter_for_selected_input(&mut self) {
        // Select by host+index when possible
        if let (Some(ref host), Some(idx)) = (
            &self.settings.input_host,
            self.settings.input_device_index_in_host,
        ) {
            let key = Some(format!("{}#{}", host, idx));
            if self.meter_stream.is_some() && self.meter_device_name == key {
                return;
            }
            let host_str = host.clone();
            let _ = self.start_input_monitor_host_index(Some(host_str.as_str()), Some(idx));
        } else {
            let key = self.settings.input_device.clone();
            if self.meter_stream.is_some() && self.meter_device_name == key {
                return;
            }
            let _ = self.start_input_monitor(None);
        }
    }

    fn start_input_monitor(&mut self, name: Option<&str>) -> Result<(), String> {
        let host = cpal::default_host();
        let device = if let Some(n) = name {
            match host.input_devices() {
                Ok(devs) => {
                    let mut found = None;
                    for d in devs {
                        if let Ok(dn) = d.name() {
                            if dn == n {
                                found = Some(d);
                                break;
                            }
                        }
                    }
                    found.or_else(|| host.default_input_device())
                }
                Err(_) => host.default_input_device(),
            }
        } else {
            host.default_input_device()
        };
        let device = device.ok_or_else(|| "Input device not found".to_string())?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let sample_format = supported.sample_format();
        let config: cpal::StreamConfig = supported.config();
        let level = self.meter_level.clone();
        let stream = match sample_format {
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    let mut sum = 0.0f32;
                    for &s in data {
                        let f = s as f32 / 32768.0;
                        sum += f * f;
                    }
                    let rms = (sum / (data.len().max(1) as f32)).sqrt();
                    if let Ok(mut l) = level.lock() {
                        *l = 0.85 * (*l) + 0.15 * rms;
                    }
                },
                move |_e| {},
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    let mut sum = 0.0f32;
                    for &s in data {
                        let f = (s as f32 - 32768.0) / 32768.0;
                        sum += f * f;
                    }
                    let rms = (sum / (data.len().max(1) as f32)).sqrt();
                    if let Ok(mut l) = level.lock() {
                        *l = 0.85 * (*l) + 0.15 * rms;
                    }
                },
                move |_e| {},
                None,
            ),
            _ => device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    let mut sum = 0.0f32;
                    for &f in data {
                        sum += f * f;
                    }
                    let rms = (sum / (data.len().max(1) as f32)).sqrt();
                    if let Ok(mut l) = level.lock() {
                        *l = 0.85 * (*l) + 0.15 * rms;
                    }
                },
                move |_e| {},
                None,
            ),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        self.meter_device_name = name.map(|s| s.to_string());
        self.meter_stream = Some(stream);
        Ok(())
    }

    fn start_input_monitor_host_index(
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
        let level = self.meter_level.clone();
        let stream = match sample_format {
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    let mut sum = 0.0f32;
                    for &s in data {
                        let f = s as f32 / 32768.0;
                        sum += f * f;
                    }
                    let rms = (sum / (data.len().max(1) as f32)).sqrt();
                    if let Ok(mut l) = level.lock() {
                        *l = 0.85 * (*l) + 0.15 * rms;
                    }
                },
                move |_e| {},
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    let mut sum = 0.0f32;
                    for &s in data {
                        let f = (s as f32 - 32768.0) / 32768.0;
                        sum += f * f;
                    }
                    let rms = (sum / (data.len().max(1) as f32)).sqrt();
                    if let Ok(mut l) = level.lock() {
                        *l = 0.85 * (*l) + 0.15 * rms;
                    }
                },
                move |_e| {},
                None,
            ),
            _ => device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    let mut sum = 0.0f32;
                    for &f in data {
                        sum += f * f;
                    }
                    let rms = (sum / (data.len().max(1) as f32)).sqrt();
                    if let Ok(mut l) = level.lock() {
                        *l = 0.85 * (*l) + 0.15 * rms;
                    }
                },
                move |_e| {},
                None,
            ),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        self.meter_device_name = Some(format!(
            "{}#{}",
            host.unwrap_or("default"),
            idx.unwrap_or(0)
        ));
        self.meter_stream = Some(stream);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn enrich_with_pipewire_name(cpal_name: &str) -> Option<String> {
    use unicode_segmentation::UnicodeSegmentation;
    // Collect PipeWire source names
    let sources = SettingsWindow::enrich_with_pipewire_sources();
    let c = cpal_name.to_lowercase();
    // Try exact or substring match
    for (_id, nm) in &sources {
        let nml = nm.to_lowercase();
        if c.contains(&nml) || nml.contains(&c) {
            return Some(nm.to_string());
        }
    }
    // Try by grouping alphanumeric blocks
    let tokens: Vec<String> = c
        .graphemes(true)
        .collect::<Vec<&str>>()
        .join("")
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    if !tokens.is_empty() {
        for (_id, nm) in &sources {
            let nml = nm.to_lowercase();
            let mut score = 0;
            for t in &tokens {
                if nml.contains(t) {
                    score += 1;
                }
            }
            if score >= 2 {
                return Some(nm.clone());
            }
        }
    }
    None
}

impl SettingsWindow {
    #[cfg(target_os = "linux")]
    pub(super) fn enrich_with_pipewire_sources() -> Vec<(u32, String)> {
        let mut res = Vec::new();
        let list = crate::utils::list_pw_sources();
        for s in list {
            res.push((s.id, s.name));
        }
        res
    }
}
