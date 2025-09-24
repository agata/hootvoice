use eframe::egui;
// use lucide icons in tabs; content headings remain plain

use super::SettingsWindow;
use crate::i18n;

impl SettingsWindow {
    pub(super) fn ui_hotkey_section(&mut self, ui: &mut egui::Ui) {
        let strong = ui.visuals().strong_text_color();
        ui.heading(egui::RichText::new(i18n::tr("section-hotkey")).color(strong));
        ui.add_space(5.0);
        egui::Frame::default()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                // Make primary texts use strong color for readability
                let strong = ui.visuals().strong_text_color();
                ui.style_mut().visuals.override_text_color = Some(strong);
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(i18n::tr("label-start-stop-recording"));
                    ui.add_space(10.0);

                    let text_edit = ui
                        .scope(|ui| {
                            let mut visuals = ui.style().visuals.clone();
                            let stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(100));
                            visuals.widgets.inactive.bg_stroke = stroke;
                            visuals.widgets.hovered.bg_stroke = stroke;
                            visuals.widgets.active.bg_stroke = stroke;
                            ui.style_mut().visuals = visuals;

                            ui.add(
                                egui::TextEdit::singleline(&mut self.hotkey_input)
                                    .desired_width(160.0),
                            )
                        })
                        .inner;

                    if text_edit.changed() {
                        self.settings.hotkey_recording = self.hotkey_input.clone();
                        self.check_changes();
                    }
                });
                // Input help (subtle, slightly larger)
                ui.add_space(6.0);
                let help_color = ui.visuals().weak_text_color();
                let base = ui
                    .style()
                    .text_styles
                    .get(&egui::TextStyle::Body)
                    .map(|f| f.size)
                    .unwrap_or(14.0);
                let help_size = (base - 1.0).max(10.0);
                ui.label(
                    egui::RichText::new(i18n::tr("hotkey-help-examples"))
                        .size(help_size)
                        .color(help_color),
                );
                ui.label(
                    egui::RichText::new(i18n::tr("hotkey-help-modifiers"))
                        .size(help_size)
                        .color(help_color),
                );
                ui.label(
                    egui::RichText::new(i18n::tr("hotkey-help-keys"))
                        .size(help_size)
                        .color(help_color),
                );
                ui.label(
                    egui::RichText::new(i18n::tr("hotkey-help-separator"))
                        .size(help_size)
                        .color(help_color),
                );
                // Note: changing hotkey may require app restart (especially on macOS)
                if self.settings.hotkey_recording != self.original_settings.hotkey_recording {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(i18n::tr("hotkey-restart-note"))
                            .size(help_size)
                            .color(help_color),
                    );
                }
            });

        // OS-specific troubleshooting (collapsible)
        ui.add_space(6.0);
        self.ui_hotkey_troubleshoot(ui);
    }
}

impl SettingsWindow {
    // OS-specific troubleshooting for hotkeys (collapsible)
    fn ui_hotkey_troubleshoot(&mut self, ui: &mut egui::Ui) {
        let header = egui::RichText::new(i18n::tr("troubleshoot-hotkey-title")).strong();
        egui::CollapsingHeader::new(header)
            .default_open(false)
            .show(ui, |ui| {
                ui.add_space(6.0);

                #[cfg(target_os = "macos")]
                {
                    ui.label(i18n::tr("hotkey-macos-desc"));
                    ui.add_space(4.0);
                    ui.label(i18n::tr("hotkey-macos-steps"));
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(i18n::tr("btn-open-input-monitoring")).clicked() {
                            let _ = std::process::Command::new("open")
                                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
                                .status()
                                .or_else(|_| std::process::Command::new("open")
                                    .arg("x-apple.systempreferences:com.apple.PreferencePane?Privacy_ListenEvent")
                                    .status());
                        }
                        if ui.button(i18n::tr("btn-go-applications-and-restart")).on_hover_text(i18n::tr("tooltip-go-applications-and-restart")).clicked() {
                            // Just a hint (no action)
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(i18n::tr("hotkey-macos-still-issues"));
                    ui.label(i18n::tr("hotkey-macos-restart"));
                    ui.label(i18n::tr("hotkey-macos-remove-readd"));
                    ui.label(i18n::tr("hotkey-macos-log-check"));
                }

                #[cfg(target_os = "linux")]
                {
                    ui.label(i18n::tr("hotkey-linux-desc"));
                    ui.add_space(4.0);
                    ui.label(i18n::tr("hotkey-linux-x11"));
                    ui.label(i18n::tr("hotkey-linux-wayland"));
                    ui.add_space(6.0);
                    ui.label(i18n::tr("hotkey-linux-workarounds"));
                    ui.label(i18n::tr("hotkey-linux-x11-session"));
                    ui.label(i18n::tr("hotkey-linux-custom-shortcut"));
                    ui.small(i18n::tr("hotkey-linux-note"));

                    ui.add_space(8.0);
                    ui.label(i18n::tr("hotkey-linux-hypr-example"));
                    ui.add_space(4.0);
                    // Hyprland bind example
                    let mut hypr_bind = String::new();
                    hypr_bind.push_str(&format!("# {}\n", i18n::tr("hotkey-hypr-bind-comment")));
                    hypr_bind.push_str("bind = SUPER, Z, exec, ~/.local/bin/hootvoice-toggle.sh\n");
                    ui.add(
                        egui::TextEdit::multiline(&mut hypr_bind)
                            .desired_rows(2)
                            .font(egui::TextStyle::Monospace)
                            .interactive(false)
                            .desired_width(f32::INFINITY)
                    );

                    ui.add_space(6.0);
                    ui.label(i18n::tr("hotkey-hypr-script-title"));
                    let mut hypr_script = String::from("#!/usr/bin/env bash\nset -euo pipefail\n\n");
                    hypr_script.push_str(&format!("# {}\n", i18n::tr("hotkey-hypr-script-comment")));
                    hypr_script.push_str(
                        "if pid=$(pidof hootvoice); then\n  kill -USR1 \"$pid\"\n  exit 0\nfi \n"
                    );
                    ui.add(
                        egui::TextEdit::multiline(&mut hypr_script)
                            .desired_rows(10)
                            .font(egui::TextStyle::Monospace)
                            .interactive(false)
                            .desired_width(f32::INFINITY)
                    );
                }

                #[cfg(target_os = "windows")]
                {
                    ui.label(i18n::tr("hotkey-windows-desc"));
                    ui.add_space(4.0);
                    ui.label(i18n::tr("hotkey-windows-avoid"));
                    ui.label(i18n::tr("hotkey-windows-check"));
                    ui.add_space(6.0);
                    ui.label(i18n::tr("hotkey-windows-try-others"));
                }
            });
    }
}
