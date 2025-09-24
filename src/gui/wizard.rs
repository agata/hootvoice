use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};

use super::settings::SettingsWindow;
use crate::i18n;

// Global flag to request opening the wizard from anywhere (dev-only button, etc.)
static REQUEST_OPEN: AtomicBool = AtomicBool::new(false);

pub fn request_open_wizard() {
    REQUEST_OPEN.store(true, Ordering::SeqCst);
}

pub fn take_open_request() -> bool {
    REQUEST_OPEN.swap(false, Ordering::SeqCst)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WizardStep {
    Welcome,
    Hotkey,
    Model,
    Devices,
    Tips,
}

pub struct FirstRunWizard {
    step: WizardStep,
    finished: bool,
    open: bool,
    app_icon: Option<egui::TextureHandle>,
}

impl FirstRunWizard {
    pub fn new() -> Self {
        Self {
            step: WizardStep::Welcome,
            finished: false,
            open: true,
            app_icon: None,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    // Show as a centered modal-like window. Returns whether it is still open.
    // If `require_model_ready` is true, the Next button on the Model step is disabled until `model_ready` is true.
    pub fn show_modal(
        &mut self,
        ctx: &egui::Context,
        settings: &mut SettingsWindow,
        require_model_ready: bool,
        model_ready: bool,
    ) -> bool {
        if !self.open {
            return false;
        }
        // Enlarge the window title bar for this wizard only by slightly increasing
        // the window frame's vertical inner margin.
        // This makes the title area (e.g. "初回セットアップ") a bit taller while keeping balance.
        let custom_frame = egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(10));
        let layer_id = egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("first_run_wizard_backdrop"),
        );
        let screen = ctx.input(|i| i.screen_rect);
        let painter = ctx.layer_painter(layer_id);
        let tint = egui::Color32::from_rgba_unmultiplied(0, 0, 0, 80);
        painter.rect_filled(screen, 0.0, tint);

        let mut open = self.open;
        // Decide window height dynamically from screen height with sensible min/max.
        let screen_h = screen.height();
        // Default height: 75% screen, clamped between 460 and 560
        let default_h = (screen_h * 0.75).clamp(460.0, 560.0);
        // Max height: at most 85% screen, and not above 600
        let max_h = (screen_h * 0.85).min(600.0);
        egui::Window::new(i18n::tr("wizard-title"))
            .resizable(false)
            .collapsible(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .frame(custom_frame)
            // Auto-fit height with caps; width stays comfortable for content
            .default_size(egui::vec2(760.0, default_h))
            .min_height(420.0)
            .max_height(max_h)
            .show(ctx, |ui| {
                // Outer padding to keep content away from window edges
                egui::Frame::default()
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_min_width(580.0);

                        // Header (steps) with padding
                        egui::Frame::default()
                            .fill(ui.visuals().extreme_bg_color)
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                self.header_progress(ui);
                            });
                        ui.add_space(8.0);

                        // Ensure bottom navigation remains visible by reserving space for footer frame
                        let nav_reserved = 64.0;
                        let content_max_h = (ui.available_height() - nav_reserved).max(120.0);

                        // Main content with padding
                        egui::Frame::default()
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(14, 10))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false; 2])
                                    .max_height(content_max_h)
                                    .show(ui, |ui| match self.step {
                                        WizardStep::Welcome => self.page_welcome(ui),
                                        WizardStep::Hotkey => self.page_hotkey(ui, settings),
                                        WizardStep::Model => self.page_model(ui, settings),
                                        WizardStep::Devices => self.page_devices(ui, settings),
                                        WizardStep::Tips => self.page_tips(ui),
                                    });
                            });

                        ui.add_space(8.0);

                        // Footer buttons with padding
                        egui::Frame::default()
                            .fill(ui.visuals().extreme_bg_color)
                            .corner_radius(egui::CornerRadius::same(6))
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // Back
                                    let can_back = self.step != WizardStep::Welcome;
                                    let back_btn = ui.add_enabled(
                                        can_back,
                                        egui::Button::new(i18n::tr("btn-back"))
                                            .min_size(egui::vec2(96.0, 28.0)),
                                    );
                                    if back_btn.clicked() {
                                        self.prev_step();
                                    }
                                    ui.add_space(8.0);

                                    // Spacer
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            // Finish / Next
                                            match self.step {
                                                WizardStep::Tips => {
                                                    if ui
                                                        .add_sized(
                                                            [140.0, 28.0],
                                                            egui::Button::new(i18n::tr(
                                                                "btn-finish-start",
                                                            )),
                                                        )
                                                        .clicked()
                                                    {
                                                        self.finished = true;
                                                    }
                                                }
                                                WizardStep::Model => {
                                                    let can_next =
                                                        !require_model_ready || model_ready;
                                                    let next_btn = ui.add_enabled(
                                                        can_next,
                                                        egui::Button::new(i18n::tr("btn-next"))
                                                            .min_size(egui::vec2(96.0, 28.0)),
                                                    );
                                                    if next_btn.clicked() {
                                                        self.next_step();
                                                    }
                                                }
                                                _ => {
                                                    if ui
                                                        .add(
                                                            egui::Button::new(i18n::tr("btn-next"))
                                                                .min_size(egui::vec2(96.0, 28.0)),
                                                        )
                                                        .clicked()
                                                    {
                                                        self.next_step();
                                                    }
                                                }
                                            }
                                        },
                                    );
                                });
                            });
                    });
            });

        self.open = open;
        self.open && !self.finished
    }

    fn header_progress(&self, ui: &mut egui::Ui) {
        // Display order: Welcome → Hotkey → Devices → Model → Tips
        let steps = [
            (WizardStep::Welcome, i18n::tr("wiz-step-welcome")),
            (WizardStep::Hotkey, i18n::tr("wiz-step-hotkey")),
            (WizardStep::Devices, i18n::tr("wiz-step-devices")),
            (WizardStep::Model, i18n::tr("wiz-step-model")),
            (WizardStep::Tips, i18n::tr("wiz-step-tips")),
        ];
        ui.horizontal(|ui| {
            for (i, (s, label)) in steps.iter().enumerate() {
                let idx = i + 1;
                let is_active = *s == self.step;
                let fill = if is_active {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().faint_bg_color
                };
                let text = egui::RichText::new(format!("{}", idx)).strong();
                // Make the step badge circular by setting equal size and full rounding
                let circle = egui::Button::new(text)
                    .min_size(egui::vec2(24.0, 24.0))
                    .corner_radius(egui::CornerRadius::same(12))
                    .fill(fill);
                let resp = ui.add(circle);
                // Active step: strong color, others: weak
                let color = if is_active {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.colored_label(color, label.as_str());
                if i < steps.len() - 1 {
                    ui.add_space(6.0);
                    ui.label("→");
                    ui.add_space(6.0);
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
        });
    }

    fn page_welcome(&mut self, ui: &mut egui::Ui) {
        // Try to load the embedded app icon once and upload it as a texture
        if self.app_icon.is_none() {
            const APP_ICON_BYTES: &[u8] = include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/packaging/icons/hootvoice.png"
            ));
            if let Ok(img) = image::load_from_memory(APP_ICON_BYTES) {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let color =
                    egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                let tex = ui.ctx().load_texture(
                    "app_icon_hootvoice",
                    color,
                    egui::TextureOptions::LINEAR,
                );
                self.app_icon = Some(tex);
            }
        }

        if let Some(tex) = &self.app_icon {
            // Centered icon at the top of the first step
            ui.vertical_centered(|ui| {
                let size = egui::vec2(96.0, 96.0);
                ui.add(egui::Image::new((tex.id(), size)));
            });
            ui.add_space(8.0);
        }
        ui.heading(
            egui::RichText::new(i18n::tr("welcome-title")).color(ui.visuals().strong_text_color()),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(i18n::tr("welcome-desc")).color(ui.visuals().strong_text_color()),
        );
        ui.add_space(8.0);
        bullet(ui, &i18n::tr("welcome-bullet-hotkey"));
        bullet(ui, &i18n::tr("welcome-bullet-model"));
        bullet(ui, &i18n::tr("welcome-bullet-devices"));
        bullet(ui, &i18n::tr("welcome-bullet-tips"));
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(i18n::tr("welcome-next-hint"))
                .color(ui.visuals().strong_text_color()),
        );
    }

    fn page_hotkey(&mut self, ui: &mut egui::Ui, settings: &mut SettingsWindow) {
        ui.heading(
            egui::RichText::new(i18n::tr("wizard-hotkey-title"))
                .color(ui.visuals().strong_text_color()),
        );
        ui.add_space(6.0);
        settings.ui_section_hotkey_only(ui);
    }

    fn page_model(&mut self, ui: &mut egui::Ui, settings: &mut SettingsWindow) {
        ui.heading(
            egui::RichText::new(i18n::tr("wizard-model-title"))
                .color(ui.visuals().strong_text_color()),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(i18n::tr("wizard-model-desc"))
                .color(ui.visuals().strong_text_color()),
        );
        ui.add_space(6.0);
        settings.ui_section_speech_model(ui);
    }

    fn page_devices(&mut self, ui: &mut egui::Ui, settings: &mut SettingsWindow) {
        ui.heading(
            egui::RichText::new(i18n::tr("wizard-devices-title"))
                .color(ui.visuals().strong_text_color()),
        );
        ui.add_space(6.0);
        settings.ui_section_devices(ui);
    }

    fn page_tips(&mut self, ui: &mut egui::Ui) {
        ui.heading(
            egui::RichText::new(i18n::tr("wizard-tips-title"))
                .color(ui.visuals().strong_text_color()),
        );
        ui.add_space(6.0);
        bullet(ui, &i18n::tr("wizard-tip1"));
        bullet(ui, &i18n::tr("wizard-tip2"));
        bullet(ui, &i18n::tr("wizard-tip3"));
        ui.add_space(6.0);
        ui.small(i18n::tr("wizard-help"));
    }

    fn next_step(&mut self) {
        self.step = match self.step {
            WizardStep::Welcome => WizardStep::Hotkey,
            WizardStep::Hotkey => WizardStep::Devices,
            WizardStep::Devices => WizardStep::Model,
            WizardStep::Model => WizardStep::Tips,
            WizardStep::Tips => WizardStep::Tips,
        };
    }

    fn prev_step(&mut self) {
        self.step = match self.step {
            WizardStep::Welcome => WizardStep::Welcome,
            WizardStep::Hotkey => WizardStep::Welcome,
            WizardStep::Devices => WizardStep::Hotkey,
            WizardStep::Model => WizardStep::Devices,
            WizardStep::Tips => WizardStep::Model,
        };
    }
}

impl Default for FirstRunWizard {
    fn default() -> Self {
        Self::new()
    }
}

fn bullet(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.label("•");
        ui.label(egui::RichText::new(text).color(ui.visuals().strong_text_color()));
    });
}
