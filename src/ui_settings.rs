//! Appearance controls gather a draft; Service validates and durably applies it.
use super::*;
use crate::preferences::{ConsoleFont, Density, Theme, UiFont};
impl DevStart {
    pub(super) fn settings_page(&mut self, ui: &mut egui::Ui) {
        let mut draft = self
            .settings_draft
            .clone()
            .unwrap_or_else(|| self.preferences());
        draft.sidebar_collapsed = self.preferences().sidebar_collapsed;
        egui::ScrollArea::vertical()
            .id_salt("settings-scroll")
            .show(ui, |ui| {
                ui.set_max_width(720.0);
                ui.add_space(10.0);
                ui.label(RichText::new("外观与字体").heading().strong());
                ui.label(
                    RichText::new("调整工作台和 Console 的阅读体验")
                        .small()
                        .color(tone(self.light, MUTED)),
                );
                ui.add_space(20.0);
                section(ui, "外观");
                setting_row(ui, "主题", |ui| {
                    ui.selectable_value(&mut draft.theme, Theme::Dark, "深色");
                    ui.selectable_value(&mut draft.theme, Theme::Light, "浅色");
                    ui.selectable_value(&mut draft.theme, Theme::System, "跟随系统");
                });
                setting_row(ui, "信息密度", |ui| {
                    ui.selectable_value(&mut draft.density, Density::Compact, "紧凑");
                    ui.selectable_value(&mut draft.density, Density::Comfortable, "舒适");
                });
                ui.add_space(20.0);
                section(ui, "界面");
                setting_row(ui, "字体", |ui| {
                    egui::ComboBox::from_id_salt("ui-font")
                        .selected_text(match draft.ui_font {
                            UiFont::System => "系统字体",
                            UiFont::Monospace => "等宽字体",
                        })
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut draft.ui_font, UiFont::System, "系统字体");
                            ui.selectable_value(&mut draft.ui_font, UiFont::Monospace, "等宽字体");
                        });
                });
                setting_row(ui, "字号", |ui| {
                    ui.add(
                        egui::Slider::new(&mut draft.ui_font_size, 10.0..=18.0)
                            .step_by(1.0)
                            .suffix(" px"),
                    );
                });
                ui.add_space(20.0);
                section(ui, "Console");
                setting_row(ui, "字体", |ui| {
                    egui::ComboBox::from_id_salt("console-font")
                        .selected_text(match draft.console_font {
                            ConsoleFont::System => "系统等宽字体",
                            ConsoleFont::Menlo => "Menlo",
                            ConsoleFont::Monaco => "Monaco",
                        })
                        .width(190.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.console_font,
                                ConsoleFont::System,
                                "系统等宽字体",
                            );
                            if cfg!(target_os = "macos") {
                                ui.selectable_value(
                                    &mut draft.console_font,
                                    ConsoleFont::Menlo,
                                    "Menlo",
                                );
                                ui.selectable_value(
                                    &mut draft.console_font,
                                    ConsoleFont::Monaco,
                                    "Monaco",
                                );
                            }
                        });
                });
                setting_row(ui, "字号", |ui| {
                    ui.add(
                        egui::Slider::new(&mut draft.console_font_size, 10.0..=24.0)
                            .step_by(1.0)
                            .suffix(" px"),
                    );
                });
                setting_row(ui, "行距", |ui| {
                    ui.add(
                        egui::Slider::new(&mut draft.console_line_height, 1.0..=1.8)
                            .step_by(0.05)
                            .fixed_decimals(2),
                    );
                });
                ui.add_space(16.0);
                egui::Frame::new()
                    .fill(if self.light {
                        Color32::from_gray(244)
                    } else {
                        BG
                    })
                    .stroke(egui::Stroke::new(1.0, tone(self.light, BORDER)))
                    .corner_radius(6)
                    .inner_margin(16.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(RichText::new("预览").small().color(tone(self.light, MUTED)));
                        ui.add_space(8.0);
                        let font = egui::FontId::new(
                            draft.console_font_size,
                            egui::FontFamily::Name(
                                match draft.console_font {
                                    ConsoleFont::System => "preview-system",
                                    ConsoleFont::Menlo => "preview-menlo",
                                    ConsoleFont::Monaco => "preview-monaco",
                                }
                                .into(),
                            ),
                        );
                        ui.spacing_mut().item_spacing.y =
                            draft.console_font_size * (draft.console_line_height - 1.0);
                        ui.label(
                            RichText::new("~/Project/my-app")
                                .font(font.clone())
                                .color(tone(self.light, MUTED)),
                        );
                        ui.label(RichText::new("$ npm run dev").font(font.clone()));
                        ui.label(
                            RichText::new("Ready · http://localhost:5173")
                                .font(font)
                                .color(tone(self.light, GREEN)),
                        );
                    });
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    if ui.button("应用并保存").clicked() {
                        let result = self
                            .service
                            .as_mut()
                            .ok_or("工作空间不可用".to_string())
                            .and_then(|s| {
                                s.update_preferences(
                                    serde_json::to_value(&draft).map_err(|e| e.to_string())?,
                                )
                            });
                        match result {
                            Ok(_) => self.settings_saved = true,
                            Err(e) => self.report(Err(e)),
                        }
                    }
                    if ui.button("恢复默认").clicked() {
                        draft = Preferences::default();
                        self.settings_saved = false;
                    }
                    if self.settings_saved {
                        ui.label(
                            RichText::new("已保存")
                                .small()
                                .color(tone(self.light, GREEN)),
                        );
                    }
                });
                ui.add_space(20.0);
                section(ui, "快捷键");
                setting_row(ui, "收起 / 展开侧栏", |ui| {
                    ui.label(
                        RichText::new("⌘ B")
                            .monospace()
                            .color(tone(self.light, MUTED)),
                    );
                });
                setting_row(ui, "打开设置", |ui| {
                    ui.label(
                        RichText::new("⌘ ,")
                            .monospace()
                            .color(tone(self.light, MUTED)),
                    );
                });
            });
        if self.settings_draft.as_ref() != Some(&draft) {
            self.settings_saved = false;
        }
        self.settings_draft = Some(draft);
    }
}
fn section(ui: &mut egui::Ui, label: &str) {
    ui.label(RichText::new(label).strong());
    ui.add_space(5.0);
    ui.separator();
    ui.add_space(5.0);
}
fn setting_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(150.0, 28.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(150.0);
                ui.label(label);
            },
        );
        add(ui);
    });
}
