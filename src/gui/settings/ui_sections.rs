use super::SettingsWindow;
// Icons are used on tab labels; content headings remain plain
use crate::audio::VadStrategy;
use crate::i18n;
use crate::utils::{app_config_dir, reveal_in_file_manager};
use eframe::egui;
use std::time::{Duration, Instant};

impl SettingsWindow {
    pub(super) fn ui_input_devices_section(&mut self, ui: &mut egui::Ui) {
        // I/O devices
        let strong = ui.visuals().strong_text_color();
        ui.heading(egui::RichText::new(i18n::tr("section-devices")).color(strong));
        ui.add_space(5.0);
        egui::Frame::default()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                let strong = ui.visuals().strong_text_color();
                ui.style_mut().visuals.override_text_color = Some(strong);
                if self.input_devices.is_empty() && self.output_devices.is_empty() {
                    self.refresh_device_lists();
                }
                ui.set_min_width(ui.available_width());
                // Input level meter toggle
                ui.horizontal(|ui| {
                    if self.is_meter_active {
                        if ui.button(i18n::tr("btn-stop-level-meter")).clicked() {
                            self.stop_meter();
                            self.is_meter_active = false;
                        }
                    } else if ui.button(i18n::tr("btn-start-level-meter")).clicked() {
                        self.ensure_meter_for_selected_input();
                        self.is_meter_active = true;
                    }
                });

                // Show level only while meter is active
                if self.is_meter_active {
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                    let level_raw = *self.meter_level.lock().unwrap();
                    let gain = (self.settings.input_gain_percent / 100.0).clamp(0.0, 2.0);
                    let level = (level_raw * gain).clamp(0.0, 1.0);
                    // dBFS display (0 is max). Map -60..0 dB to 0..1 for readability.
                    let db = 20.0 * (level.max(1e-9)).log10();
                    let db_clamped = db.clamp(-60.0, 0.0);
                    let bar = ((db_clamped + 60.0) / 60.0).clamp(0.0, 1.0);
                    ui.horizontal(|ui| {
                        ui.label(i18n::tr("label-input-level"));
                        ui.add(egui::ProgressBar::new(bar).desired_width(220.0));
                        ui.monospace(format!("{:.1} dBFS", db));
                    });
                }
                // Show sensitivity slider only while meter is active
                if self.is_meter_active {
                    ui.add_space(6.0);
                    // Input sensitivity (gain)
                    ui.horizontal(|ui| {
                        ui.label(i18n::tr("label-input-sensitivity"));
                        let slider = ui.add(
                            egui::Slider::new(&mut self.settings.input_gain_percent, 0.0..=200.0)
                                .suffix("%"),
                        );
                        if slider.changed() {
                            self.check_changes();
                        }
                        ui.add_space(8.0);
                        if ui
                            .button(i18n::tr("btn-auto-adjust"))
                            .on_hover_text(i18n::tr("tooltip-auto-adjust"))
                            .clicked()
                        {
                            let target_db = -12.0f32;
                            // Compute dB from current level
                            let level_raw = *self.meter_level.lock().unwrap();
                            let gain = (self.settings.input_gain_percent / 100.0).clamp(0.0, 2.0);
                            let level = (level_raw * gain).clamp(0.0, 1.0);
                            let curr_db = 20.0 * (level.max(1e-9)).log10();
                            if curr_db.is_finite() {
                                let curr_gain =
                                    (self.settings.input_gain_percent / 100.0).clamp(0.01, 2.0);
                                let factor = 10f32.powf((target_db - curr_db) / 20.0);
                                let new_gain = (curr_gain * factor).clamp(0.2, 2.0);
                                self.settings.input_gain_percent = (new_gain * 100.0).round();
                                self.check_changes();
                            }
                        }
                    });
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(i18n::tr("label-mic-input"));
                    let current = if let (Some(ref host), Some(idx)) = (
                        &self.settings.input_host,
                        self.settings.input_device_index_in_host,
                    ) {
                        // Lookup: find matching (host, idx) in input_map and convert to display name
                        if let Some(pos) = self
                            .input_map
                            .iter()
                            .position(|(h, i)| h == host && *i == idx)
                        {
                            self.input_devices
                                .get(pos)
                                .cloned()
                                .unwrap_or_else(|| i18n::tr("option-system-default"))
                        } else {
                            self.settings
                                .input_device
                                .clone()
                                .unwrap_or_else(|| i18n::tr("option-system-default"))
                        }
                    } else {
                        self.settings
                            .input_device
                            .clone()
                            .unwrap_or_else(|| i18n::tr("option-system-default"))
                    };
                    egui::ComboBox::from_id_salt("input_device_combo")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            let mut chosen: Option<(String, Option<usize>, Option<String>)> = None;
                            if ui
                                .selectable_label(
                                    self.settings.input_host.is_none(),
                                    i18n::tr("option-system-default"),
                                )
                                .clicked()
                            {
                                chosen = Some((String::new(), None, None));
                            }
                            for (pos, disp) in self.input_devices.iter().cloned().enumerate() {
                                let (h, i) = &self.input_map[pos];
                                let sel = self.settings.input_host.as_deref() == Some(h.as_str())
                                    && self.settings.input_device_index_in_host == Some(*i);
                                if ui.selectable_label(sel, &disp).clicked() {
                                    // Save: host, per-host idx, name (for stability)
                                    let raw_name = self.input_names.get(pos).cloned();
                                    chosen = Some((h.clone(), Some(*i), raw_name));
                                }
                            }
                            if let Some((h, oi, oname)) = chosen {
                                if let Some(i) = oi {
                                    self.settings.input_host = Some(h);
                                    self.settings.input_device_index_in_host = Some(i);
                                    self.settings.input_device = oname; // temporarily store display name
                                } else {
                                    self.settings.input_host = None;
                                    self.settings.input_device_index_in_host = None;
                                    self.settings.input_device = None;
                                }
                                self.check_changes();
                                self.restart_meter();
                            }
                        });
                    if ui.button(i18n::tr("btn-reload")).clicked() {
                        self.refresh_device_lists();
                    }
                    ui.add_space(6.0);
                    if !self.is_test_recording {
                        if ui
                            .button(i18n::tr("btn-test-recording"))
                            .on_hover_text(i18n::tr("tooltip-test-recording"))
                            .clicked()
                        {
                            if let Err(e) = self.start_test_recording_for_selected_input() {
                                self.save_status_message =
                                    Some(format!("Failed to start test recording: {}", e));
                            } else {
                                self.is_test_recording = true;
                                self.test_started_at = Some(Instant::now());
                            }
                        }
                    } else {
                        if ui
                            .button(i18n::tr("btn-stop-and-save"))
                            .on_hover_text(i18n::tr("tooltip-stop-and-save"))
                            .clicked()
                        {
                            if let Err(e) = self.stop_and_save_test_recording() {
                                self.save_status_message = Some(format!("Save failed: {}", e));
                            } else {
                                self.save_status_message =
                                    Some(i18n::tr("msg-test-recording-saved"));
                            }
                        }
                        if let Some(start) = self.test_started_at {
                            ui.add_space(6.0);
                            let elapsed = start.elapsed();
                            let mm = elapsed.as_secs() / 60;
                            let ss = elapsed.as_secs() % 60;
                            ui.monospace(format!(
                                "{} {:02}:{:02}",
                                i18n::tr("status-recording"),
                                mm,
                                ss
                            ));
                            ui.ctx().request_repaint_after(Duration::from_millis(200));
                        }
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(i18n::tr("label-output-device"));
                    let current = self
                        .settings
                        .output_device
                        .clone()
                        .unwrap_or_else(|| i18n::tr("option-system-default"));
                    egui::ComboBox::from_id_salt("output_device_combo")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            let mut chosen: Option<Option<String>> = None;
                            if ui
                                .selectable_label(
                                    self.settings.output_device.is_none(),
                                    i18n::tr("option-system-default"),
                                )
                                .clicked()
                            {
                                chosen = Some(None);
                            }
                            let list = self.output_devices.clone();
                            for name in list {
                                let sel =
                                    self.settings.output_device.as_deref() == Some(name.as_str());
                                if ui.selectable_label(sel, &name).clicked() {
                                    chosen = Some(Some(name));
                                }
                            }
                            if let Some(val) = chosen {
                                self.settings.output_device = val;
                                self.check_changes();
                            }
                        });
                    if ui.button(i18n::tr("btn-test-play")).clicked() {
                        if let Some(ref name) = self.settings.output_device {
                            crate::utils::sound::set_output_device(Some(name));
                        } else {
                            crate::utils::sound::set_output_device(None);
                        }
                        crate::utils::sound::play_sound_async("sounds/complete.mp3");
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let mut enabled = self.settings.sound_enabled;
                    if ui
                        .checkbox(&mut enabled, i18n::tr("label-play-sounds"))
                        .changed()
                    {
                        self.settings.sound_enabled = enabled;
                        crate::utils::sound::set_enabled(enabled);
                        self.check_changes();
                    }
                    ui.add_enabled_ui(enabled, |ui| {
                        let slider =
                            egui::Slider::new(&mut self.settings.sound_volume_percent, 0.0..=100.0)
                                .text(i18n::tr("label-volume"));
                        if ui.add(slider).changed() {
                            crate::utils::sound::set_volume_percent(
                                self.settings.sound_volume_percent,
                            );
                            self.check_changes();
                        }
                    });
                });
            });
    }

    pub(super) fn ui_speech_model_section(&mut self, ui: &mut egui::Ui) {
        let strong = ui.visuals().strong_text_color();
        ui.heading(egui::RichText::new(i18n::tr("section-speech-model")).color(strong));
        ui.add_space(5.0);
        egui::Frame::default()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                let strong = ui.visuals().strong_text_color();
                ui.style_mut().visuals.override_text_color = Some(strong);
                ui.set_min_width(ui.available_width());
                // 推奨モデルのプリセット選択 + ダウンロード/適用
                ui.horizontal(|ui| {
                    ui.label(i18n::tr("label-preset"));
                    let presets: Vec<String> = super::SUPPORTED_MODELS
                        .iter()
                        .map(|m| i18n::tr(m.label_key))
                        .collect();
                    let mut idx = self.selected_model_index;
                    egui::ComboBox::from_id_salt("preset_model_combo")
                        .selected_text(presets[idx].clone())
                        .show_ui(ui, |ui| {
                            for (i, label) in presets.iter().enumerate() {
                                if ui.selectable_label(i == idx, label).clicked() {
                                    idx = i;
                                }
                            }
                        });
                    if idx != self.selected_model_index {
                        self.selected_model_index = idx;
                    }
                    ui.add_space(10.0);
                    let selected = &super::SUPPORTED_MODELS[self.selected_model_index];
                    // Models are under the OS-standard app config directory
                    let models_dir = app_config_dir().join("models");
                    let target_abs = models_dir.join(selected.filename);
                    let exists = target_abs.exists();
                    let partial = target_abs.with_extension("download").exists();
                    let btn_text = if exists {
                        i18n::tr("btn-change")
                    } else if partial {
                        i18n::tr("btn-resume")
                    } else {
                        i18n::tr("btn-download")
                    };
                    if ui
                        .add_sized([110.0, 28.0], egui::Button::new(btn_text))
                        .clicked()
                    {
                        if exists {
                            // Apply using an absolute path (saving converts to relative later)
                            *self.pending_apply_model.lock().unwrap() = Some(target_abs.clone());
                        } else {
                            self.show_download_confirm = true;
                        }
                    }
                });
                // Show progress just below the Download button
                let downloading_now = *self.downloading.lock().unwrap();
                if downloading_now {
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                    if let Some((done, total)) = *self.download_progress.lock().unwrap() {
                        let denom = if total > 0 { total as f32 } else { 1.0 };
                        let frac = (done as f32 / denom).clamp(0.0, 1.0);
                        ui.add(egui::ProgressBar::new(frac).show_percentage());
                        ui.label(format!(
                            "{:.1} / {:.1} MB",
                            done as f32 / 1_000_000.0,
                            total as f32 / 1_000_000.0
                        ));
                    } else {
                        ui.add(egui::ProgressBar::new(0.0).show_percentage());
                    }
                    if let Some(msg) = self.download_message.lock().unwrap().clone() {
                        ui.colored_label(egui::Color32::LIGHT_GREEN, msg);
                    }
                    ui.add_space(6.0);
                    if ui.button(i18n::tr("btn-cancel")).clicked() {
                        self.download_cancel_flag
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                        if let Ok(mut m) = self.download_message.lock() {
                            *m = Some(i18n::tr("msg-cancelling"));
                        }
                    }
                }
                // After finishing, keep showing the last message close to the button
                if !downloading_now {
                    if let Some(msg) = self.download_message.lock().unwrap().clone() {
                        let lower = msg.to_lowercase();
                        let is_error = lower.contains("error")
                            || lower.contains("failed")
                            || lower.contains("cancelled");
                        let color = if is_error {
                            egui::Color32::YELLOW
                        } else {
                            egui::Color32::LIGHT_GREEN
                        };
                        ui.colored_label(color, msg);
                    }
                }
                // Open the models folder
                ui.add_space(6.0);
                // Show the models folder path in a non-editable text area,
                // and place the button below.
                ui.label(i18n::tr("label-model-folder"));
                let models_dir = app_config_dir().join("models");
                let mut path_text = models_dir.to_string_lossy().to_string();
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut path_text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .interactive(false)
                        .frame(false),
                );
                resp.on_hover_text(path_text);
                ui.add_space(4.0);
                if ui.button(i18n::tr("btn-open-folder")).clicked() {
                    reveal_in_file_manager(&models_dir);
                }
                // Quality/speed indicator for selected model
                self.model_quality_speed_panel(ui);
                // Selection info
                {
                    let info = &super::SUPPORTED_MODELS[self.selected_model_index];
                    let size_mb = (info.size_bytes as f64 / 1_000_000f64).round() as u64;
                    ui.label(format!(
                        "{} {} (~{} MB)",
                        i18n::tr("label-selection"),
                        info.filename,
                        size_mb
                    ));
                }
                // Language setting (Auto + common languages)
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(i18n::tr("label-language"));
                    let display = match self.settings.whisper_language.as_str() {
                        "auto" => i18n::tr("option-auto-detect"),
                        "ja" => i18n::tr("option-japanese-ja"),
                        "en" => i18n::tr("option-english-en"),
                        "zh" => i18n::tr("option-chinese-zh"),
                        "es" => i18n::tr("option-spanish-es"),
                        "fr" => i18n::tr("option-french-fr"),
                        "de" => i18n::tr("option-german-de"),
                        "ko" => i18n::tr("option-korean-ko"),
                        "pt" => i18n::tr("option-portuguese-pt"),
                        "ru" => i18n::tr("option-russian-ru"),
                        "hi" => i18n::tr("option-hindi-hi"),
                        _ => self.settings.whisper_language.clone(),
                    };
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("whisper_lang_combo")
                        .selected_text(display)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "auto",
                                    i18n::tr("option-auto-detect"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "auto".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "ja",
                                    i18n::tr("option-japanese-ja"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "ja".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "en",
                                    i18n::tr("option-english-en"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "en".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "zh",
                                    i18n::tr("option-chinese-zh"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "zh".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "es",
                                    i18n::tr("option-spanish-es"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "es".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "fr",
                                    i18n::tr("option-french-fr"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "fr".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "de",
                                    i18n::tr("option-german-de"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "de".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "ko",
                                    i18n::tr("option-korean-ko"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "ko".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "pt",
                                    i18n::tr("option-portuguese-pt"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "pt".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "ru",
                                    i18n::tr("option-russian-ru"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "ru".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.whisper_language == "hi",
                                    i18n::tr("option-hindi-hi"),
                                )
                                .clicked()
                            {
                                self.settings.whisper_language = "hi".to_string();
                                changed = true;
                            }
                        });
                    if changed {
                        self.check_changes();
                    }
                });
                ui.add_space(4.0);
                ui.label(i18n::tr("msg-language-accuracy"));

                // Download confirmation dialog
                if self.show_download_confirm {
                    egui::Window::new(i18n::tr("title-download-model"))
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ui.ctx(), |ui_win| {
                            let info = &super::SUPPORTED_MODELS[self.selected_model_index];
                            let size_mb = (info.size_bytes as f64 / 1_000_000f64).round() as u64;
                            ui_win.label(i18n::tr("msg-download-whisper"));
                            ui_win.label(format!(
                                "{} {} (~{} MB)",
                                i18n::tr("label-selection"),
                                info.filename,
                                size_mb
                            ));
                            ui_win.label(i18n::tr("msg-download-once"));
                            ui_win.add_space(8.0);
                            ui_win.horizontal(|ui_h| {
                                if ui_h.button(i18n::tr("btn-yes")).clicked() {
                                    self.start_download_current_selection();
                                    self.show_download_confirm = false;
                                }
                                if ui_h.button(i18n::tr("btn-no")).clicked() {
                                    self.show_download_confirm = false;
                                }
                            });
                        });
                }
                // (progress/status moved above, below the button)
                // Show current setting and model in use
                ui.add_space(6.0);
                // Current setting path in a non-editable text area on its own line
                ui.label(i18n::tr("label-current-setting"));
                let mut curr_path = self.settings.whisper_model_path.display().to_string();
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut curr_path)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .interactive(false)
                        .frame(false),
                );
                // Tooltip with full path
                resp.on_hover_text(curr_path);
                if let Some(ref used) = self.current_used_model {
                    let same = used.file_name() == self.settings.whisper_model_path.file_name();
                    if same {
                        ui.colored_label(
                            egui::Color32::GREEN,
                            format!("{} {}", i18n::tr("label-current-used"), used.display()),
                        );
                    } else {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            format!(
                                "{} {} ({})",
                                i18n::tr("label-current-used"),
                                used.display(),
                                i18n::tr("label-current-used-pending")
                            ),
                        );
                    }
                }
                ui.add_space(6.0);
                // Advanced settings in a collapsed section (default closed)
                egui::CollapsingHeader::new(i18n::tr("header-advanced"))
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.add_space(4.0);
                        // Whisper optimization
                        ui.heading(i18n::tr("header-whisper-opt"));
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            let before = (
                                self.settings.whisper_no_timestamps,
                                self.settings.whisper_token_timestamps,
                                self.settings.whisper_use_physical_cores,
                            );
                            ui.checkbox(
                                &mut self.settings.whisper_no_timestamps,
                                i18n::tr("chk-no-timestamps"),
                            );
                            ui.add_space(8.0);
                            ui.checkbox(
                                &mut self.settings.whisper_token_timestamps,
                                i18n::tr("chk-token-timestamps"),
                            );
                            ui.add_space(8.0);
                            ui.checkbox(
                                &mut self.settings.whisper_use_physical_cores,
                                i18n::tr("chk-use-physical-cores"),
                            );
                            let after = (
                                self.settings.whisper_no_timestamps,
                                self.settings.whisper_token_timestamps,
                                self.settings.whisper_use_physical_cores,
                            );
                            if before != after {
                                self.check_changes();
                            }
                        });

                        // Below: VAD (chunk split strategy)
                        ui.add_space(10.0);
                        ui.heading(i18n::tr("header-chunking"));
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            ui.label(i18n::tr("label-split-strategy"));
                            ui.add_space(10.0);
                            let display = match self.settings.chunk_split_strategy {
                                VadStrategy::Normal => i18n::tr("option-normal"),
                                VadStrategy::Aggressive => i18n::tr("option-aggressive"),
                            };
                            let mut changed = false;
                            egui::ComboBox::from_id_salt("chunk_split_strategy_combo")
                                .selected_text(display)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(
                                            self.settings.chunk_split_strategy
                                                == VadStrategy::Normal,
                                            i18n::tr("option-normal"),
                                        )
                                        .clicked()
                                    {
                                        self.settings.chunk_split_strategy = VadStrategy::Normal;
                                        changed = true;
                                    }
                                    if ui
                                        .selectable_label(
                                            self.settings.chunk_split_strategy
                                                == VadStrategy::Aggressive,
                                            i18n::tr("option-aggressive"),
                                        )
                                        .clicked()
                                    {
                                        self.settings.chunk_split_strategy =
                                            VadStrategy::Aggressive;
                                        changed = true;
                                    }
                                });
                            if changed {
                                self.check_changes();
                            }
                        });

                        // Advanced: auto‑stop (silence / max duration)
                        ui.add_space(10.0);
                        ui.heading(i18n::tr("header-auto-stop"));
                        ui.add_space(5.0);
                        // Auto‑stop on silence
                        ui.horizontal(|ui| {
                            ui.label(i18n::tr("label-auto-stop-silence"));
                            let old = self.settings.auto_stop_silence_secs;
                            let slider = ui.add(
                                egui::Slider::new(
                                    &mut self.settings.auto_stop_silence_secs,
                                    0.0..=60.0,
                                )
                                .clamping(egui::SliderClamping::Always)
                                .suffix(" s"),
                            );
                            if slider.changed()
                                && (self.settings.auto_stop_silence_secs - old).abs() > f32::EPSILON
                            {
                                self.check_changes();
                            }
                            if self.settings.auto_stop_silence_secs == 0.0 {
                                ui.label(i18n::tr("label-disabled"));
                            } else {
                                ui.label(i18n::tr("label-auto-stop-silence-tip"));
                            }
                        });
                        // Max recording duration
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(i18n::tr("label-max-recording-time"));
                            let old = self.settings.max_record_secs;
                            let slider = ui.add(
                                egui::Slider::new(&mut self.settings.max_record_secs, 0.0..=3600.0)
                                    .clamping(egui::SliderClamping::Always)
                                    .suffix(" s"),
                            );
                            if slider.changed()
                                && (self.settings.max_record_secs - old).abs() > f32::EPSILON
                            {
                                self.check_changes();
                            }
                            if self.settings.max_record_secs == 0.0 {
                                ui.label(i18n::tr("label-disabled"));
                            }
                        });
                    });
            });
    }

    // removed: ui_appearance_section (unused)

    // removed: correction section
}
