use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::app::WhisperApp;
use super::settings::SettingsWindow;
use super::wizard::{take_open_request, FirstRunWizard};
use crate::core::WhisperCore;
use crate::i18n;
use crate::utils::paths::app_config_dir;

type CoreResult = Arc<WhisperCore>;
type LoadResultSlot = Arc<Mutex<Option<Result<CoreResult, String>>>>;

enum RootState {
    // Load asynchronously so the UI shows quickly even when the model exists
    Loading(LoadingState),
    Setup(SetupState),
    Running(WhisperApp),
}

struct LoadingState {
    expected_abs_model: PathBuf,
    // Background thread result (set to Some when loaded)
    result: LoadResultSlot,
}

struct SetupState {
    settings_window: SettingsWindow,
    auto_prompted: bool,
    expected_abs_model: PathBuf,
    wizard: FirstRunWizard,
    applied_model: Option<PathBuf>,
}

pub struct RootApp {
    state: RootState,
    unmaximize_once: bool,
    // For in-app testing (dev builds), allow opening the wizard anytime
    wizard_in_app: Option<FirstRunWizard>,
    // Run mic permission preflight once at launch (macOS only)
    #[cfg(target_os = "macos")]
    mic_preflight_started: bool,
}

impl RootApp {
    pub fn new() -> Self {
        // Resolve absolute path for the expected model (from default/settings)
        let (expected_abs, _rel) = resolve_expected_model_path();
        if expected_abs.exists() {
            // Even if it exists, model loading is heavy — start with async loading and show UI first
            let result: LoadResultSlot = Arc::new(Mutex::new(None));
            let path = expected_abs.clone();
            let result_clone = result.clone();
            std::thread::spawn(move || {
                let loaded = WhisperCore::new(&path)
                    .map(Arc::new)
                    .map_err(|e| format!("{}", e));
                if let Ok(mut guard) = result_clone.lock() {
                    *guard = Some(loaded);
                }
            });
            let state = LoadingState {
                expected_abs_model: expected_abs,
                result,
            };
            Self {
                state: RootState::Loading(state),
                unmaximize_once: true,
                wizard_in_app: None,
                #[cfg(target_os = "macos")]
                mic_preflight_started: false,
            }
        } else {
            // First run (no model yet): show Settings to prompt download
            let settings_window = SettingsWindow::new();
            let setup = SetupState {
                settings_window,
                auto_prompted: false,
                expected_abs_model: expected_abs,
                wizard: FirstRunWizard::new(),
                applied_model: None,
            };
            Self {
                state: RootState::Setup(setup),
                unmaximize_once: true,
                wizard_in_app: None,
                #[cfg(target_os = "macos")]
                mic_preflight_started: false,
            }
        }
    }
}

impl Default for RootApp {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for RootApp {
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // On startup, ensure not maximized/fullscreen (avoid carry-over on macOS)
        if self.unmaximize_once {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            self.unmaximize_once = false;
        }
        // Prepare next state (avoid simultaneous mutable borrow of self)
        let mut next_state: Option<RootState> = None;

