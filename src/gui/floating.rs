use eframe::egui;
use std::sync::Arc;

use crate::core::{SimpleRecState, WhisperCore};
// removed unused icon-loading paths
use egui::FontFamily;
use lucide_icons::Icon;

pub struct FloatingWindow {
    core: Arc<WhisperCore>,
    size: egui::Vec2,
    #[cfg(all(target_os = "linux", feature = "wayland_layer"))]
    is_wayland: bool,
    #[cfg(all(target_os = "linux", feature = "wayland_layer"))]
    sidecar: Option<std::process::Child>,
    #[cfg(all(target_os = "linux", feature = "wayland_layer"))]
    sidecar_failed: bool,
}

impl FloatingWindow {
    pub fn new(core: Arc<WhisperCore>) -> Self {
        Self {
            core,
            size: egui::Vec2::new(280.0, 96.0),
            #[cfg(all(target_os = "linux", feature = "wayland_layer"))]
            is_wayland: std::env::var("XDG_SESSION_TYPE")
                .map(|v| v == "wayland")
                .unwrap_or(false)
                || std::env::var("WAYLAND_DISPLAY").is_ok(),
            #[cfg(all(target_os = "linux", feature = "wayland_layer"))]
            sidecar: None,
            #[cfg(all(target_os = "linux", feature = "wayland_layer"))]
            sidecar_failed: false,
        }
    }

