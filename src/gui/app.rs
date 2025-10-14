use eframe::egui;
use std::sync::{Arc, Mutex};
// no cross-thread command channel needed; handle hotkey/SIGUSR1 inline
use crate::utils::logfile::{push_log_and_persist, trim_log_file_startup};
use chrono::Local;
use std::collections::VecDeque;
use std::fs::OpenOptions;

use super::floating::FloatingWindow;
use super::settings::SettingsWindow;
use super::waybar;
use crate::audio::VadStrategy;
use crate::core::{SimpleRecState, WhisperCore};
use crate::hotkey::HotkeyManager;
use crate::i18n;
use crate::llm::LlmPostProcessSettings;
use crate::utils::app_config_dir;
use egui::FontFamily;
use lucide_icons::Icon;
// UNIX-only: import signal handling for SIGUSR1/SIGUSR2
#[cfg(unix)]
use signal_hook::{
    consts::{SIGUSR1, SIGUSR2},
    iterator::Signals,
};

#[derive(PartialEq)]
enum TabView {
    General,
    Devices,
    SpeechModel,
    Dictionary,
    Llm,
    History,
    Logs,
}

pub struct WhisperApp {
    core: Arc<WhisperCore>,
    pub(crate) settings_window: SettingsWindow,
    floating_window: FloatingWindow,
    show_settings: bool,
    show_floating: bool,
    status_message: String,
    debug_logs: Arc<Mutex<VecDeque<String>>>,
    auto_scroll: bool,
    last_waybar_state: Option<SimpleRecState>,
    active_tab: TabView,
    live_settings: Arc<Mutex<LiveSettingsSnapshot>>, // for hotkey/SIGUSR1
    settings_requested: Arc<std::sync::atomic::AtomicBool>,
    // Platform + whether we hid/minimized the main window programmatically
    is_wayland: bool,
    main_hidden_by_app: bool,
    main_minimized_by_app: bool,
    // Keep global hotkey manager alive for app lifetime
    hotkey_manager: Option<HotkeyManager>,
    llm_was_enabled: bool,
}

#[derive(Clone, Debug)]
struct LiveSettingsSnapshot {
    whisper_language: String,
    input_device: Option<String>,
    input_host: Option<String>,
    input_device_index_in_host: Option<usize>,
    input_device_index: Option<usize>,
    output_device: Option<String>,
    input_gain_percent: f32,
    auto_paste: bool,
    whisper_no_timestamps: bool,
    whisper_token_timestamps: bool,
    whisper_use_physical_cores: bool,
    chunk_split_strategy: VadStrategy,
    auto_stop_silence_secs: f32,
    max_record_secs: f32,
    sound_enabled: bool,
    sound_volume_percent: f32,
    llm_postprocess: LlmPostProcessSettings,
}

// File I/O helpers moved to utils::logfile

