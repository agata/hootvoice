use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
// ProjectDirs and utility imports moved to submodules
use crate::i18n;
use crate::transcription::SUPPORTED_MODELS;
use crate::utils::update::{releases_latest_url, spawn_check_update, AvailableUpdate, UpdateState};
use crate::utils::{open::open_url, reveal_in_file_manager, update};
use std::sync::{Arc, Mutex};
// (kept above) use std::sync::atomic::{AtomicBool, Ordering};
use crate::audio::VadStrategy;
use std::sync::atomic::AtomicBool;
// device trait usage moved to submodules
use std::time::Instant;
// moved audio test helpers into submodule; keep imports local there

// removed: correction feature

// Internal submodules (split by UI sections)
mod audio_test;
mod dictionary_tab;
mod ui_sections;
// mod ui_common; // unused (legacy tab nav)
mod audio_meter;
mod hotkey;
mod whisper_models;
// removed: Ollama support
mod persistence;

// Bundle third-party licenses as Markdown at build time
const THIRD_PARTY_LICENSES_MD: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/THIRD_PARTY_LICENSES.md"
));

// Legacy tab enum removed (unused)

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub hotkey_recording: String,
    pub whisper_model_path: PathBuf,
    pub whisper_language: String,
    // UI language (auto/ja/en)
    pub ui_language: String,
    pub input_device: Option<String>,
    pub input_host: Option<String>,
    pub input_device_index_in_host: Option<usize>,
    pub input_device_index: Option<usize>,
    pub output_device: Option<String>,
    pub input_gain_percent: f32,
    pub auto_paste: bool,
    pub use_clipboard: bool,
    pub floating_opacity: f32,
    pub floating_always_on_top: bool,
    // Last floating window position (screen coords after OS scale)
    pub floating_position: Option<[f32; 2]>,
    pub whisper_no_timestamps: bool,
    pub whisper_token_timestamps: bool,
    pub whisper_use_physical_cores: bool,
    pub chunk_split_strategy: VadStrategy,
    // Auto stop (0 disables)
    pub auto_stop_silence_secs: f32, // 0 disables
    pub max_record_secs: f32,        // 0 disables
    // Last shown UI mode ("settings" | "floating")
    pub last_ui_mode: String,
    // Prompt mic permission shortly after launch (macOS)
    pub preflight_mic_on_launch: bool,
    // Whether mic preflight succeeded
    pub preflight_mic_done: bool,
    // Status sound options
    pub sound_enabled: bool,
    pub sound_volume_percent: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey_recording: "Ctrl+Shift+R".to_string(),
            whisper_model_path: PathBuf::from("models/ggml-large-v3.bin"),
            // Default: auto-detect
            whisper_language: "auto".to_string(),
            // UI language follows OS/env
            ui_language: "auto".to_string(),
            input_device: None,
            input_host: None,
            input_device_index_in_host: None,
            input_device_index: None,
            output_device: None,
            input_gain_percent: 100.0,
            auto_paste: true,
            use_clipboard: true,
            floating_opacity: 1.0,
            floating_always_on_top: true,
            floating_position: None,
            whisper_no_timestamps: true,
            whisper_token_timestamps: false,
            whisper_use_physical_cores: true,
            // Default: aggressive VAD (earlier splits)
            chunk_split_strategy: VadStrategy::Aggressive,
            auto_stop_silence_secs: 10.0,
            max_record_secs: 600.0,
            // Start at Settings by default
            last_ui_mode: "settings".to_string(),
            preflight_mic_on_launch: true,
            preflight_mic_done: false,
            sound_enabled: true,
            sound_volume_percent: 100.0,
        }
    }
}

pub struct SettingsWindow {
    settings: Settings,
    original_settings: Settings, // keep original settings
    hotkey_input: String,
    has_unsaved_changes: bool,
    save_status_message: Option<String>,
    // Whisper model management
    selected_model_index: usize,
    download_progress: Arc<Mutex<Option<(u64, u64)>>>,
    downloading: Arc<Mutex<bool>>, // progress bar while true
    download_message: Arc<Mutex<Option<String>>>,
    pending_apply_model: Arc<Mutex<Option<PathBuf>>>,
    show_download_confirm: bool,
    show_reset_confirm: bool,
    download_cancel_flag: Arc<AtomicBool>,
    current_used_model: Option<PathBuf>,
    // Cached I/O device lists
    // Flattened input devices (display names)
    input_devices: Vec<String>,
    // Flattened -> (host_id, per-host index)
    input_map: Vec<(String, usize)>,
    // Flattened -> raw device name (without host)
    input_names: Vec<String>,
    // Available host names (strings)
    input_hosts: Vec<String>,
    output_devices: Vec<String>,
    // Input level meter
    meter_stream: Option<cpal::Stream>,
    meter_level: Arc<Mutex<f32>>, // 0.0..=1.0
    meter_device_name: Option<String>,
    is_meter_active: bool, // Whether the meter is running
    // Gemma3 fixed policy (no selection index)
    // Test recording
    test_stream: Option<cpal::Stream>,
    test_buffer: Option<Arc<Mutex<Vec<f32>>>>,
    test_sample_rate: u32,
    test_channels: u16,
    is_test_recording: bool,
    test_started_at: Option<Instant>,
    // License dialog
    show_licenses: bool,
    licenses_text: String,
    // Dictionary editing
    pub(crate) dict_entries: Vec<crate::dictionary::DictionaryEntry>,
    pub(crate) dict_dirty: bool,
    pub(crate) pending_apply_dictionary: bool,
    // Dictionary editor dialog state
    pub(crate) dict_editor_open: bool,
    pub(crate) dict_editor_edit_index: Option<usize>,
    pub(crate) dict_editor_canonical: String,
    pub(crate) dict_editor_aliases: Vec<String>,
    pub(crate) dict_editor_includes: Vec<String>,
    // Dictionary list search filter
    pub(crate) dict_filter_text: String,
    // Update check state (GitHub Releases)
    update_state: Arc<Mutex<UpdateState>>,
    update_downloading: Arc<Mutex<bool>>,
    update_progress: Arc<Mutex<Option<(u64, u64)>>>,
    update_message: Arc<Mutex<Option<String>>>,
    update_cancel_flag: Arc<AtomicBool>,
    update_downloaded_path: Arc<Mutex<Option<PathBuf>>>,
    update_logs: Arc<Mutex<Vec<String>>>,
}

