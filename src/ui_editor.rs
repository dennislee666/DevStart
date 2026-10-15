//! Native code surface; all draft, disk and version mutations go through the editor service.
use super::*;
use crate::editor::Document;
use serde_json::json;
#[derive(Clone)]
pub(super) enum EditorAction {
    Close(u64),
    Reload(u64),
    Restore(u64, u64),
}
impl DevStart {
    pub(super) fn open_editor(&mut self, sid: u64) {
        let result = self
            .service
            .as_mut()
            .ok_or("工作空间不可用".to_string())
            .and_then(|s| s.editor_open(sid));
        match result {
            Ok(_) => {
                self.selected_editor = Some(sid);
                self.filter = Filter::Editor;
            }
            Err(e) => self.error = Some(e),
        }
    }
    pub(super) fn editor_sidebar(&mut self, ui: &mut egui::Ui, row_height: f32) {
        let records = self
            .service
            .as_ref()
            .map(|s| s.editor_list())
            .unwrap_or_default();
        if records.is_empty() {
            return;
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new("启动代码")
                .small()
                .color(tone(self.light, MUTED)),
        );
        egui::ScrollArea::vertical()
            .id_salt("editor-files")
            .max_height((ui.available_height() - 96.0).clamp(0.0, 260.0))
            .show(ui, |ui| {
                for record in records {
                    let sid = record["script_id"].as_u64().unwrap();
                    let opened = record["open"].as_bool().unwrap_or(false);
                    let filename = record["filename"].as_str().unwrap_or("");
                    let selected =
                        self.filter == Filter::Editor && self.selected_editor == Some(sid);
                    let button = egui::Button::new(button_label(
                        ui,
                        &format!(
                            "{}   {}{}",
                            icon::FILE_CODE,
                            filename,
                            if record["dirty"] == true { " •" } else { "" }
                        ),
                        ui.is_enabled(),
                    ))
                    .right_text(if selected {
                        icon::EYE
                    } else if opened {
                        ""
                    } else {
                        icon::CLOCK_COUNTER_CLOCKWISE
                    })
                    .truncate()
                    .selected(selected)
                    .frame_when_inactive(selected);
                    if ui
                        .add_sized([ui.available_width(), row_height], button)
                        .on_hover_text(if selected {
                            format!("当前编辑\n{}", record["tooltip"].as_str().unwrap_or(""))
                        } else {
                            record["tooltip"].as_str().unwrap_or("").to_owned()
                        })
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.open_editor(sid);
                    }
                }
            });
    }
    fn editor_command(&mut self, method: &str, params: serde_json::Value) -> bool {
        let result = self
            .service
            .as_mut()
            .ok_or("工作空间不可用".into())
            .and_then(|s| s.editor_action(method, &params))
            .map(|_| ());
        let ok = result.is_ok();
        self.report(result);
        ok
    }
    fn save_editor(&mut self, sid: u64) -> bool {
        let Some(doc) = self
            .service
            .as_ref()
            .and_then(|s| s.editor.read(sid).ok())
            .cloned()
        else {
            return false;
        };
        self.editor_command(
            "editor.save",
            json!({"script_id":sid,"expected_revision":doc.revision}),
        )
    }
    fn request_editor_action(&mut self, action: EditorAction) {
        let sid = match action {
            EditorAction::Close(id) | EditorAction::Reload(id) | EditorAction::Restore(id, _) => id,
        };
        if self
            .service
            .as_ref()
            .is_some_and(|s| s.editor.read(sid).is_ok_and(|d| d.dirty))
        {
            self.editor_pending = Some(action);
        } else {
            self.perform_editor_action(action, false);
        }
    }
    fn perform_editor_action(&mut self, action: EditorAction, discard: bool) {
        let sid = match action {
            EditorAction::Close(id) | EditorAction::Reload(id) | EditorAction::Restore(id, _) => id,
        };
        let Some(doc) = self
            .service
            .as_ref()
            .and_then(|s| s.editor.read(sid).ok())
            .cloned()
        else {
            return;
        };
        let mut rev = doc.revision;
        if discard && matches!(action, EditorAction::Restore(..)) {
            if !self.editor_command(
                "editor.reload",
                json!({"script_id":sid,"expected_revision":rev,"discard":true}),
            ) {
                return;
            }
            rev = self
                .service
                .as_ref()
                .unwrap()
                .editor
                .read(sid)
                .unwrap()
                .revision
                .clone();
        }
        let ok = match action {
            EditorAction::Close(_) => self.editor_command(
                "editor.close",
                json!({"script_id":sid,"expected_revision":rev,"discard":discard}),
            ),
            EditorAction::Reload(_) => self.editor_command(
                "editor.reload",
                json!({"script_id":sid,"expected_revision":rev,"discard":discard}),
            ),
            EditorAction::Restore(_, version) => self.editor_command(
                "editor.restore",
                json!({"script_id":sid,"expected_revision":rev,"version_id":version}),
            ),
        };
        if ok && matches!(action, EditorAction::Close(_)) {
            self.selected_editor = self
                .service
                .as_ref()
                .and_then(|s| s.editor_list().into_iter().find(|v| v["open"] == true))
                .and_then(|v| v["script_id"].as_u64());
            if self.selected_editor.is_none() {
                self.filter = Filter::All;
            }
        }
        if ok {
            self.editor_reset_ids.push(sid);
            self.editor_versions = None;
        }
    }
    pub(super) fn editor_page(&mut self, ui: &mut egui::Ui) {
        let Some(sid) = self.selected_editor else {
            ui.label("选择一个启动文件开始编辑");
            return;
        };
        let Some(doc) = self
            .service
            .as_ref()
            .and_then(|s| s.editor.read(sid).ok())
            .cloned()
        else {
            ui.label("文件已关闭，可从左侧历史重新打开");
            return;
        };
        let association = self
            .service
            .as_ref()
            .unwrap()
            .editor_list()
            .into_iter()
            .find(|v| v["script_id"] == sid)
            .unwrap_or_default();
        let can_start = self
            .service
            .as_ref()
            .and_then(|s| s.script(sid))
            .is_some_and(|s| s.launch_block_reason().is_none());
        let mut save = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
        let mut close = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::W));
        let mut run = false;
        let mut reload = false;
        let mut history = false;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} {}{}",
                    icon::FILE_CODE,
                    association["filename"].as_str().unwrap_or(""),
                    if doc.dirty { " •" } else { "" }
                ))
                .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close |= small(ui, icon::X, "关闭编辑 · ⌘W");
                history = small(ui, icon::CLOCK_COUNTER_CLOCKWISE, "版本历史 · 最近 10 次");
                reload = small(ui, icon::ARROW_CLOCKWISE, "重新载入磁盘文件");
                run = action_button(
                    ui,
                    can_start,
                    icon::PLAY,
                    "保存并启动",
                    Some(tone(self.light, GREEN)),
                )
                .clicked();
                save |= small(ui, icon::FLOPPY_DISK, "保存 · ⌘S");
                ui.label(
                    RichText::new(if doc.dirty { "未保存" } else { "已保存" })
                        .small()
                        .color(tone(self.light, MUTED)),
                );
            });
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "{}  /  {}",
                        association["project"].as_str().unwrap_or(""),
                        association["launch"].as_str().unwrap_or("")
                    ))
                    .small()
                    .color(tone(self.light, MUTED)),
                )
                .truncate(),
            )
            .on_hover_text(association["tooltip"].as_str().unwrap_or(""));
        });
        ui.add(
            egui::Label::new(
                RichText::new(crate::service::path_label(&doc.path))
                    .small()
                    .color(tone(self.light, MUTED)),
            )
            .truncate(),
        )
        .on_hover_text(doc.path.display().to_string());
        ui.add_space(8.0);
        self.code_buffer(ui, &doc);
        if save || run {
            if !self.save_editor(sid) {
                return;
            }
            if run {
                self.launch(sid);
            }
        }
        if close {
            self.request_editor_action(EditorAction::Close(sid));
        }
        if reload {
            self.request_editor_action(EditorAction::Reload(sid));
        }
        if history {
            match self
                .service
                .as_mut()
                .unwrap()
                .editor_action("editor.versions", &json!({"script_id":sid}))
                .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
            {
                Ok(versions) => self.editor_versions = Some((sid, versions)),
                Err(e) => self.error = Some(e),
            }
        }
    }
    fn code_buffer(&mut self, ui: &mut egui::Ui, doc: &Document) {
        let widget_id = egui::Id::new(("source-buffer", doc.script_id));
        if egui::TextEdit::load_state(ui.ctx(), widget_id).is_none() {
            let mut state = egui::text_edit::TextEditState::default();
            state.set_undoer(egui::util::undoer::Undoer::with_settings(
                egui::util::undoer::Settings {
                    max_undos: 10,
                    ..Default::default()
                },
            ));
            state.store(ui.ctx(), widget_id);
        }
        let mut text = doc.content.clone();
        let size = self.preferences().console_font_size;
        let theme = if self.light {
            egui_extras::syntax_highlighting::CodeTheme::light(size)
        } else {
            egui_extras::syntax_highlighting::CodeTheme::dark(size)
        };
        let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width: f32| {
            let mut job = egui_extras::syntax_highlighting::highlight(
                ui.ctx(),
                ui.style(),
                &theme,
                buffer.as_str(),
                &doc.language,
            );
            crate::editor_layout::wrap(&mut job, width);
            for section in &mut job.sections {
                section.format.font_id = egui::FontId::monospace(size);
            }
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let font = egui::FontId::monospace(size);
        let color = tone(self.light, MUTED);
        let digit_width = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let gutter_width = digit_width * crate::editor_layout::gutter_digits(&text) as f32 + 16.0;
        let mut changed = false;
        egui::ScrollArea::vertical()
            .horizontal_scroll_offset(0.0)
            .id_salt(("source-editor", doc.script_id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let (gutter, _) =
                        ui.allocate_exact_size(Vec2::new(gutter_width, 0.0), egui::Sense::hover());
                    let edit = egui::TextEdit::multiline(&mut text)
                        .id(widget_id)
                        .code_editor()
                        .font(font.clone())
                        .desired_width(ui.available_width())
                        .desired_rows(30)
                        .frame(false)
                        .layouter(&mut layouter);
                    let output = edit.cursor_at_end(false).show(ui);
                    changed = output.response.changed();
                    let clip = ui.clip_rect();
                    for (number, y) in crate::editor_layout::line_numbers(&output.galley) {
                        let top = output.galley_pos.y + y;
                        if top + size >= clip.top() && top <= clip.bottom() {
                            ui.painter().text(
                                egui::pos2(gutter.right() - 8.0, top),
                                egui::Align2::RIGHT_TOP,
                                number.to_string(),
                                font.clone(),
                                color,
                            );
                        }
                    }
                });
            });
        if changed {
            self.editor_command(
                "editor.change",
                json!({"script_id":doc.script_id,"expected_revision":doc.revision,"content":text}),
            );
        }
    }
    pub(super) fn editor_quit_guard(&mut self, ctx: &egui::Context) {
        for id in self.editor_reset_ids.drain(..) {
            ctx.data_mut(|d| {
                d.remove::<egui::text_edit::TextEditState>(egui::Id::new(("source-buffer", id)))
            });
        }
        if !self.editor_quit_confirmed
            && ctx.input(|i| i.viewport().close_requested())
            && self
                .service
                .as_ref()
                .is_some_and(|s| s.editor.documents.values().any(|d| d.dirty))
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.editor_quit_pending = true;
        }
        if self.editor_quit_pending {
            dialog_ui::window(ctx, "退出前保存修改")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label("有启动文件尚未保存。");
                    dialog_ui::actions(ui, |ui| {
                        if ui.button("全部保存并退出").clicked() {
                            let ids: Vec<_> = self
                                .service
                                .as_ref()
                                .map(|s| {
                                    s.editor
                                        .documents
                                        .values()
                                        .filter(|d| d.dirty)
                                        .map(|d| d.script_id)
                                        .collect()
                                })
                                .unwrap_or_default();
                            let mut saved = true;
                            for id in ids {
                                if !self.save_editor(id) {
                                    saved = false;
                                    break;
                                }
                            }
                            if saved {
                                self.editor_quit_confirmed = true;
                                self.editor_quit_pending = false;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        if ui.button("放弃修改并退出").clicked() {
                            self.editor_quit_confirmed = true;
                            self.editor_quit_pending = false;
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("取消").clicked() {
                            self.editor_quit_pending = false;
                        }
                    });
                });
        }
    }
    pub(super) fn editor_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(action) = self.editor_pending.clone() {
            let sid = match action {
                EditorAction::Close(id)
                | EditorAction::Reload(id)
                | EditorAction::Restore(id, _) => id,
            };
            dialog_ui::window(ctx, "未保存的修改")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label("保存当前修改后继续？");
                    dialog_ui::actions(ui, |ui| {
                        if ui.button("保存并继续").clicked() && self.save_editor(sid) {
                            self.editor_pending = None;
                            self.perform_editor_action(action.clone(), false);
                        }
                        if ui.button("放弃修改").clicked() {
                            self.editor_pending = None;
                            self.perform_editor_action(action.clone(), true);
                        }
                        if ui.button("取消").clicked() {
                            self.editor_pending = None;
                        }
                    });
                });
        }
        if let Some((sid, versions)) = self.editor_versions.clone() {
            let mut opened = true;
            let mut restore = None;
            dialog_ui::window(ctx, "版本历史")
                .open(&mut opened)
                .collapsible(false)
                .default_width(480.0)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label(
                        RichText::new("回退会保留当前版本，下次启动使用回退后的文件。")
                            .small()
                            .color(tone(self.light, MUTED)),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(360.0)
                        .show(ui, |ui| {
                            for version in versions {
                                ui.horizontal(|ui| {
                                    ui.label(format!(
                                        "{}  ·  {}",
                                        crate::editor::display_time(version.created_ms),
                                        crate::editor::reason_label(&version.reason)
                                    ));
                                    ui.label(
                                        RichText::new(format!("{} B", version.bytes))
                                            .small()
                                            .color(tone(self.light, MUTED)),
                                    );
                                    if ui
                                        .button("回退")
                                        .on_hover_text(format!(
                                            "版本 {}\n{}",
                                            version.id,
                                            version.path.display()
                                        ))
                                        .clicked()
                                    {
                                        restore = Some(version.id);
                                    }
                                });
                            }
                        });
                });
            if !opened {
                self.editor_versions = None;
            }
            if let Some(id) = restore {
                self.editor_versions = None;
                self.request_editor_action(EditorAction::Restore(sid, id));
            }
        }
    }
}