impl WhisperApp {
    pub fn new(core: Arc<WhisperCore>) -> Self {
        let debug_logs = Arc::new(Mutex::new(VecDeque::with_capacity(1000)));
        let settings_window = SettingsWindow::new();
        // initialize snapshot from current settings
        let s0 = settings_window.get_settings().clone();
        let live_settings = Arc::new(Mutex::new(LiveSettingsSnapshot {
            whisper_language: s0.whisper_language.clone(),
            input_device: s0.input_device.clone(),
            input_host: s0.input_host.clone(),
            input_device_index_in_host: s0.input_device_index_in_host,
            input_device_index: s0.input_device_index,
            output_device: s0.output_device.clone(),
            input_gain_percent: s0.input_gain_percent,
            auto_paste: s0.auto_paste,
            whisper_no_timestamps: s0.whisper_no_timestamps,
            whisper_token_timestamps: s0.whisper_token_timestamps,
            whisper_use_physical_cores: s0.whisper_use_physical_cores,
            chunk_split_strategy: s0.chunk_split_strategy,
            auto_stop_silence_secs: s0.auto_stop_silence_secs,
            max_record_secs: s0.max_record_secs,
            sound_enabled: s0.sound_enabled,
            sound_volume_percent: s0.sound_volume_percent,
            llm_postprocess: s0.llm_postprocess.clone(),
        }));
        crate::utils::sound::set_enabled(s0.sound_enabled);
        crate::utils::sound::set_volume_percent(s0.sound_volume_percent);

        // Prepare debug log file under the app's config directory
        let log_path = app_config_dir().join("debug.log");
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Ensure the file exists without truncating previous contents
        let _ = OpenOptions::new().create(true).append(true).open(&log_path);
        // Trim on startup only: keep the newest 2000 lines for performance
        trim_log_file_startup(&log_path, 2000);

        // Setup log handler for Core
        let logs_for_callback = debug_logs.clone();
        let log_path_for_callback = log_path.clone();
        let log_callback = Arc::new(move |message: &str| {
            let timestamp = Local::now().format("%H:%M:%S%.3f");
            let log_line = format!("[{}] {}", timestamp, message);

            // Update in-memory logs and persist to file
            push_log_and_persist(&logs_for_callback, &log_path_for_callback, &log_line);
        }) as crate::core::LogCallback;

        core.set_log_callback(log_callback.clone());
        // Apply initial dictionary to core
        if !settings_window.dict_entries.is_empty() {
            core.set_dictionary_entries(settings_window.dict_entries.clone());
        }
        core.set_llm_postprocess_settings(s0.llm_postprocess.clone());
        // Settings UI log integration not required

        let settings_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Startup view: restore last state (default is Settings)
        let start_in_floating = s0.last_ui_mode == "floating";

        let mut app = Self {
            core: core.clone(),
            settings_window,
            floating_window: FloatingWindow::new(core.clone()),
            show_settings: !start_in_floating,
            show_floating: start_in_floating,
            status_message: String::from("Ready"),
            debug_logs: debug_logs.clone(),
            auto_scroll: true,
            last_waybar_state: None,
            active_tab: TabView::General,
            live_settings: live_settings.clone(),
            settings_requested: settings_requested.clone(),
            is_wayland: std::env::var("XDG_SESSION_TYPE")
                .map(|v| v == "wayland")
                .unwrap_or(false)
                || std::env::var("WAYLAND_DISPLAY").is_ok(),
            main_hidden_by_app: false,
            main_minimized_by_app: false,
            hotkey_manager: None,
            llm_was_enabled: s0.llm_postprocess.enabled,
        };

        // removed: system tray

        app.add_log(&format!("[Startup] Version: {}", env!("CARGO_PKG_VERSION")));
        app.add_log("[Startup] HootVoice started");
        app.add_log("[Startup] Whisper model initialized");
        // Log current model
        let mp = app.core.get_model_path();
        app.add_log(&format!("[Whisper] Using model: {}", mp.display()));
        let hk = app.settings_window.get_settings().hotkey_recording.clone();
        app.add_log(&format!("[Startup] Hotkey: {}", hk));
        // Advertise signal usage only on UNIX
        #[cfg(unix)]
        app.add_log("[Startup] Signal: pkill -USR1 hootvoice");
        #[cfg(windows)]
        app.add_log("[Startup] Unix signals are not available on Windows");

        // Register global hotkey (skip on Wayland/if disabled)
        let is_wayland = std::env::var("XDG_SESSION_TYPE")
            .map(|v| v == "wayland")
            .unwrap_or(false)
            || std::env::var("WAYLAND_DISPLAY").is_ok();
        let disable_hotkeys =
            std::env::var("HOOTVOICE_DISABLE_HOTKEYS").ok().as_deref() == Some("1");
        if is_wayland || disable_hotkeys {
            app.add_log("[Info] Skipping global hotkey registration (Wayland/disabled)");
        } else {
            match HotkeyManager::new() {
                Ok(mut hotkey_manager) => {
                    let core_for_hotkey = app.core.clone();
                    let live_for_hotkey = live_settings.clone();
                    let initial_hotkey =
                        app.settings_window.get_settings().hotkey_recording.clone();
                    if let Err(e) = hotkey_manager.register_hotkey(&initial_hotkey, move || {
                        // Apply latest settings snapshot before toggling
                        if let Ok(s) = live_for_hotkey.lock() {
                            core_for_hotkey.set_behavior_options(true, s.auto_paste);
                            let lang_opt = if s.whisper_language == "auto" {
                                None
                            } else {
                                Some(s.whisper_language.as_str())
                            };
                            core_for_hotkey.set_language(lang_opt);
                            core_for_hotkey.set_audio_devices(
                                s.input_device.as_deref(),
                                s.output_device.as_deref(),
                            );
                            core_for_hotkey.set_input_device_host_and_index(
                                s.input_host.as_deref(),
                                s.input_device_index_in_host,
                            );
                            core_for_hotkey.set_input_device_index(s.input_device_index);
                            core_for_hotkey
                                .set_input_gain((s.input_gain_percent / 100.0).clamp(0.0, 2.0));
                            crate::utils::sound::set_enabled(s.sound_enabled);
                            crate::utils::sound::set_volume_percent(s.sound_volume_percent);
                            // Whisper最適化設定を反映
                            use crate::transcription::WhisperOptimizationParams;
                            core_for_hotkey.set_whisper_optimization(WhisperOptimizationParams {
                                no_timestamps: s.whisper_no_timestamps,
                                token_timestamps: s.whisper_token_timestamps,
                                use_physical_cores: s.whisper_use_physical_cores,
                                ..Default::default()
                            });
                            core_for_hotkey.set_chunk_split_strategy(s.chunk_split_strategy);
                            core_for_hotkey
                                .set_auto_stop_params(s.auto_stop_silence_secs, s.max_record_secs);
                            core_for_hotkey.set_llm_postprocess_settings(s.llm_postprocess.clone());
                        }
                        core_for_hotkey.toggle_recording();
                    }) {
                        app.add_log(&format!("[Warning] Failed to register hotkey: {}", e));
                    } else {
                        hotkey_manager.spawn_event_thread();
                        app.add_log("[Startup] Registered global hotkey");
                        // Hold manager to keep registration alive
                        // (GlobalHotKeyManager unregisters on drop)
                        // Safe to store; callbacks are in an internal Arc
                        app.hotkey_manager = Some(hotkey_manager);
                    }
                }
                Err(e) => {
                    app.add_log(&format!("[Warning] Failed to initialize hotkey: {}", e));
                }
            }
        }
        // SIGUSR1/SIGUSR2 signal handling (Linux/macOS only)
        #[cfg(unix)]
        {
            let core_for_signal = app.core.clone();
            let live_for_signal = live_settings.clone();
            let settings_flag = settings_requested.clone();
            std::thread::spawn(move || {
                let mut signals = match Signals::new([SIGUSR1, SIGUSR2]) {
                    Ok(sigs) => sigs,
                    Err(_e) => {
                        // Failed to install signal handlers; skip thread
                        return;
                    }
                };
                for sig in signals.forever() {
                    if sig == SIGUSR1 {
                        if let Ok(s) = live_for_signal.lock() {
                            core_for_signal.set_behavior_options(true, s.auto_paste);
                            let lang_opt = if s.whisper_language == "auto" {
                                None
                            } else {
                                Some(s.whisper_language.as_str())
                            };
                            core_for_signal.set_language(lang_opt);
                            core_for_signal.set_audio_devices(
                                s.input_device.as_deref(),
                                s.output_device.as_deref(),
                            );
                            core_for_signal.set_input_device_host_and_index(
                                s.input_host.as_deref(),
                                s.input_device_index_in_host,
                            );
                            core_for_signal.set_input_device_index(s.input_device_index);
                            core_for_signal
                                .set_input_gain((s.input_gain_percent / 100.0).clamp(0.0, 2.0));
                            crate::utils::sound::set_enabled(s.sound_enabled);
                            crate::utils::sound::set_volume_percent(s.sound_volume_percent);
                            // Apply Whisper optimization settings
                            use crate::transcription::WhisperOptimizationParams;
                            core_for_signal.set_whisper_optimization(WhisperOptimizationParams {
                                no_timestamps: s.whisper_no_timestamps,
                                token_timestamps: s.whisper_token_timestamps,
                                use_physical_cores: s.whisper_use_physical_cores,
                                ..Default::default()
                            });
                            core_for_signal.set_chunk_split_strategy(s.chunk_split_strategy);
                            core_for_signal
                                .set_auto_stop_params(s.auto_stop_silence_secs, s.max_record_secs);
                            core_for_signal.set_llm_postprocess_settings(s.llm_postprocess.clone());
                        }
                        core_for_signal.toggle_recording();
                    } else if sig == SIGUSR2 {
                        settings_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            });
        }

        app
    }

    fn apply_live_settings_to_core(&self) {
        let s = self.settings_window.get_settings();
        // Clipboard usage always enabled; only auto-paste toggles
        self.core.set_behavior_options(true, s.auto_paste);
        // 言語
        let lang_opt = if s.whisper_language == "auto" {
            None
        } else {
            Some(s.whisper_language.as_str())
        };
        self.core.set_language(lang_opt);
        // デバイス
        self.core
            .set_audio_devices(s.input_device.as_deref(), s.output_device.as_deref());
        self.core
            .set_input_device_host_and_index(s.input_host.as_deref(), s.input_device_index_in_host);
        self.core.set_input_device_index(s.input_device_index);
        // 入力ゲイン
        self.core
            .set_input_gain((s.input_gain_percent / 100.0).clamp(0.0, 2.0));
        crate::utils::sound::set_enabled(s.sound_enabled);
        crate::utils::sound::set_volume_percent(s.sound_volume_percent);
        // Whisper最適化設定
        use crate::transcription::WhisperOptimizationParams;
        self.core
            .set_whisper_optimization(WhisperOptimizationParams {
                no_timestamps: s.whisper_no_timestamps,
                token_timestamps: s.whisper_token_timestamps,
                use_physical_cores: s.whisper_use_physical_cores,
                ..Default::default()
            });
        // 分割戦略
        self.core.set_chunk_split_strategy(s.chunk_split_strategy);
        // 自動停止
        self.core
            .set_auto_stop_params(s.auto_stop_silence_secs, s.max_record_secs);
        self.core
            .set_llm_postprocess_settings(s.llm_postprocess.clone());
    }

    pub fn show_floating_window(&mut self) {
        self.show_floating = true;
        self.show_settings = false;
        self.settings_window.set_last_ui_mode("floating");
    }

    pub fn show_settings_window(&mut self) {
        self.show_settings = true;
        self.show_floating = false;
        self.settings_window.set_last_ui_mode("settings");
    }

    pub fn add_log(&self, message: &str) {
        let timestamp = Local::now().format("%H:%M:%S%.3f");
        let log_line = format!("[{}] {}", timestamp, message);
        let log_path = app_config_dir().join("debug.log");

        push_log_and_persist(&self.debug_logs, &log_path, &log_line);
    }
}

impl eframe::App for WhisperApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Fully transparent background for floating window
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 0).to_normalized_gamma_f32()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Visibility control of the main window
        // - When floating-only: hide/minimize the main window
        // - When switching back to Settings: only restore if we hid it programmatically
        // Do NOT fight user-initiated minimize; respect the OS minimize button.
        let want_hidden = self.show_floating && !self.show_settings;
        if self.is_wayland {
            // Wayland: toggle Visible; avoid using Minimized.
            if want_hidden {
                if !self.main_hidden_by_app {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    self.main_hidden_by_app = true;
                }
            } else if self.main_hidden_by_app {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                self.main_hidden_by_app = false;
            }
        } else {
            // macOS/Windows/X11: prefer Minimized for hiding; unminimize only if we minimized it.
            if want_hidden {
                if !self.main_minimized_by_app {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    self.main_minimized_by_app = true;
                }
            } else if self.main_minimized_by_app {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                self.main_minimized_by_app = false;
            }
        }
        // Handle external request to show Settings (Linux: SIGUSR2)
        if self
            .settings_requested
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.show_settings = true;
            self.show_floating = false;
        }
        // Each frame, reflect latest UI settings to a snapshot
        {
            let s = self.settings_window.get_settings();
            let llm_enabled_now = s.llm_postprocess.enabled;
            if let Ok(mut snap) = self.live_settings.lock() {
                snap.whisper_language = s.whisper_language.clone();
                snap.input_device = s.input_device.clone();
                snap.input_host = s.input_host.clone();
                snap.input_device_index_in_host = s.input_device_index_in_host;
                snap.input_device_index = s.input_device_index;
                snap.output_device = s.output_device.clone();
                snap.input_gain_percent = s.input_gain_percent;
                snap.auto_paste = s.auto_paste;
                snap.whisper_no_timestamps = s.whisper_no_timestamps;
                snap.whisper_token_timestamps = s.whisper_token_timestamps;
                snap.whisper_use_physical_cores = s.whisper_use_physical_cores;
                snap.chunk_split_strategy = s.chunk_split_strategy;
                snap.auto_stop_silence_secs = s.auto_stop_silence_secs;
                snap.max_record_secs = s.max_record_secs;
                snap.sound_enabled = s.sound_enabled;
                snap.sound_volume_percent = s.sound_volume_percent;
                snap.llm_postprocess = s.llm_postprocess.clone();
            }
            if llm_enabled_now && !self.llm_was_enabled {
                self.add_log("LLM post-processing enabled.");
            }
            self.llm_was_enabled = llm_enabled_now;
        }
        // system tray removed

        // Periodically update even when idle
        let state = self.core.get_state();

        // システムトレイ機能は削除済み

        // Update Waybar custom module status file (on change)
        if self.last_waybar_state != Some(state) {
            waybar::write_status(state);
            self.last_waybar_state = Some(state);
        }

        match state {
            SimpleRecState::Idle => {
                // 待機中は10秒ごとに更新
                ctx.request_repaint_after(std::time::Duration::from_secs(10));
            }
            SimpleRecState::Recording
            | SimpleRecState::Processing
            | SimpleRecState::PostProcessing
            | SimpleRecState::Busy => {
                // アクティブな処理中は1秒ごとに更新
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
            }
        }

        if self.show_settings {
            // Fill CentralPanel with current theme panel color to ensure readability
            // in both light and dark modes (avoid transparent background showing clear_color).
            let panel_fill = ctx.style().visuals.panel_fill;
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::default()
                        .fill(panel_fill)
                        .inner_margin(egui::Margin::symmetric(15, 12)),
                )
                .show(ctx, |ui| {
                    // ボタンのパディングを設定
                    ui.spacing_mut().button_padding = egui::vec2(8.0, 5.0);
                    let strong = ui.visuals().strong_text_color();
                    ui.heading(egui::RichText::new("HootVoice").color(strong));

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(5.0);

                    // ステータス表示
                    ui.horizontal(|ui| {
                        // ステータス表示部分
                        let state = self.core.get_state();

                        // Show status as a badge (use the same Lucide icon as floating)
                        let (status_text, bg_color, text_color, icon_glyph) = match state {
                            SimpleRecState::Idle => (
                                i18n::tr("status-idle"),
                                egui::Color32::from_rgb(40, 167, 69),
                                egui::Color32::WHITE,
                                Icon::Pause.unicode(),
                            ),
                            SimpleRecState::Recording => {
                                ctx.request_repaint_after(std::time::Duration::from_millis(500));
                                (
                                    i18n::tr("status-recording"),
                                    egui::Color32::from_rgb(220, 53, 69),
                                    egui::Color32::WHITE,
                                    Icon::Mic.unicode(),
                                )
                            }
                            SimpleRecState::Processing => {
                                ctx.request_repaint_after(std::time::Duration::from_millis(500));
                                (
                                    i18n::tr("status-processing"),
                                    egui::Color32::from_rgb(255, 193, 7),
                                    egui::Color32::BLACK,
                                    Icon::Loader.unicode(),
                                )
                            }
                            SimpleRecState::PostProcessing => {
                                ctx.request_repaint_after(std::time::Duration::from_millis(500));
                                (
                                    i18n::tr("status-post-processing"),
                                    egui::Color32::from_rgb(75, 154, 242),
                                    egui::Color32::WHITE,
                                    Icon::Loader.unicode(),
                                )
                            }
                            SimpleRecState::Busy => {
                                ctx.request_repaint_after(std::time::Duration::from_millis(500));
                                (
                                    i18n::tr("status-busy"),
                                    egui::Color32::from_rgb(108, 117, 125),
                                    egui::Color32::WHITE,
                                    Icon::Loader.unicode(),
                                )
                            }
                        };

                        // ステータスバッジを表示
                        egui::Frame::default()
                            .inner_margin(egui::Margin::symmetric(10, 6))
                            .corner_radius(egui::CornerRadius::same(4))
                            .fill(bg_color)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(icon_glyph)
                                            .family(FontFamily::Name("lucide".into()))
                                            .color(text_color)
                                            .size(16.0),
                                    );
                                    ui.label(
                                        egui::RichText::new(status_text).color(text_color).strong(),
                                    );
                                });
                            });

                        ui.add_space(20.0);

                        if ui
                            .add_sized(
                                [120.0, 30.0],
                                egui::Button::new(i18n::tr("btn-toggle-recording")),
                            )
                            .clicked()
                        {
                            // 直前の設定を反映してから録音切替
                            self.apply_live_settings_to_core();
                            self.settings_window.stop_input_meter();
                            crate::utils::sound::stop_loop("processing");
                            let new_state = self.core.toggle_recording();
                            self.status_message = match new_state {
                                SimpleRecState::Recording => {
                                    self.add_log("[Record] Recording started");
                                    self.show_floating_window();
                                    i18n::tr("msg-recording-started")
                                }
                                SimpleRecState::Processing => {
                                    self.add_log("[Record] Stopped recording; started processing");
                                    i18n::tr("msg-processing")
                                }
                                SimpleRecState::PostProcessing => {
                                    self.add_log(
                                        "[Record] Stopped recording; running post-processing",
                                    );
                                    i18n::tr("status-post-processing")
                                }
                                SimpleRecState::Idle => {
                                    self.add_log("[Record] Recording stopped");
                                    i18n::tr("msg-recording-stopped")
                                }
                                SimpleRecState::Busy => {
                                    self.add_log("[Warning] Toggle busy");
                                    i18n::tr("msg-busy")
                                }
                            };
                        }

                        if ui
                            .add_sized(
                                [120.0, 30.0],
                                egui::Button::new(i18n::tr("btn-show-floating")),
                            )
                            .clicked()
                        {
                            self.show_floating_window();
                        }
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);

                    // Tab bar (single row, flat): General → Logs
                    let tab_bar_ir = ui.horizontal(|ui| {
                        let tab_size = egui::vec2(96.0, 32.0);
                        let selected_fill = ui.visuals().selection.bg_fill;
                        let unselected_fill = ui.visuals().faint_bg_color;
                        let rounding = egui::CornerRadius {
                            nw: 6,
                            ne: 6,
                            sw: 0,
                            se: 0,
                        };

                        let mut add_tab =
                            |icon: Icon, label: &str, tab: TabView, ui: &mut egui::Ui| {
                                let is_sel = self.active_tab == tab;
                                // Compose icon + label; rely on font fallback for the text
                                let text = format!("{} {}", icon.unicode(), label);
                                let btn = egui::Button::new(egui::RichText::new(text).strong())
                                    .min_size(tab_size)
                                    .fill(if is_sel {
                                        selected_fill
                                    } else {
                                        unselected_fill
                                    })
                                    .corner_radius(rounding);
                                if ui.add(btn).clicked() {
                                    self.active_tab = tab;
                                }
                            };

                        {
                            let label = i18n::tr("tab-general");
                            add_tab(Icon::Keyboard, &label, TabView::General, ui);
                        }
                        ui.add_space(6.0);
                        {
                            let label = i18n::tr("tab-devices");
                            add_tab(Icon::Mic, &label, TabView::Devices, ui);
                        }
                        ui.add_space(6.0);
                        {
                            let label = i18n::tr("tab-speech-model");
                            add_tab(Icon::Brain, &label, TabView::SpeechModel, ui);
                        }
                        ui.add_space(6.0);
                        {
                            let label = i18n::tr("tab-dictionary");
                            add_tab(Icon::Library, &label, TabView::Dictionary, ui);
                        }
                        ui.add_space(6.0);
                        {
                            let label = i18n::tr("tab-llm");
                            add_tab(Icon::Wand, &label, TabView::Llm, ui);
                        }
                        ui.add_space(6.0);
                        {
                            let label = i18n::tr("tab-history");
                            add_tab(Icon::Clock, &label, TabView::History, ui);
                        }
                        ui.add_space(6.0);
                        {
                            let label = i18n::tr("tab-logs");
                            add_tab(Icon::FileText, &label, TabView::Logs, ui);
                        }
                    });

                    // タブバー直下に極薄の仕切り線をウィンドウ横幅いっぱいに描画
                    {
                        let tab_rect = tab_bar_ir.response.rect;
                        let y = tab_rect.bottom();
                        let full = ui.max_rect();
                        let left = full.left();
                        let right = full.right();
                        let painter = ui.painter();
                        let color = ui.visuals().widgets.noninteractive.bg_stroke.color;
                        painter.line_segment(
                            [egui::pos2(left, y), egui::pos2(right, y)],
                            egui::Stroke::new(1.0, color),
                        );
                    }
                    // タブとコンテンツの間に少し余白を入れる
                    ui.add_space(6.0);

                    // タブコンテンツ
                    let available_h = ui.available_height();
                    egui::ScrollArea::vertical()
                        .max_height(available_h)
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            match self.active_tab {
                                TabView::General => {
                                    self.settings_window.ui_section_general(ui);
                                }
                                TabView::Devices => {
                                    self.settings_window.ui_section_devices(ui);
                                }
                                TabView::SpeechModel => {
                                    self.settings_window
                                        .set_current_used_model_path(self.core.get_model_path());
                                    self.settings_window.ui_section_speech_model(ui);
                                }
                                TabView::Dictionary => {
                                    self.settings_window.ui_dictionary_section(ui);
                                }
                                TabView::Llm => {
                                    self.settings_window.ui_section_llm(ui);
                                }
                                TabView::History => {
                                    self.settings_window.ui_section_history(ui);
                                }
                                TabView::Logs => {
                                    // ログビュー
                                    ui.horizontal(|ui| {
                                        ui.heading(i18n::tr("title-debug-log"));
                                        ui.checkbox(
                                            &mut self.auto_scroll,
                                            i18n::tr("label-auto-scroll"),
                                        );
                                        if ui
                                            .add_sized(
                                                [80.0, 28.0],
                                                egui::Button::new(i18n::tr("btn-clear")),
                                            )
                                            .clicked()
                                        {
                                            if let Ok(mut logs) = self.debug_logs.lock() {
                                                logs.clear();
                                            }
                                            self.add_log(&format!(
                                                "[system] {}",
                                                i18n::tr("msg-log-cleared")
                                            ));
                                        }
                                    });

                                    ui.separator();

                                    // ログ表示エリア
                                    let available_height = ui.available_height();
                                    egui::ScrollArea::vertical()
                                        .max_height(available_height)
                                        .auto_shrink([false; 2])
                                        .stick_to_bottom(self.auto_scroll)
                                        .id_salt("debug_console")
                                        .show(ui, |ui| {
                                            ui.style_mut().override_text_style =
                                                Some(egui::TextStyle::Monospace);
                                            // スナップショット方式で読み取り時間を最小化
                                            let snapshot: Option<Vec<String>> = {
                                                if let Ok(logs) = self.debug_logs.lock() {
                                                    Some(logs.iter().cloned().collect())
                                                } else {
                                                    None
                                                }
                                            };
                                            if let Some(lines) = snapshot {
                                                for log in lines.iter() {
                                                    let color = if log.contains("[Error]") {
                                                        egui::Color32::from_rgb(255, 100, 100)
                                                    } else if log.contains("[Warning]") {
                                                        egui::Color32::from_rgb(255, 200, 100)
                                                    } else if log.contains("[Record]") {
                                                        egui::Color32::from_rgb(100, 200, 255)
                                                    } else if log.contains("[Process]") {
                                                        egui::Color32::from_rgb(255, 255, 100)
                                                    } else if log.contains("[Whisper]")
                                                        || log.contains("[llm]")
                                                    {
                                                        egui::Color32::from_rgb(200, 150, 255)
                                                    } else if log.contains("[Startup]")
                                                        || log.contains("[Tray]")
                                                        || log.contains("[Info]")
                                                    {
                                                        egui::Color32::from_rgb(100, 255, 200)
                                                    } else {
                                                        egui::Color32::from_rgb(200, 200, 200)
                                                    };
                                                    ui.colored_label(color, log);
                                                }
                                            }
                                        });
                                    // Shorter refresh interval while recording/processing
                                    ctx.request_repaint_after(std::time::Duration::from_millis(
                                        200,
                                    ));
                                }
                            }
                        });
                });
        }

        // No special log tab switching needed

        // Reflect settings to core (lightweight update each frame) — skip on Logs tab
        if self.active_tab != TabView::Logs {
            let s = self.settings_window.get_settings();
            // Clipboard always enabled; toggle only auto-paste
            self.core.set_behavior_options(true, s.auto_paste);
            let llm_settings_snapshot = s.llm_postprocess.clone();
            // Apply Whisper language (auto: None)
            let lang_opt = if s.whisper_language == "auto" {
                None
            } else {
                Some(s.whisper_language.as_str())
            };
            self.core.set_language(lang_opt);
            // Reflect I/O device settings
            self.core
                .set_audio_devices(s.input_device.as_deref(), s.output_device.as_deref());
            // Apply input gain (0..200% → 0.0..2.0)
            self.core
                .set_input_gain((s.input_gain_percent / 100.0).clamp(0.0, 2.0));
            crate::utils::sound::set_enabled(s.sound_enabled);
            crate::utils::sound::set_volume_percent(s.sound_volume_percent);
            // Reflect Whisper optimization settings
            use crate::transcription::WhisperOptimizationParams;
            self.core
                .set_whisper_optimization(WhisperOptimizationParams {
                    no_timestamps: s.whisper_no_timestamps,
                    token_timestamps: s.whisper_token_timestamps,
                    use_physical_cores: s.whisper_use_physical_cores,
                    ..Default::default()
                });
            self.core.set_chunk_split_strategy(s.chunk_split_strategy);

            // Apply model if requested
            if let Some(new_path) = self.settings_window.take_model_to_apply() {
                match self.core.reload_model(&new_path) {
                    Ok(()) => {
                        self.add_log(&format!(
                            "[Settings] Model switched: {}",
                            new_path.display()
                        ));
                    }
                    Err(e) => {
                        self.add_log(&format!("[Error] Failed to switch model: {}", e));
                    }
                }
            }
            // Apply dictionary if requested
            if let Some(entries) = self.settings_window.take_dictionary_to_apply() {
                self.core.set_dictionary_entries(entries);
            }

            self.core
                .set_llm_postprocess_settings(llm_settings_snapshot);
        }

        // Drain SettingsWindow-generated update logs into Debug Log tab
        let upd_logs = self.settings_window.drain_update_logs();
        for line in upd_logs.iter() {
            self.add_log(line);
        }

        if self.show_floating {
            let open_settings =
                self.floating_window
                    .show(ctx, &mut self.show_floating, &mut self.settings_window);
            // Show settings only when Settings button pressed
            if !self.show_floating && open_settings {
                self.show_settings = true;
                self.settings_window.set_last_ui_mode("settings");
            }
        }
    }
}