impl SettingsWindow {
    pub fn new() -> Self {
        let settings = Self::load_settings().unwrap_or_default();
        let mut this = Self {
            hotkey_input: settings.hotkey_recording.clone(),
            original_settings: settings.clone(),
            settings,
            has_unsaved_changes: false,
            save_status_message: None,
            selected_model_index: 4, // large-v3 (default)
            download_progress: Arc::new(Mutex::new(None)),
            downloading: Arc::new(Mutex::new(false)),
            download_message: Arc::new(Mutex::new(None)),
            pending_apply_model: Arc::new(Mutex::new(None)),
            show_download_confirm: false,
            show_reset_confirm: false,
            download_cancel_flag: Arc::new(AtomicBool::new(false)),
            current_used_model: None,
            input_devices: Vec::new(),
            input_map: Vec::new(),
            input_names: Vec::new(),
            input_hosts: Vec::new(),
            output_devices: Vec::new(),
            meter_stream: None,
            meter_level: Arc::new(Mutex::new(0.0)),
            meter_device_name: None,
            is_meter_active: false,
            // Initialize test recording
            test_stream: None,
            test_buffer: None,
            test_sample_rate: 16_000,
            test_channels: 1,
            is_test_recording: false,
            test_started_at: None,
            show_licenses: false,
            licenses_text: THIRD_PARTY_LICENSES_MD.to_string(),
            dict_entries: Vec::new(),
            dict_dirty: false,
            pending_apply_dictionary: false,
            dict_editor_open: false,
            dict_editor_edit_index: None,
            dict_editor_canonical: String::new(),
            dict_editor_aliases: Vec::new(),
            dict_editor_includes: Vec::new(),
            dict_filter_text: String::new(),
            update_state: Arc::new(Mutex::new(UpdateState::Checking)),
            update_downloading: Arc::new(Mutex::new(false)),
            update_progress: Arc::new(Mutex::new(None)),
            update_message: Arc::new(Mutex::new(None)),
            update_cancel_flag: Arc::new(AtomicBool::new(false)),
            update_downloaded_path: Arc::new(Mutex::new(None)),
            update_logs: Arc::new(Mutex::new(Vec::new())),
        };

        crate::utils::sound::set_enabled(this.settings.sound_enabled);
        crate::utils::sound::set_volume_percent(this.settings.sound_volume_percent);

        // Infer preset from the current model filename
        if let Some(name) = this
            .settings
            .whisper_model_path
            .file_name()
            .and_then(|s| s.to_str())
        {
            if let Some((i, _)) = SUPPORTED_MODELS
                .iter()
                .enumerate()
                .find(|(_, m)| m.filename == name)
            {
                this.selected_model_index = i;
            }
        }

        // no legacy config migration
        // Load dictionary (create default if missing)
        match crate::dictionary::load_or_init_dictionary() {
            Ok(list) => {
                this.dict_entries = list;
                this.dict_dirty = false;
            }
            Err(_) => {
                this.dict_entries = Vec::new();
                this.dict_dirty = false;
            }
        }
        // Initialize UI language (switch Fluent based on the setting)
        crate::i18n::set_ui_language_preference(&this.settings.ui_language);
        // Kick off one-shot update check (background)
        spawn_check_update(this.update_state.clone(), Some(this.update_logs.clone()));
        this
    }

    // Persist flag when mic preflight succeeds
    #[cfg(target_os = "macos")]
    pub fn mark_mic_preflight_done(&mut self) {
        if !self.settings.preflight_mic_done {
            self.settings.preflight_mic_done = true;
            self.save_settings();
            self.original_settings = self.settings.clone();
        }
    }

    // Take “apply dictionary” request from Settings (one-shot)
    pub fn take_dictionary_to_apply(&mut self) -> Option<Vec<crate::dictionary::DictionaryEntry>> {
        if self.pending_apply_dictionary {
            self.pending_apply_dictionary = false;
            Some(self.dict_entries.clone())
        } else {
            None
        }
    }

    // Log callback integration and LLM test feature removed

    // ui_hotkey_section moved to hotkey.rs

    // Download-confirm helper moved to whisper_models.rs

    // Legacy SettingsWindow::show UI (unused)
    #[allow(dead_code)]
    #[cfg(any())]
    pub fn show(&mut self, ui: &mut egui::Ui) {
        // Set button padding
        ui.spacing_mut().button_padding = egui::vec2(8.0, 5.0);

        // New layout: tab-nav + tab-contents (unused)
        // self.ui_tabs_nav(ui);
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);

        // Tab content area is always vertically scrollable
        let max_h = ui.available_height();
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .max_height(max_h)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                match self.active_tab {
                    SettingsTab::General => {
                        self.ui_hotkey_section(ui);
                        // License link at the bottom of General tab
                        self.ui_licenses_link(ui);
                    }
                    SettingsTab::Audio => {
                        self.ui_input_devices_section(ui);
                    }
                    SettingsTab::Model => {
                        self.ui_speech_model_section(ui);
                    }
                    SettingsTab::Behavior => {
                        self.ui_appearance_section(ui);
                    }
                    SettingsTab::Dictionary => {
                        self.ui_dictionary_section(ui);
                    }
                }

