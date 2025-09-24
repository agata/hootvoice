use eframe::egui;

use crate::dictionary::{save_dictionary, DictionaryEntry};
use lucide_icons::Icon;

use super::SettingsWindow;
use crate::i18n;

fn entry_matches_filter(entry: &DictionaryEntry, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let ql = q.to_lowercase();
    if entry.canonical.to_lowercase().contains(&ql) {
        return true;
    }
    if entry.aliases.iter().any(|a| a.to_lowercase().contains(&ql)) {
        return true;
    }
    if entry.include.iter().any(|k| k.to_lowercase().contains(&ql)) {
        return true;
    }
    false
}

impl SettingsWindow {
    pub(crate) fn ui_dictionary_section(&mut self, ui: &mut egui::Ui) {
        let strong = ui.visuals().strong_text_color();
        ui.heading(egui::RichText::new(i18n::tr("section-dictionary")).color(strong));
        ui.add_space(5.0);

        egui::Frame::default()
            .fill(ui.visuals().faint_bg_color)
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.style_mut().visuals.override_text_color = Some(strong);
                ui.set_min_width(ui.available_width());

                // Description hint: alias → canonical automatic replacement
                let prev = ui.style().visuals.override_text_color;
                ui.style_mut().visuals.override_text_color = Some(ui.visuals().weak_text_color());
                let base = ui
                    .style()
                    .text_styles
                    .get(&egui::TextStyle::Body)
                    .map(|f| f.size)
                    .unwrap_or(14.0);
                ui.label(
                    egui::RichText::new(i18n::tr("dict-description")).size((base - 1.0).max(10.0)),
                );
                ui.style_mut().visuals.override_text_color = prev;
                ui.add_space(8.0);

                // Header: show save location in a non-editable text area
                ui.label(i18n::tr("label-save-location"));
                let p = crate::dictionary::dictionary_path();
                let mut path_text = p.to_string_lossy().to_string();
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut path_text)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .interactive(false)
                        .frame(false),
                );
                resp.on_hover_text(path_text);
                ui.add_space(6.0);
                // Buttons under the text area
                ui.horizontal(|ui| {
                    if ui.button(i18n::tr("btn-open-folder")).clicked() {
                        if let Some(parent) = p.parent() {
                            crate::utils::reveal_in_file_manager(parent);
                        }
                    }
                    if ui.button(i18n::tr("btn-reload")).clicked() {
                        match crate::dictionary::load_or_init_dictionary() {
                            Ok(list) => {
                                self.dict_entries = list;
                                self.dict_dirty = false;
                                self.save_status_message = Some(i18n::tr("msg-dict-reloaded"));
                                self.pending_apply_dictionary = true; // update core as well
                            }
                            Err(e) => {
                                self.save_status_message =
                                    Some(format!("{} {}", i18n::tr("msg-dict-reload-failed"), e));
                            }
                        }
                    }
                    if ui.button(i18n::tr("btn-add-entry")).clicked() {
                        self.open_dict_editor_new();
                    }
                });

                ui.add_space(8.0);

                // Search filter (magnifier icon + hint)
                ui.horizontal(|ui| {
                    let icon = egui::RichText::new(Icon::Search.unicode())
                        .size(16.0)
                        .color(ui.visuals().weak_text_color());
                    ui.label(icon);
                    let hint = egui::RichText::new(i18n::tr("search-hint"))
                        .color(ui.visuals().weak_text_color());
                    let te = egui::TextEdit::singleline(&mut self.dict_filter_text)
                        .hint_text(hint)
                        .desired_width(260.0);
                    ui.add(te);
                    if !self.dict_filter_text.is_empty()
                        && ui
                            .add(egui::Button::new(i18n::tr("btn-clear")).small())
                            .clicked()
                    {
                        self.dict_filter_text.clear();
                    }
                });
                ui.add_space(6.0);

                // Grid header
                let mut edit_to_open: Option<usize> = None;
                let mut delete_index: Option<usize> = None;
                // Adjust alias column width (~2/3 of previous) + new Include column (~1/3)
                let base = (ui.available_width() * 0.275).max(180.0);
                let alias_col_width = (base * (2.0 / 3.0)).max(140.0);
                let cond_col_width = (base * (1.0 / 3.0)).max(120.0);
                egui::Grid::new("dict_grid")
                    .num_columns(4)
                    .spacing(egui::vec2(10.0, 6.0))
                    .striped(true)
                    .show(ui, |ui| {
                        // Header row: vertically centered
                        ui.allocate_ui_with_layout(
                            egui::vec2(0.0, 0.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.strong(i18n::tr("col-standard"));
                            },
                        );
                        // Header: left align, vertically centered; keep widths consistent
                        ui.allocate_ui_with_layout(
                            egui::vec2(alias_col_width, 0.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.strong(i18n::tr("col-aliases"));
                            },
                        );
                        // New header: Include (apply only if text contains any)
                        ui.allocate_ui_with_layout(
                            egui::vec2(cond_col_width, 0.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.strong(i18n::tr("col-include"));
                            },
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(0.0, 0.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.strong(i18n::tr("col-actions"));
                            },
                        );
                        ui.end_row();

                        // Rows
                        for (i, entry) in self.dict_entries.iter().enumerate() {
                            if !entry_matches_filter(entry, &self.dict_filter_text) {
                                continue;
                            }
                            // Canonical: vertically centered
                            ui.allocate_ui_with_layout(
                                egui::vec2(0.0, 0.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.label(entry.canonical.as_str());
                                },
                            );
                            let alias_preview = if entry.aliases.is_empty() {
                                i18n::tr("none")
                            } else {
                                entry.aliases.join(", ")
                            };
                            // Aliases: left align, vertically centered, truncated to width
                            ui.allocate_ui_with_layout(
                                egui::vec2(alias_col_width, 0.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(egui::Label::new(alias_preview).truncate());
                                },
                            );
                            // Include: comma-separated keywords
                            let cond_preview = if entry.include.is_empty() {
                                String::new()
                            } else {
                                entry.include.join(", ")
                            };
                            ui.allocate_ui_with_layout(
                                egui::vec2(cond_col_width, 0.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(egui::Label::new(cond_preview).truncate());
                                },
                            );
                            // Actions: horizontally aligned buttons, vertically centered
                            let (mut edit_clicked, mut del_clicked) = (false, false);
                            ui.allocate_ui_with_layout(
                                egui::vec2(0.0, 0.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    if ui.button(i18n::tr("btn-edit")).clicked() {
                                        edit_clicked = true;
                                    }
                                    if ui.button(i18n::tr("btn-delete")).clicked() {
                                        del_clicked = true;
                                    }
                                },
                            );
                            if edit_clicked {
                                edit_to_open = Some(i);
                            }
                            if del_clicked {
                                delete_index = Some(i);
                            }
                            ui.end_row();
                        }
                    });

                // Apply row actions after the grid borrow ends
                if let Some(idx) = delete_index {
                    self.dict_entries.remove(idx);
                    if let Err(e) = save_dictionary(&self.dict_entries) {
                        self.save_status_message =
                            Some(format!("[Dictionary] Failed to save deletion: {}", e));
                    } else {
                        self.save_status_message = Some(i18n::tr("msg-dict-entry-deleted"));
                        self.pending_apply_dictionary = true; // apply to core
                    }
                }
                if let Some(i) = edit_to_open {
                    self.open_dict_editor_edit(i);
                }

                // Editor dialog
                if self.dict_editor_open {
                    // Use a local open flag to avoid borrowing self mutably for the .open() call.
                    let mut w_open = self.dict_editor_open;
                    // Defer actions to after show() to avoid multiple mutable borrows of self
                    enum EditorAction {
                        Save,
                        Cancel,
                    }
                    let mut action: Option<EditorAction> = None;
                    egui::Window::new(i18n::tr("title-edit-entry"))
                        .open(&mut w_open)
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .default_size(egui::vec2(420.0, 340.0))
                        .show(ui.ctx(), |ui| {
                            ui.label(i18n::tr("label-standard"));
                            ui.add(
                                egui::TextEdit::singleline(&mut self.dict_editor_canonical)
                                    .desired_width(360.0),
                            );
                            ui.add_space(6.0);
                            ui.label(i18n::tr("label-aliases"));
                            let mut alias_to_remove: Option<usize> = None;
                            egui::ScrollArea::vertical()
                                .id_salt("dict_aliases_scroll")
                                .max_height(220.0)
                                .show(ui, |ui| {
                                    for (j, alias) in
                                        self.dict_editor_aliases.iter_mut().enumerate()
                                    {
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::TextEdit::singleline(alias)
                                                    .desired_width(360.0),
                                            );
                                            if ui
                                                .add(
                                                    egui::Button::new(i18n::tr("btn-delete"))
                                                        .small(),
                                                )
                                                .clicked()
                                            {
                                                alias_to_remove = Some(j);
                                            }
                                        });
                                    }
                                });
                            if let Some(j) = alias_to_remove {
                                self.dict_editor_aliases.remove(j);
                            }
                            if ui.button(i18n::tr("btn-add-alias")).clicked() {
                                self.dict_editor_aliases.push(String::new());
                            }
                            ui.add_space(10.0);

                            // Include (apply only when input text contains any of these)
                            ui.strong(i18n::tr("label-include"));
                            ui.small(
                                egui::RichText::new(i18n::tr("hint-include"))
                                    .color(ui.visuals().weak_text_color()),
                            );
                            let mut inc_to_remove: Option<usize> = None;
                            egui::ScrollArea::vertical()
                                .id_salt("dict_includes_scroll")
                                .max_height(160.0)
                                .show(ui, |ui| {
                                    for (j, kw) in self.dict_editor_includes.iter_mut().enumerate()
                                    {
                                        ui.horizontal(|ui| {
                                            ui.add(
                                                egui::TextEdit::singleline(kw).desired_width(360.0),
                                            );
                                            if ui
                                                .add(
                                                    egui::Button::new(i18n::tr("btn-delete"))
                                                        .small(),
                                                )
                                                .clicked()
                                            {
                                                inc_to_remove = Some(j);
                                            }
                                        });
                                    }
                                });
                            if let Some(j) = inc_to_remove {
                                self.dict_editor_includes.remove(j);
                            }
                            if ui.button(i18n::tr("btn-add-include")).clicked() {
                                self.dict_editor_includes.push(String::new());
                            }
                            // Footer: separator + right-aligned action bar
                            ui.add_space(6.0);
                            ui.separator();
                            ui.add_space(6.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // Temporarily increase button padding/size to look like dialog actions
                                    ui.scope(|ui| {
                                        let mut style: egui::Style = ui.style().as_ref().clone();
                                        style.spacing.button_padding = egui::vec2(14.0, 8.0);
                                        style.spacing.item_spacing.x = 8.0;
                                        ui.set_style(style);

                                        let can_save =
                                            !self.dict_editor_canonical.trim().is_empty();
                                        // Right: Save (prominent)
                                        let save_label =
                                            egui::RichText::new(i18n::tr("btn-save-apply"))
                                                .strong();
                                        let mut save_btn = egui::Button::new(save_label)
                                            .min_size(egui::vec2(128.0, 36.0))
                                            .sense(egui::Sense::click());
                                        // Use selection color to emphasize
                                        save_btn = save_btn.fill(ui.visuals().selection.bg_fill);
                                        if ui.add_enabled(can_save, save_btn).clicked() {
                                            action = Some(EditorAction::Save);
                                        }

                                        // Left: Cancel (standard button)
                                        if ui
                                            .add(
                                                egui::Button::new(i18n::tr("btn-cancel"))
                                                    .min_size(egui::vec2(108.0, 36.0)),
                                            )
                                            .clicked()
                                        {
                                            action = Some(EditorAction::Cancel);
                                        }
                                    });
                                },
                            );
                        });
                    // Apply deferred action (close and optionally save)
                    match action {
                        Some(EditorAction::Save) => {
                            self.commit_dict_editor();
                            self.dict_editor_open = false;
                        }
                        Some(EditorAction::Cancel) => {
                            self.dict_editor_open = false;
                        }
                        None => {
                            // Reflect close via window X button
                            self.dict_editor_open = w_open;
                        }
                    }
                }
            });
    }

    pub(crate) fn open_dict_editor_new(&mut self) {
        self.dict_editor_open = true;
        self.dict_editor_edit_index = None;
        self.dict_editor_canonical.clear();
        // For a new entry, start with a single alias input row
        self.dict_editor_aliases.clear();
        self.dict_editor_aliases.push(String::new());
        self.dict_editor_includes.clear();
    }

    pub(crate) fn open_dict_editor_edit(&mut self, index: usize) {
        self.dict_editor_open = true;
        self.dict_editor_edit_index = Some(index);
        if let Some(e) = self.dict_entries.get(index) {
            self.dict_editor_canonical = e.canonical.clone();
            self.dict_editor_aliases = e.aliases.clone();
            if self.dict_editor_aliases.is_empty() {
                // Ensure at least one alias row even if empty
                self.dict_editor_aliases.push(String::new());
            }
            self.dict_editor_includes = e.include.clone();
        } else {
            self.dict_editor_canonical.clear();
            self.dict_editor_aliases.clear();
            self.dict_editor_aliases.push(String::new());
            self.dict_editor_includes.clear();
        }
    }

    pub(crate) fn commit_dict_editor(&mut self) {
        // sanitize: trim and drop empty aliases
        let canonical = self.dict_editor_canonical.trim().to_string();
        let mut aliases: Vec<String> = self
            .dict_editor_aliases
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        // optional: deduplicate while preserving order
        let mut seen = std::collections::HashSet::new();
        aliases.retain(|a| seen.insert(a.clone()));

        let mut include: Vec<String> = self
            .dict_editor_includes
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let mut seen2 = std::collections::HashSet::new();
        include.retain(|a| seen2.insert(a.clone()));

        let new_entry = DictionaryEntry {
            canonical,
            aliases,
            include,
        };
        match self.dict_editor_edit_index {
            Some(i) => {
                if let Some(e) = self.dict_entries.get_mut(i) {
                    *e = new_entry;
                }
            }
            None => {
                self.dict_entries.push(new_entry);
            }
        }
        // persist and apply
        match save_dictionary(&self.dict_entries) {
            Ok(()) => {
                self.save_status_message = Some(i18n::tr("msg-dict-saved"));
                self.pending_apply_dictionary = true;
            }
            Err(e) => {
                self.save_status_message =
                    Some(format!("{} {}", i18n::tr("msg-dict-save-failed"), e));
            }
        }
        // Do not manage dialog visibility here; caller decides.
    }
}
