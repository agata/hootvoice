use std::path::PathBuf;
// no Arc/Mutex needed in this module
use std::thread;

use eframe::egui;

use crate::transcription::{download_with_progress_cancelable, SUPPORTED_MODELS};
use crate::utils::app_config_dir;

use super::SettingsWindow;

impl SettingsWindow {
    pub(super) fn star_string(value: f32) -> String {
        // Represent 1..=5 by filled/empty stars
        let v = value.round() as i32;
        let mut s = String::new();
        for _ in 0..v {
            s.push('★');
        }
        for _ in v..5 {
            s.push('☆');
        }
        s
    }

    pub(super) fn model_quality_speed_panel(&self, ui: &mut egui::Ui) {
        let info = &SUPPORTED_MODELS[self.selected_model_index];
        let speed_frac = (info.speed_rating / 5.0).clamp(0.0, 1.0);
        let quality_frac = (info.quality_rating / 5.0).clamp(0.0, 1.0);
        ui.add_space(6.0);
        egui::Frame::default()
            .fill(ui.visuals().extreme_bg_color)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::tr("label-speed"));
                    ui.add(egui::ProgressBar::new(speed_frac).desired_width(120.0));
                    ui.label(Self::star_string(info.speed_rating));
                });
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::tr("label-accuracy"));
                    ui.add(egui::ProgressBar::new(quality_frac).desired_width(120.0));
                    ui.label(Self::star_string(info.quality_rating));
                });
                ui.add_space(4.0);
                // Rough estimates: memory and CPU time per minute
                let size_mb = info.size_bytes as f64 / 1_000_000f64;
                let mem_mb = (size_mb * 1.3).ceil();
                let time_per_minute = match info.speed_rating.round() as i32 {
                    // rough guide
                    5 => crate::i18n::tr("estimate-sec-per-min-10"),
                    4 => crate::i18n::tr("estimate-sec-per-min-20"),
                    3 => crate::i18n::tr("estimate-sec-per-min-40"),
                    2 => crate::i18n::tr("estimate-sec-per-min-80"),
                    _ => crate::i18n::tr("estimate-sec-per-min-160"),
                };
                ui.label(format!(
                    "{} {:.0}MB | {} {:.0}MB | {} {}",
                    crate::i18n::tr("label-capacity-estimate"),
                    size_mb,
                    crate::i18n::tr("label-memory-estimate"),
                    mem_mb,
                    crate::i18n::tr("label-speed-estimate-cpu"),
                    time_per_minute
                ))
                .on_hover_text(crate::i18n::tr("note-estimates-variance"));
                ui.small(format!(
                    "{} {}",
                    crate::i18n::tr("note-prefix"),
                    crate::i18n::tr(info.notes_key)
                ));
            });
    }

    pub fn prompt_download_for_current_selection(&mut self) {
        self.show_download_confirm = true;
    }

    pub fn take_model_to_apply(&mut self) -> Option<PathBuf> {
        let pending = {
            let mut guard = self.pending_apply_model.lock().unwrap();
            guard.take()
        };
        if let Some(path) = pending {
            // Reflect into settings (save as relative models/<filename>)
            let rel = path
                .file_name()
                .map(|f| PathBuf::from("models").join(f))
                .unwrap_or_else(|| PathBuf::from("models/ggml-large-v3.bin"));
            self.settings.whisper_model_path = rel;
            self.check_changes();
            self.save_settings();
            self.save_status_message = Some("Model applied".to_string());
            // Return absolute path for actual loading
            Some(path)
        } else {
            None
        }
    }

    pub(super) fn start_download_current_selection(&self) {
        let info = &SUPPORTED_MODELS[self.selected_model_index];
        // Download destination under OS-standard models dir
        let dest = app_config_dir().join("models").join(info.filename);
        *self.downloading.lock().unwrap() = true;
        *self.download_progress.lock().unwrap() = Some((0, info.size_bytes));
        *self.download_message.lock().unwrap() = Some(crate::i18n::tr("msg-download-started"));
        self.download_cancel_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);

        let prog = self.download_progress.clone();
        let downloading = self.downloading.clone();
        let msg = self.download_message.clone();
        let pending = self.pending_apply_model.clone();
        let cancel = self.download_cancel_flag.clone();

        thread::spawn(move || {
            let url = info.url;
            let res = download_with_progress_cancelable(url, &dest, cancel, |done, total| {
                if let Ok(mut p) = prog.lock() {
                    *p = Some((done, total));
                }
            });
            match res {
                Ok(()) => {
                    if let Ok(mut m) = msg.lock() {
                        *m = Some(crate::i18n::tr("msg-download-completed"));
                    }
                    if let Ok(mut ap) = pending.lock() {
                        *ap = Some(dest.clone());
                    }
                }
                Err(e) => {
                    if let Ok(mut m) = msg.lock() {
                        *m = Some(format!("Error/Cancelled: {}", e));
                    }
                }
            }
            if let Ok(mut d) = downloading.lock() {
                *d = false;
            }
        });
    }
}