                // Footer under tabs removed (auto-save label removed)
            });

        // Periodic repaint for status message
        if self.save_status_message.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(300));
        }

        // End after drawing the new layout
        // License window (if needed)
        if self.show_licenses {
            egui::Window::new("Open Source Licenses")
                .open(&mut self.show_licenses)
                .default_size(egui::vec2(760.0, 560.0))
                .resizable(true)
                .show(ui.ctx(), |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.licenses_text)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Monospace)
                                    .interactive(false),
                            );
                        });
                });
        }
        return;

        #[allow(unreachable_code)]
        {
            // Legacy layout (unused)
            // Scrollable settings area
            egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                // Hotkey settings
                self.ui_hotkey_section(ui);

                ui.add_space(10.0);

                // Speech recognition settings (section)
                ui.heading("Speech Recognition Settings");
                ui.add_space(5.0);

                // Input/Output devices (prioritized)
                ui.heading("Input/Output Devices");
                ui.add_space(5.0);
                egui::Frame::default()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        if self.input_devices.is_empty() && self.output_devices.is_empty() {
                            self.refresh_device_lists();
                        }
                        ui.set_min_width(ui.available_width());
                        // 入力レベル確認ボタン
                        ui.horizontal(|ui| {
                            if self.is_meter_active {
                                if ui.button("Stop Level Meter").clicked() {
                                    self.stop_meter();
                                    self.is_meter_active = false;
                                }
                            } else {
                                if ui.button("Start Level Meter").clicked() {
                                    self.ensure_meter_for_selected_input();
                                    self.is_meter_active = true;
                                }
                            }
                        });

                        // メーターが動作中の場合のみレベル表示
                        if self.is_meter_active {
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                            let level_raw = *self.meter_level.lock().unwrap();
                            let gain = (self.settings.input_gain_percent / 100.0).clamp(0.0, 2.0);
                            let level = (level_raw * gain).clamp(0.0, 1.0);
                            // dBFS display (0 is max). Map -60..0 dB to 0..1 for readability
                            let db = 20.0 * (level.max(1e-9)).log10();
                            let db_clamped = db.max(-60.0).min(0.0);
                            let bar = ((db_clamped + 60.0) / 60.0).clamp(0.0, 1.0);
                            ui.horizontal(|ui| {
                                ui.label("Input Level:");
                                ui.add(egui::ProgressBar::new(bar).desired_width(220.0));
                                ui.monospace(format!("{:.1} dBFS", db));
                            });
                        }
                        // メーターが動作中の場合のみ感度調整を表示
                        if self.is_meter_active {
                            ui.add_space(6.0);
                            // Input sensitivity (gain)
                            ui.horizontal(|ui| {
                                ui.label("Input Sensitivity:");
                                let slider = ui.add(
                                    egui::Slider::new(&mut self.settings.input_gain_percent, 0.0..=200.0)
                                        .suffix("%")
                                );
                                if slider.changed() { self.check_changes(); }
                                ui.add_space(8.0);
                                if ui.button("Auto Adjust").on_hover_text("Set current input level to -12 dBFS").clicked() {
                                    let target_db = -12.0f32;
                                    // Compute dB from current level
                                    let level_raw = *self.meter_level.lock().unwrap();
                                    let gain = (self.settings.input_gain_percent / 100.0).clamp(0.0, 2.0);
                                    let level = (level_raw * gain).clamp(0.0, 1.0);
                                    let curr_db = 20.0 * (level.max(1e-9)).log10();
                                    if curr_db.is_finite() {
                                        let curr_gain = (self.settings.input_gain_percent / 100.0).clamp(0.01, 2.0);
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
                            ui.label("Microphone Input:");
                            let current = if let (Some(ref host), Some(idx)) = (&self.settings.input_host, self.settings.input_device_index_in_host) {
                                // 検索: input_map から一致する (host, idx) を探し表示名に変換
                                if let Some(pos) = self.input_map.iter().position(|(h,i)| h==host && *i==idx) {
                                    self.input_devices.get(pos).cloned().unwrap_or_else(|| "(system default)".to_string())
                                } else {
                                    self.settings.input_device.clone().unwrap_or_else(|| "(system default)".to_string())
                                }
                            } else {
                                self.settings.input_device.clone().unwrap_or_else(|| "(system default)".to_string())
                            };
                            egui::ComboBox::from_id_salt("input_device_combo")
                                .selected_text(current)
                                .show_ui(ui, |ui| {
                                    let mut chosen: Option<(String, Option<usize>, Option<String>)> = None;
                                    if ui.selectable_label(self.settings.input_host.is_none(), "(system default)").clicked() {
                                        chosen = Some((String::new(), None, None));
                                    }
                                    for (pos, disp) in self.input_devices.iter().cloned().enumerate() {
                                        let (h, i) = &self.input_map[pos];
                                        let sel = self.settings.input_host.as_deref() == Some(h.as_str()) && self.settings.input_device_index_in_host == Some(*i);
                                        if ui.selectable_label(sel, &disp).clicked() {
                                            // 保存: host, per-host idx, name（後方互換）
                                            let raw_name = self.input_names.get(pos).cloned();
                                            chosen = Some((h.clone(), Some(*i), raw_name));
                                        }
                                    }
                                    if let Some((h, oi, oname)) = chosen {
                                        if let Some(i) = oi {
                                            self.settings.input_host = Some(h);
                                            self.settings.input_device_index_in_host = Some(i);
                                            self.settings.input_device = oname; // 表示名を暫定保存
                                        } else {
                                            self.settings.input_host = None;
                                            self.settings.input_device_index_in_host = None;
                                            self.settings.input_device = None;
                                        }
                                        self.check_changes();
                                        self.restart_meter();
                                    }
                                });
                            if ui.button("Reload").clicked() { self.refresh_device_lists(); }
                            ui.add_space(6.0);
                            if !self.is_test_recording {
                                if ui.button("Test Recording").on_hover_text("Start recording with the selected input device").clicked() {
                                    if let Err(e) = self.start_test_recording_for_selected_input() {
                                        self.save_status_message = Some(format!("Failed to start test recording: {}", e));
                                    } else {
                                        self.is_test_recording = true;
                                        self.test_started_at = Some(Instant::now());
                                    }
                                }
                            } else {
                                if ui.button("Stop and Save").on_hover_text("Stop recording and choose where to save").clicked() {
                                    if let Err(e) = self.stop_and_save_test_recording() {
                                        self.save_status_message = Some(format!("Save failed: {}", e));
                                    } else {
                                        self.save_status_message = Some("Saved test recording".to_string());
                                    }
                                }
                                if let Some(start) = self.test_started_at {
                                    ui.add_space(6.0);
                                    let elapsed = start.elapsed();
                                    let mm = elapsed.as_secs() / 60;
                                    let ss = elapsed.as_secs() % 60;
                                    ui.monospace(format!("{} {:02}:{:02}", i18n::tr("status-recording"), mm, ss));
                                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                                }
                            }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label("Output Device (effects):");
                            let current = self.settings.output_device.clone().unwrap_or_else(|| "(system default)".to_string());
                            egui::ComboBox::from_id_salt("output_device_combo")
                                .selected_text(current)
                                .show_ui(ui, |ui| {
                                    let mut chosen: Option<Option<String>> = None;
                                    if ui.selectable_label(self.settings.output_device.is_none(), "(system default)").clicked() {
                                        chosen = Some(None);
                                    }
                                    let list = self.output_devices.clone();
                                    for name in list {
                                        let sel = self.settings.output_device.as_deref() == Some(name.as_str());
                                        if ui.selectable_label(sel, &name).clicked() {
                                            chosen = Some(Some(name));
                                        }
                                    }
                                    if let Some(val) = chosen { self.settings.output_device = val; self.check_changes(); }
                                });
                            if ui.button("Test Play").clicked() {
                                if let Some(ref name) = self.settings.output_device { crate::utils::sound::set_output_device(Some(name)); } else { crate::utils::sound::set_output_device(None); }
                                crate::utils::sound::play_sound_async("sounds/complete.mp3");
                            }
                        });
                    });

                ui.add_space(10.0);

                // Model settings
                ui.heading("🤖 Model Settings");
                ui.add_space(5.0);
                egui::Frame::default()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        // 推奨モデルのプリセット選択 + ダウンロード/適用
                        ui.horizontal(|ui| {
                            ui.label("Preset:");
                            let presets: Vec<String> = SUPPORTED_MODELS.iter().map(|m| crate::i18n::tr(m.label_key)).collect();
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
                            if idx != self.selected_model_index { self.selected_model_index = idx; }
                            ui.add_space(10.0);
                            let selected = &SUPPORTED_MODELS[self.selected_model_index];
                            // Models are under the OS-standard app config directory
                            let models_dir = app_config_dir().join("models");
                            let target_abs = models_dir.join(selected.filename);
                            let exists = target_abs.exists();
                            let partial = target_abs.with_extension("download").exists();
                            let btn_text = if exists { "Change" } else if partial { "Resume" } else { "Download" };
                            if ui.add_sized([110.0, 28.0], egui::Button::new(btn_text)).clicked() {
                                if exists {
                                    // 実際の適用は絶対パスを使う（保存は後で相対に変換）
                                    *self.pending_apply_model.lock().unwrap() = Some(target_abs.clone());
                                } else {
                                    self.show_download_confirm = true;
                                }
                            }
                        });
                        // Progress display (moved just below the Download button)
                        if *self.downloading.lock().unwrap() {
                            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                            if let Some((done, total)) = *self.download_progress.lock().unwrap() {
                                let denom = if total > 0 { total as f32 } else { 1.0 };
                                let frac = (done as f32 / denom).clamp(0.0, 1.0);
                                ui.add(egui::ProgressBar::new(frac).show_percentage());
                                ui.label(format!("{:.1} / {:.1} MB", done as f32 / 1_000_000.0, total as f32 / 1_000_000.0));
                            } else {
                                ui.add(egui::ProgressBar::new(0.0).show_percentage());
                            }
                            if let Some(msg) = self.download_message.lock().unwrap().clone() {
                                ui.colored_label(egui::Color32::LIGHT_GREEN, msg);
                            }
                            ui.add_space(6.0);
                            if ui.button("Cancel").clicked() {
                                self.download_cancel_flag.store(true, Ordering::SeqCst);
                                if let Ok(mut m) = self.download_message.lock() { *m = Some("Cancelling...".to_string()); }
                            }
                        }

                        // Open models folder
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label("Model folder:");
                            let models_dir = app_config_dir().join("models");
                            ui.monospace(models_dir.to_string_lossy());
                            if ui.button("Open folder").clicked() {
                                reveal_in_file_manager(&models_dir);
                            }
                        });
                        // Quality/speed indicator for selected model
                        self.model_quality_speed_panel(ui);
                        // Selection info
                        {
                            let info = &SUPPORTED_MODELS[self.selected_model_index];
                            let size_mb = (info.size_bytes as f64 / 1_000_000f64).round() as u64;
                            ui.label(format!("Selected: {} (~{} MB)", info.filename, size_mb));
                        }
                        // Language setting (Auto/ja/en)
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
                                    if ui.selectable_label(self.settings.whisper_language == "auto", i18n::tr("option-auto-detect")).clicked() {
                                        self.settings.whisper_language = "auto".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "ja", i18n::tr("option-japanese-ja")).clicked() {
                                        self.settings.whisper_language = "ja".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "en", i18n::tr("option-english-en")).clicked() {
                                        self.settings.whisper_language = "en".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "zh", i18n::tr("option-chinese-zh")).clicked() {
                                        self.settings.whisper_language = "zh".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "es", i18n::tr("option-spanish-es")).clicked() {
                                        self.settings.whisper_language = "es".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "fr", i18n::tr("option-french-fr")).clicked() {
                                        self.settings.whisper_language = "fr".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "de", i18n::tr("option-german-de")).clicked() {
                                        self.settings.whisper_language = "de".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "ko", i18n::tr("option-korean-ko")).clicked() {
                                        self.settings.whisper_language = "ko".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "pt", i18n::tr("option-portuguese-pt")).clicked() {
                                        self.settings.whisper_language = "pt".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "ru", i18n::tr("option-russian-ru")).clicked() {
                                        self.settings.whisper_language = "ru".to_string();
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.whisper_language == "hi", i18n::tr("option-hindi-hi")).clicked() {
                                        self.settings.whisper_language = "hi".to_string();
                                        changed = true;
                                    }
                                });
                            if changed { self.check_changes(); }
                            // Change detection: reflect has_unsaved only when UI actually changed
                        });
                        // Download confirmation dialog
                        if self.show_download_confirm {
                            egui::Window::new("Download Speech Recognition Data")
                                .collapsible(false)
                                .resizable(false)
                                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                                .show(ui.ctx(), |ui_win| {
                                    let info = &SUPPORTED_MODELS[self.selected_model_index];
                                    let size_mb = (info.size_bytes as f64 / 1_000_000f64).round() as u64;
                                    ui_win.label("Download Whisper model data used to convert speech to text.");
                                    ui_win.label(format!("Selected: {} (~{} MB)", info.filename, size_mb));
                                    ui_win.label("Models are cached; future runs won’t need to download again.");
                                    ui_win.add_space(8.0);
                                    ui_win.horizontal(|ui_h| {
                                        if ui_h.button("Yes").clicked() {
                                            self.start_download_current_selection();
                                            self.show_download_confirm = false;
                                        }
                                        if ui_h.button("No").clicked() {
                                            self.show_download_confirm = false;
                                        }
                                    });
                                });
                        }
                        // (progress moved above, near the Download button)
                        // Show current setting and model in use
                        ui.add_space(6.0);
                        ui.label(format!("Current setting: {}", self.settings.whisper_model_path.display()));
                        if let Some(ref used) = self.current_used_model {
                            let same = used.file_name() == self.settings.whisper_model_path.file_name();
                            if same {
                                ui.colored_label(egui::Color32::GREEN, format!("In use: {}", used.display()));
                            } else {
                                ui.colored_label(egui::Color32::YELLOW, format!("In use: {} (apply on switch)", used.display()));
                            }
                        }

                        ui.add_space(10.0);
                        // Chunk splitting strategy
                        ui.heading("✂️ Chunk Splitting (VAD)");
                        ui.add_space(5.0);
                        ui.horizontal(|ui| {
                            ui.label("Strategy:");
                            ui.add_space(10.0);
                            let display = match self.settings.chunk_split_strategy {
                                VadStrategy::Normal => "Normal".to_string(),
                                VadStrategy::Aggressive => "Aggressive (earlier splits)".to_string(),
                            };
                            let mut changed = false;
                            egui::ComboBox::from_id_salt("chunk_split_strategy_combo")
                                .selected_text(display)
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(self.settings.chunk_split_strategy == VadStrategy::Normal, "Normal").clicked() {
                                        self.settings.chunk_split_strategy = VadStrategy::Normal;
                                        changed = true;
                                    }
                                    if ui.selectable_label(self.settings.chunk_split_strategy == VadStrategy::Aggressive, "Aggressive (earlier splits)").clicked() {
                                        self.settings.chunk_split_strategy = VadStrategy::Aggressive;
                                        changed = true;
                                    }
                                });
                            if changed { self.check_changes(); }
                        });

                    });

                ui.add_space(10.0);

                // Behavior
                ui.heading("Behavior");
                ui.add_space(5.0);
                egui::Frame::default()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        // Auto-paste toggle (copy-only when OFF)
                        if ui.checkbox(&mut self.settings.auto_paste, i18n::tr("label-auto-paste-copy-only")).changed() {
                            self.check_changes();
                        }
                    });

                ui.add_space(10.0);

                // Floating window settings
                ui.heading(i18n::tr("section-floating-window"));
                ui.add_space(5.0);
                egui::Frame::default()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(i18n::tr("label-opacity"));
                            ui.add_space(10.0);

                            let slider = ui.add(
                                egui::Slider::new(&mut self.settings.floating_opacity, 0.1..=1.0)
                                    .show_value(true)
                            );
                            if slider.changed() {
                                self.check_changes();
                            }
                        });
                        ui.add_space(6.0);
                        if ui.checkbox(&mut self.settings.floating_always_on_top, i18n::tr("chk-always-on-top")).changed() {
                            self.check_changes();
                        }
                    });

                ui.add_space(20.0);

                // 校正関連のUIは削除

                // （移動済み）入出力デバイスセクション

                // 手動保存UIは廃止
            });

            // ステータスメッセージを3秒後にクリア（旧レイアウト）
            if self.save_status_message.is_some() {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs(3));
            }
        }
    }

    // 公開: 一般セクション
    pub fn ui_section_general(&mut self, ui: &mut egui::Ui) {
        self.ui_hotkey_section(ui);
        ui.add_space(10.0);
        egui::Frame::default()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                let strong = ui.visuals().strong_text_color();
                ui.style_mut().visuals.override_text_color = Some(strong);
                ui.set_min_width(ui.available_width());
                // Replace emoji heading with Lucide icon
                ui.heading(egui::RichText::new(i18n::tr("heading-settings-usage")).color(strong));
                ui.add_space(6.0);
                // 表示言語（UI）
                ui.horizontal(|ui| {
                    ui.label(i18n::tr("label-ui-language"));
                    let display = match self.settings.ui_language.as_str() {
                        "auto" => i18n::tr("option-auto-os"),
                        "en" => i18n::tr("option-english"),
                        _ => i18n::tr("option-japanese"),
                    };
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("ui_lang_combo")
                        .selected_text(display)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    self.settings.ui_language == "auto",
                                    i18n::tr("option-auto-os"),
                                )
                                .clicked()
                            {
                                self.settings.ui_language = "auto".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.ui_language == "ja",
                                    i18n::tr("option-japanese"),
                                )
                                .clicked()
                            {
                                self.settings.ui_language = "ja".to_string();
                                changed = true;
                            }
                            if ui
                                .selectable_label(
                                    self.settings.ui_language == "en",
                                    i18n::tr("option-english"),
                                )
                                .clicked()
                            {
                                self.settings.ui_language = "en".to_string();
                                changed = true;
                            }
                        });
                    if changed {
                        // 保存＆即時反映（タブ名等が次フレームで切り替わる）
                        crate::i18n::set_ui_language_preference(&self.settings.ui_language);
                        self.check_changes();
                    }
                });
                ui.add_space(6.0);
                if ui
                    .checkbox(&mut self.settings.auto_paste, i18n::tr("label-auto-paste"))
                    .changed()
                {
                    self.check_changes();
                }
            });

        // Auto‑paste troubleshooting (collapsible)
        ui.add_space(6.0);
        self.ui_auto_paste_troubleshoot(ui);

        // Dev-only: On-demand wizard launcher for testing
        if cfg!(debug_assertions) {
            ui.add_space(10.0);
            egui::Frame::default()
                .fill(ui.visuals().extreme_bg_color)
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(i18n::tr("label-dev-tools"));
                        if ui
                            .add_sized(
                                [220.0, 28.0],
                                egui::Button::new(i18n::tr("btn-launch-setup-wizard")),
                            )
                            .clicked()
                        {
                            crate::gui::wizard::request_open_wizard();
                        }
                    });
                });
        }

        // Show “Reset all to defaults” at the end of the General tab
        ui.add_space(10.0);
        egui::Frame::default()
            .fill(ui.visuals().extreme_bg_color)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add_sized(
                            [200.0, 28.0],
                            egui::Button::new(i18n::tr("btn-reset-defaults")),
                        )
                        .clicked()
                    {
                        self.show_reset_confirm = true;
                    }
                    if let Some(msg) = &self.save_status_message {
                        ui.add_space(10.0);
                        ui.colored_label(egui::Color32::GREEN, msg);
                    }
                });

                if self.show_reset_confirm {
                    egui::Window::new(i18n::tr("title-reset-defaults"))
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ui.ctx(), |ui_win| {
                            ui_win.label(i18n::tr("msg-reset-defaults-confirm"));
                            ui_win.add_space(8.0);
                            ui_win.horizontal(|ui_h| {
                                if ui_h.button(i18n::tr("btn-yes")).clicked() {
                                    self.settings = Settings::default();
                                    self.hotkey_input = self.settings.hotkey_recording.clone();
                                    // Update model preset to match defaults
                                    if let Some(name) = self
                                        .settings
                                        .whisper_model_path
                                        .file_name()
                                        .and_then(|s| s.to_str())
                                    {
                                        if let Some((i, _)) = SUPPORTED_MODELS
                                            .iter()
                                            .enumerate()
                                            .find(|(_, m)| m.filename == name)
                                        {
                                            self.selected_model_index = i;
                                        }
                                    }
                                    self.save_settings();
                                    self.original_settings = self.settings.clone();
                                    self.has_unsaved_changes = false;
                                    self.save_status_message =
                                        Some(i18n::tr("msg-applied-defaults"));
                                    self.show_reset_confirm = false;
                                }
                                if ui_h.button(i18n::tr("btn-no")).clicked() {
                                    self.show_reset_confirm = false;
                                }
                            });
                        });
                }
            });

        // App version
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(i18n::tr("label-app-version"));
            ui.monospace(env!("CARGO_PKG_VERSION"));
        });
        // Update availability status (startup-only check)
        ui.add_space(4.0);
        self.ui_update_status(ui);

        // Bottom link in Settings (subtle, hover highlights + pointer cursor)
        ui.add_space(6.0);
        let base_size = ui
            .style()
            .text_styles
            .get(&egui::TextStyle::Body)
            .map(|f| f.size)
            .unwrap_or(14.0);
        let label = egui::Label::new(
            egui::RichText::new(i18n::tr("link-open-source-licenses")).size(base_size + 1.0),
        )
        .sense(egui::Sense::click())
        .selectable(false);
        let (pos, galley, mut resp) = label.layout_in_ui(ui);
        let hovered = resp.hovered();
        let color = if hovered {
            ui.visuals().hyperlink_color
        } else {
            ui.visuals().weak_text_color()
        };
        // Always show underline to make it look like a link
        let underline = egui::Stroke::new(ui.style().interact(&resp).fg_stroke.width, color);
        if ui.is_rect_visible(resp.rect) {
            ui.painter().add(
                egui::epaint::TextShape::new(pos, galley.clone(), color).with_underline(underline),
            );
        }
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp = resp.on_hover_text(i18n::tr("tooltip-open-source-licenses"));
        if resp.clicked() {
            self.show_licenses = true;
        }

        // License window (if needed)
        if self.show_licenses {
            egui::Window::new(i18n::tr("title-open-source-licenses"))
                .open(&mut self.show_licenses)
                .default_size(egui::vec2(760.0, 560.0))
                .resizable(true)
                .show(ui.ctx(), |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.licenses_text)
                                    .desired_width(f32::INFINITY)
                                    .font(egui::TextStyle::Monospace)
                                    .interactive(false),
                            );
                        });
                });
        }
    }

    fn ui_update_status(&mut self, ui: &mut egui::Ui) {
        let state = self.update_state.lock().unwrap().clone();
        match state {
            UpdateState::Checking => {
                ui.small(i18n::tr("update-checking"));
            }
            UpdateState::UpToDate {
                current: _,
                latest: _,
            } => {
                let text = i18n::tr("update-up-to-date");
                ui.label(egui::RichText::new(text).italics());
            }
            UpdateState::Error(msg) => {
                let label = format!("{} ({})", i18n::tr("update-error"), msg);
                ui.colored_label(ui.visuals().weak_text_color(), label);
            }
            UpdateState::Available(AvailableUpdate {
                current: _,
                latest,
                asset_name: _,
                asset_url: _,
                asset_size: _,
            }) => {
                egui::Frame::default()
                    .fill(ui.visuals().extreme_bg_color)
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let green = egui::Color32::from_rgb(80, 200, 120);
                            let txt = egui::RichText::new(format!(
                                "{} v{}",
                                i18n::tr("update-available-label"),
                                latest
                            ))
                            .color(green)
                            .strong();
                            ui.label(txt);
                            if ui.button(i18n::tr("btn-open-releases-page")).clicked() {
                                let url = releases_latest_url();
                                if let Ok(mut lg) = self.update_logs.lock() {
                                    lg.push(format!("[Update] Open releases page: {}", url));
                                }
                                open_url(&url);
                            }
                        });
                        // No in-app downloading; we just open the releases page in browser.
                    });
            }
        }
    }

    fn start_update_download(&self) {
        // Read asset selection from state
        let info = match self.update_state.lock().unwrap().clone() {
            UpdateState::Available(a) => a,
            _ => return,
        };

        *self.update_downloading.lock().unwrap() = true;
        *self.update_progress.lock().unwrap() = Some((0, info.asset_size));
        *self.update_message.lock().unwrap() = None;
        self.update_cancel_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);

        let prog = self.update_progress.clone();
        let downloading = self.update_downloading.clone();
        let msg = self.update_message.clone();
        let cancel = self.update_cancel_flag.clone();
        let dest_slot = self.update_downloaded_path.clone();
        let url = info.asset_url.clone();
        let dest = update::downloads_dir().join(info.asset_name.clone());
        // Log start of download
        if let Ok(mut lg) = self.update_logs.lock() {
            lg.push(format!(
                "[Update] Download start: {} -> {} ({} bytes)",
                url,
                dest.display(),
                info.asset_size
            ));
        }
        let logs = self.update_logs.clone();

        std::thread::spawn(move || {
            let res = crate::transcription::download_with_progress_cancelable(
                &url,
                &dest,
                cancel,
                |done, total| {
                    if let Ok(mut p) = prog.lock() {
                        *p = Some((done, total));
                    }
                },
            );
            match res {
                Ok(()) => {
                    if let Ok(mut m) = msg.lock() {
                        *m = Some(crate::i18n::tr("msg-update-downloaded"));
                    }
                    if let Ok(mut d) = dest_slot.lock() {
                        *d = Some(dest.clone());
                    }
                    if let Ok(mut lg) = logs.lock() {
                        lg.push(format!("[Update] Download complete: {}", dest.display()));
                    }
                }
                Err(e) => {
                    if let Ok(mut m) = msg.lock() {
                        *m = Some(format!(
                            "{}: {}",
                            crate::i18n::tr("msg-update-download-failed"),
                            e
                        ));
                    }
                    if let Ok(mut lg) = logs.lock() {
                        lg.push(format!("[Update] Download failed: {}", e));
                    }
                }
            }
            if let Ok(mut d) = downloading.lock() {
                *d = false;
            }
        });
    }

    // Drain update-related logs so the app can mirror them to the Debug Log tab
    pub fn drain_update_logs(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(mut lg) = self.update_logs.lock() {
            for line in lg.drain(..) {
                out.push(line);
            }
        }
        out
    }

    // Public: hotkey only (wizard)
    pub fn ui_section_hotkey_only(&mut self, ui: &mut egui::Ui) {
        self.ui_hotkey_section(ui);
    }

    // Public: devices section
    pub fn ui_section_devices(&mut self, ui: &mut egui::Ui) {
        self.ui_input_devices_section(ui);
    }
    // Public: speech model (Whisper)
    pub fn ui_section_speech_model(&mut self, ui: &mut egui::Ui) {
        self.ui_speech_model_section(ui);
    }
    // Public: appearance
    #[cfg(any())]
    pub fn ui_section_appearance(&mut self, ui: &mut egui::Ui) {
        self.ui_appearance_section(ui);
    }

    // Footer UI removed (moved to bottom of General tab)

    // ui_tabs_nav moved to ui_common.rs

    // ui_auto_save_footer moved to ui_common.rs

    // removed: LLM/Ollama/ONNX test UI and helpers

    // Section: files for text correction (legacy)
    // ui_correction_files_section moved to ui_common.rs

    // Section: save buttons
    // ui_save_buttons moved to ui_common.rs

    // Egui’s Key→string mapping. Character keys are handled via Text events; focus on special keys here.
    // key_to_string moved to hotkey.rs

    pub fn set_current_used_model_path(&mut self, p: PathBuf) {
        self.current_used_model = Some(p);
    }

    // 入力デバイスとメーター関連は audio_meter.rs へ移動

    // モデル品質・速度パネルは whisper_models.rs へ移動

    // バイアス機能は削除

    pub fn get_settings(&self) -> &Settings {
        &self.settings
    }

    pub fn set_last_ui_mode(&mut self, mode: &str) {
        self.settings.last_ui_mode = mode.to_string();
        // 即時保存（UI操作ではないため check_changes は使わない）
        self.save_settings();
        self.original_settings = self.settings.clone();
    }

    // フローティングウィンドウの位置を保存（即時保存）
    pub fn set_floating_position(&mut self, pos: egui::Pos2) {
        self.settings.floating_position = Some([pos.x, pos.y]);
        self.save_settings();
        self.original_settings = self.settings.clone();
    }

    // 保存済みのフローティング位置を取得
    pub fn get_floating_position(&self) -> Option<egui::Pos2> {
        self.settings
            .floating_position
            .map(|xy| egui::pos2(xy[0], xy[1]))
    }

    // 設定画面からの「モデル適用」要求を取り出す（1回限り）
    // take_model_to_apply は whisper_models.rs へ移動

    fn check_changes(&mut self) {
        let changed = self.settings != self.original_settings;
        if changed {
            // 常時自動保存（元に戻すは廃止）
            self.save_settings();
            self.original_settings = self.settings.clone();
            self.has_unsaved_changes = false;
            self.save_status_message = Some(i18n::tr("msg-settings-saved"));
        } else {
            self.has_unsaved_changes = false;
        }
    }

    // save_settings は persistence.rs へ移動

    // load_settings は persistence.rs へ移動

    // get_config_path は persistence.rs へ移動

    // start_download_current_selection は whisper_models.rs へ移動

    // update_root_config_model_path は persistence.rs へ移動

    // removed: update_root_config_ollama_* references (unused)
}