        match &mut self.state {
            RootState::Loading(state) => {
                // Simple loading UI
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(20.0);
                        ui.heading(i18n::tr("loading-whisper-model"));
                        ui.add_space(8.0);
                        ui.label(state.expected_abs_model.to_string_lossy());
                        ui.add_space(8.0);
                        ui.spinner();
                    });
                });

                // Check background result
                let maybe = state.result.lock().ok().and_then(|g| g.clone());
                if let Some(res) = maybe {
                    match res {
                        Ok(core) => {
                            let mut app = WhisperApp::new(core);
                            // On first display, force open Settings (avoid invisible window issues)
                            app.show_settings_window();
                            next_state = Some(RootState::Running(app));
                        }
                        Err(_e) => {
                            // Load failed: fallback to setup
                            let settings_window = SettingsWindow::new();
                            let setup = SetupState {
                                settings_window,
                                auto_prompted: false,
                                expected_abs_model: state.expected_abs_model.clone(),
                                wizard: FirstRunWizard::new(),
                                applied_model: None,
                            };
                            next_state = Some(RootState::Setup(setup));
                        }
                    }
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
            RootState::Running(app) => {
                // macOS: run mic permission preflight on first start
                #[cfg(target_os = "macos")]
                if !self.mic_preflight_started {
                    let s = app.settings_window.get_settings().clone();
                    if s.preflight_mic_on_launch && !s.preflight_mic_done {
                        self.mic_preflight_started = true;
                        let host = s.input_host.clone();
                        let idx = s.input_device_index_in_host;
                        match crate::utils::mic::preflight_mic_access(host.as_deref(), idx) {
                            Ok(()) => {
                                app.settings_window.mark_mic_preflight_done();
                                eprintln!("[MicPreflight] Completed (Running)");
                            }
                            Err(e) => {
                                eprintln!("[MicPreflight] Skipped/failed: {}", e);
                            }
                        }
                    }
                }
                app.update(ctx, _frame);

                // Dev-only request to open wizard
                if take_open_request() && self.wizard_in_app.is_none() {
                    self.wizard_in_app = Some(FirstRunWizard::new());
                }
                // Show wizard overlay if requested
                if let Some(wiz) = &mut self.wizard_in_app {
                    let still_open = wiz.show_modal(ctx, &mut app.settings_window, false, true);
                    if !still_open || wiz.is_finished() {
                        self.wizard_in_app = None;
                    }
                }
            }
            RootState::Setup(setup) => {
                // Early auto-prompt the download confirm on first entry
                if !setup.auto_prompted {
                    setup
                        .settings_window
                        .prompt_download_for_current_selection();
                    setup.auto_prompted = true;
                }

                // macOS: mic permission preflight on first start
                #[cfg(target_os = "macos")]
                if !self.mic_preflight_started {
                    let s = setup.settings_window.get_settings().clone();
                    if s.preflight_mic_on_launch && !s.preflight_mic_done {
                        self.mic_preflight_started = true;
                        let host = s.input_host.clone();
                        let idx = s.input_device_index_in_host;
                        match crate::utils::mic::preflight_mic_access(host.as_deref(), idx) {
                            Ok(()) => {
                                setup.settings_window.mark_mic_preflight_done();
                                eprintln!("[MicPreflight] Completed (Setup)");
                            }
                            Err(e) => {
                                eprintln!("[MicPreflight] Skipped/failed: {}", e);
                            }
                        }
                    }
                }

                // Keep track if user applied a model (after download/change)
                if let Some(new_path) = setup.settings_window.take_model_to_apply() {
                    setup.applied_model = Some(new_path);
                }

                // Draw the wizard modal; require model before passing the Model step
                let model_ready = setup.expected_abs_model.exists()
                    || setup
                        .applied_model
                        .as_ref()
                        .map(|p| p.exists())
                        .unwrap_or(false);
                let still_open =
                    setup
                        .wizard
                        .show_modal(ctx, &mut setup.settings_window, true, model_ready);
                if !still_open || setup.wizard.is_finished() {
                    // Determine final model path to use
                    let model_path_abs = setup
                        .applied_model
                        .clone()
                        .filter(|p| p.exists())
                        .unwrap_or_else(|| {
                            absolute_model_path_for_settings(setup.settings_window.get_settings())
                        });
                    if model_path_abs.exists() {
                        if let Ok(core) = WhisperCore::new(&model_path_abs) {
                            let app = WhisperApp::new(Arc::new(core));
                            next_state = Some(RootState::Running(app));
                        }
                    }
                }

                // Light repaint during setup
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        // After closing the UI, transition state safely
        if let Some(ns) = next_state {
            self.state = ns;
        }
    }
}

fn resolve_expected_model_path() -> (PathBuf, PathBuf) {
    // Load whisper model path from settings.toml (same logic as SettingsWindow)
    let settings_path = app_config_dir().join("settings.toml");
    let settings_str = std::fs::read_to_string(&settings_path).unwrap_or_default();
    let settings: super::settings::Settings = toml::from_str(&settings_str).unwrap_or_default();

    let abs = absolute_model_path_for_settings(&settings);
    (abs, settings.whisper_model_path.clone())
}

fn absolute_model_path_for_settings(settings: &super::settings::Settings) -> PathBuf {
    let p = &settings.whisper_model_path;
    if p.is_absolute() {
        p.clone()
    } else {
        let rel = p.strip_prefix("./").unwrap_or(p);
        app_config_dir().join(rel)
    }
}