    // Return whether settings window was requested
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        settings: &mut crate::gui::settings::SettingsWindow,
    ) -> bool {
        #[cfg(all(target_os = "linux", feature = "wayland_layer"))]
        if self.is_wayland && !self.sidecar_failed {
            return self.show_wayland_sidecar(ctx, open, settings);
        }
        self.show_viewport_overlay(ctx, open, settings)
    }

    fn show_viewport_overlay(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        settings: &mut crate::gui::settings::SettingsWindow,
    ) -> bool {
        let mut requested_settings = false;

        if !*open {
            // Close the floating viewport if requested (safe if already closed)
            let id = egui::ViewportId::from_hash_of("floating_viewport");
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Close);
            return false;
        }

        let id = egui::ViewportId::from_hash_of("floating_viewport");
        // Tiny floating window dimensions
        self.size = egui::vec2(120.0, 28.0);
        let mut builder = egui::ViewportBuilder::default()
            .with_title("HootVoice - Floating")
            .with_inner_size(self.size)
            .with_decorations(false)
            .with_resizable(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_app_id("HootVoice-Floating");

        // Restore last saved position
        if let Some(pos) = settings.get_floating_position() {
            builder = builder.with_position(pos);
        }

        ctx.show_viewport_immediate(id, builder, |ctx2, _class| {
            // Pick background color based on current theme for better readability
            let bg_fill = ctx2.style().visuals.window_fill();
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(bg_fill)
                        .corner_radius(egui::CornerRadius::same(6))
                        .stroke(egui::Stroke::new(
                            1.0,
                            egui::Color32::from_rgba_unmultiplied(200, 200, 200, 40),
                        ))
                        // Minimal inner margin to fit in 28px height
                        .inner_margin(egui::Margin::symmetric(2, 2)),
                )
                .show(ctx2, |ui| {
                    let state = self.core.get_state();

                    // Make the whole background draggable
                    let drag_rect = ui.max_rect();
                    let drag_resp = ui.allocate_rect(drag_rect, egui::Sense::drag());
                    if drag_resp.drag_started() {
                        ctx2.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag_resp.dragged() {
                        // Persist window position continuously while dragging
                        if let Some(outer) = ctx2.input(|i| i.viewport().outer_rect) {
                            let pos = outer.min;
                            settings.set_floating_position(pos);
                        }
                    }

                    ui.scope_builder(egui::UiBuilder::new().max_rect(drag_rect), |ui| {
                        ui.horizontal(|ui| {
                            // Left padding
                            ui.add_space(4.0);
                            // Record toggle (Lucide icon)
                            // Show Mic while idle/recording, Loader while processing
                            let rec_glyph = match state {
                                SimpleRecState::Idle => Icon::Pause,
                                SimpleRecState::Recording => Icon::Mic,
                                SimpleRecState::Processing => Icon::Loader,
                                SimpleRecState::PostProcessing => Icon::Loader,
                                SimpleRecState::Busy => Icon::Loader,
                            }
                            .unicode();
                            // State color (match settings badge colors)
                            let rec_color = match state {
                                SimpleRecState::Idle => egui::Color32::from_rgb(40, 167, 69), // green
                                SimpleRecState::Recording => egui::Color32::from_rgb(220, 53, 69), // red
                                SimpleRecState::Processing => egui::Color32::from_rgb(255, 193, 7), // yellow
                                SimpleRecState::PostProcessing => {
                                    egui::Color32::from_rgb(75, 154, 242)
                                } // blue
                                SimpleRecState::Busy => egui::Color32::from_rgb(108, 117, 125), // gray
                            };

                            let rec_clicked = ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(rec_glyph)
                                            .family(FontFamily::Name("lucide".into()))
                                            .size(16.0)
                                            .color(rec_color),
                                    )
                                    .min_size(egui::vec2(24.0, 20.0)),
                                )
                                .clicked();
                            if rec_clicked
                                && state != SimpleRecState::Processing
                                && state != SimpleRecState::PostProcessing
                                && state != SimpleRecState::Busy
                            {
                                self.core.toggle_recording();
                            }

                            ui.add_space(4.0);
                            // Open Settings button
                            let settings_clicked = ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(Icon::Settings.unicode())
                                            .family(FontFamily::Name("lucide".into()))
                                            .size(16.0),
                                    )
                                    .min_size(egui::vec2(24.0, 20.0)),
                                )
                                .clicked();
                            if settings_clicked {
                                // Save current position
                                if let Some(outer) = ctx2.input(|i| i.viewport().outer_rect) {
                                    let pos = outer.min;
                                    settings.set_floating_position(pos);
                                }
                                requested_settings = true;
                                // Close the viewport
                                ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    });
                });

            // Light refresh
            ctx2.request_repaint_after(std::time::Duration::from_millis(200));
        });

        if requested_settings {
            *open = false;
        }
        requested_settings
    }

    #[cfg(all(target_os = "linux", feature = "wayland_layer"))]
    fn show_wayland_sidecar(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        settings: &mut crate::gui::settings::SettingsWindow,
    ) -> bool {
        if *open {
            // ensure child is running
            let is_running = self
                .sidecar
                .as_mut()
                .map(|c| c.try_wait().ok().flatten().is_none())
                .unwrap_or(false);
            if !is_running {
                // spawn sidecar next to current exe
                if let Ok(me) = std::env::current_exe() {
                    let sidecar = me
                        .parent()
                        .map(|p| p.join("hootvoice-float"))
                        .unwrap_or_else(|| std::path::PathBuf::from("hootvoice-float"));
                    let mut cmd = std::process::Command::new(sidecar);
                    let ppid = std::process::id();
                    cmd.env("HOOTVOICE_PARENT_PID", ppid.to_string());
                    match cmd.spawn() {
                        Ok(child) => {
                            self.sidecar = Some(child);
                        }
                        Err(e) => {
                            eprintln!("failed to spawn sidecar: {}", e);
                            // Try PATH fallback
                            match std::process::Command::new("hootvoice-float").spawn() {
                                Ok(child2) => {
                                    self.sidecar = Some(child2);
                                }
                                Err(e2) => {
                                    eprintln!("failed to spawn sidecar from PATH: {}", e2);
                                    // Avoid spamming — disable sidecar for this session and fallback
                                    self.sidecar_failed = true;
                                    return self.show_viewport_overlay(ctx, open, settings);
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // request close
            if let Some(mut child) = self.sidecar.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        false
    }
}

#[cfg(all(target_os = "linux", feature = "wayland_layer"))]
impl Drop for FloatingWindow {
    fn drop(&mut self) {
        if let Some(mut child) = self.sidecar.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// OverlayIcons and PNG-based icon loading have been removed (unused).