impl Default for SettingsWindow {
    fn default() -> Self {
        Self::new()
    }
}

// removed: Gemma3/Ollama related notes

// ライセンスリンクの描画（一般タブの末尾）
impl SettingsWindow {
    // 自動ペーストに関するOS別トラブルシュート（折りたたみ）
    fn ui_auto_paste_troubleshoot(&mut self, ui: &mut egui::Ui) {
        let header = egui::RichText::new(i18n::tr("troubleshoot-autopaste-title")).strong();
        egui::CollapsingHeader::new(header)
            .default_open(false)
            .show(ui, |ui| {
                ui.add_space(6.0);

                #[cfg(target_os = "macos")]
                {
                    ui.label(i18n::tr("autopaste-macos-desc"));
                    ui.add_space(4.0);
                    ui.label(i18n::tr("autopaste-macos-acc"));
                    ui.label(i18n::tr("autopaste-macos-auto"));
                    ui.add_space(6.0);
                    ui.label(i18n::tr("autopaste-macos-open-settings"));
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(i18n::tr("btn-open-accessibility")).clicked() {
                            let _ = std::process::Command::new("open")
                                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
                                .status();
                        }
                        if ui.button(i18n::tr("btn-open-automation")).clicked() {
                            let _ = std::process::Command::new("open")
                                .arg("x-apple.systempreferences:com.apple.PreferencePane?Privacy_Automation")
                                .status()
                                .or_else(|_| std::process::Command::new("open")
                                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Automation")
                                    .status());
                        }
                        if ui.button(i18n::tr("btn-go-applications-and-launch")).on_hover_text(i18n::tr("tooltip-go-applications-and-launch")).clicked() {
                            // 使い方のヒント表示のみ（実動作は行わない）
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(i18n::tr("autopaste-macos-still-issues"));
                    ui.label(i18n::tr("autopaste-macos-rehint1"));
                    ui.label(i18n::tr("autopaste-macos-rehint2"));
                    ui.label(i18n::tr("autopaste-macos-rehint3"));
                    ui.add_space(4.0);
                    ui.small(i18n::tr("autopaste-macos-note"));
                }

                #[cfg(target_os = "linux")]
                {
                    ui.label(i18n::tr("autopaste-linux-desc"));
                    ui.add_space(4.0);
                    ui.label(i18n::tr("autopaste-linux-wayland"));
                    ui.label(i18n::tr("autopaste-linux-x11"));
                    ui.add_space(6.0);
                    ui.label(i18n::tr("autopaste-linux-packages"));
                    ui.monospace(i18n::tr("autopaste-linux-debian"));
                    ui.monospace(i18n::tr("autopaste-linux-arch"));
                    ui.add_space(6.0);
                    ui.small(i18n::tr("autopaste-linux-note"));
                }

                #[cfg(target_os = "windows")]
                {
                    ui.label(i18n::tr("autopaste-windows-desc"));
                    ui.add_space(4.0);
                    ui.label(i18n::tr("autopaste-windows-hints1"));
                    ui.label(i18n::tr("autopaste-windows-hints2"));
                    ui.add_space(6.0);
                    ui.small(i18n::tr("autopaste-windows-note"));
                }
            });
    }

    // removed: legacy ui_licenses_link (inline link is implemented in ui_section_general)
}
