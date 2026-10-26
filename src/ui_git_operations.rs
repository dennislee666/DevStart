//! IDEA-style operation choices and explicit three-pane conflict review.
use super::*;
use git::integration::Method;
pub(super) struct UpdatePicker {
    pub project: u64,
    pub key: String,
    pub branch: String,
    pub method: Method,
}
pub(super) struct RebasePicker {
    project: u64,
    current: String,
    onto: String,
    branches: Vec<String>,
}
impl RebasePicker {
    pub fn new(project: u64, current: &str, branches: &[String]) -> Self {
        let branches: Vec<_> = branches
            .iter()
            .filter(|b| b.as_str() != current)
            .cloned()
            .collect();
        Self {
            project,
            current: current.into(),
            onto: branches.first().cloned().unwrap_or_default(),
            branches,
        }
    }
}
pub(super) struct PushRejected {
    pub project: u64,
    pub key: String,
    pub branch: String,
    pub error: String,
}
pub(super) struct ConflictEditor {
    project: u64,
    doc: git::conflicts::Document,
    result: String,
    side: Option<bool>,
    show_base: bool,
    pub error: Option<String>,
}
impl ConflictEditor {
    pub fn new(project: u64, doc: git::conflicts::Document) -> Self {
        Self {
            project,
            result: doc.result.clone(),
            doc,
            side: None,
            show_base: false,
            error: None,
        }
    }
}
fn source_pane(ui: &mut egui::Ui, title: &str, text: Option<&str>, height: f32) {
    ui.strong(tr(title));
    egui::ScrollArea::both()
        .id_salt(title)
        .max_height(height)
        .show(ui, |ui| {
            if let Some(text) = text {
                let mut text = text.to_owned();
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(22)
                        .interactive(false),
                );
            } else {
                ui.label(tr("此版本已删除文件，或文件为二进制/过大，无法显示文本。"));
            }
        });
}
impl DevStart {
    pub(super) fn git_operation_dialogs(&mut self, ctx: &egui::Context) {
        let busy = self.git_page.pending.is_some();
        let mut action = None;
        let selected = self.git_page.selected;
        if let Some(picker) = &mut self.git_page.update_picker {
            let mut close = selected != Some(picker.project);
            let response = egui::Modal::new(egui::Id::new("git-update-method"))
                .frame(dialog_ui::frame(ctx))
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.set_width(430.0);
                    ui.heading("Update Project");
                    ui.label(&picker.branch);
                    ui.radio_value(&mut picker.method, Method::Merge, "Merge")
                        .on_hover_text(tr("合并远程提交，必要时创建合并提交。"));
                    ui.radio_value(&mut picker.method, Method::Rebase, "Rebase")
                        .on_hover_text(tr(
                            "把本地提交重新应用到远程最新提交之后，会改写本地提交哈希。",
                        ));
                    ui.label(
                        RichText::new(tr(
                            "本地未提交改动会先保存并在更新后恢复；冲突由你确认处理。",
                        ))
                        .small()
                        .color(tone(self.light, MUTED)),
                    );
                    dialog_ui::actions(ui, |ui| {
                        close |= ui.button(tr("取消")).clicked();
                        if ui
                            .add_enabled(!busy, egui::Button::new(tr("更新")))
                            .clicked()
                        {
                            action = Some(Action::Update(
                                picker.key.clone(),
                                picker.branch.clone(),
                                picker.method,
                            ));
                            close = true;
                        }
                    });
                });
            if close || response.should_close() {
                self.git_page.update_picker = None;
            }
        } else if let Some(picker) = &mut self.git_page.rebase_picker {
            let mut close = selected != Some(picker.project);
            let response = egui::Modal::new(egui::Id::new("git-rebase-picker")).frame(dialog_ui::frame(ctx)).show(ctx, |ui| {
                dialog_ui::content(ui); ui.set_width(440.0); ui.heading("Rebase");
                ui.label(format!("{}: {}", tr("当前分支"), picker.current));
                ui.label(tr("将当前分支变基到"));
                egui::ComboBox::from_id_salt("git-rebase-onto").selected_text(&picker.onto).width(320.0).show_ui(ui, |ui| {
                    for branch in &picker.branches { ui.selectable_value(&mut picker.onto, branch.clone(), branch); }
                });
                ui.label(RichText::new(tr("Rebase 会改写当前分支的本地提交哈希。已共享的提交应谨慎变基；推送仍不会强制覆盖远程。")).small().color(tone(self.light, MUTED)));
                dialog_ui::actions(ui, |ui| {
                    close |= ui.button(tr("取消")).clicked();
                    if ui.add_enabled(!busy && !picker.onto.is_empty(), egui::Button::new(tr("开始 Rebase"))).clicked() {
                        action = Some(Action::Rebase(picker.onto.clone())); close = true;
                    }
                });
            });
            if close || response.should_close() {
                self.git_page.rebase_picker = None;
            }
        } else if let Some(rejected) = &self.git_page.push_rejected {
            let mut close = selected != Some(rejected.project);
            let response = egui::Modal::new(egui::Id::new("git-push-rejected")).frame(dialog_ui::frame(ctx)).show(ctx, |ui| {
                dialog_ui::content(ui); ui.set_width(500.0); ui.heading(tr("推送被拒绝"));
                ui.label(tr("远程分支有新提交。选择更新方式，成功后重试推送；未提交文件不会被自动提交。"));
                egui::CollapsingHeader::new(tr("原始错误")).show(ui, |ui| { ui.label(&rejected.error); });
                dialog_ui::actions(ui, |ui| {
                    close |= ui.button(tr("取消")).clicked();
                    for method in [Method::Merge, Method::Rebase] {
                        if ui.add_enabled(!busy, egui::Button::new(format!("{} · {}", method.label(), tr("更新并推送")))).clicked() {
                            action = Some(Action::UpdateThenPush(rejected.key.clone(), rejected.branch.clone(), method)); close = true;
                        }
                    }
                });
            });
            if close || response.should_close() {
                self.git_page.push_rejected = None;
            }
        } else if let Some(editor) = &mut self.git_page.conflict_editor {
            let mut open = selected == Some(editor.project);
            let mut close = false;
            egui::Window::new(tr("解决合并冲突")).id(egui::Id::new("git-conflict-editor")).open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .default_width(1120.0).max_width((ctx.content_rect().width() - 40.0).max(600.0)).resizable(true).collapsible(false)
                .show(ctx, |ui| {
                    ui.label(&editor.doc.file);
                    ui.label(RichText::new(tr("选择一侧完整版本，或直接编辑中间结果；应用后仅暂存该文件，点击继续完成操作。")).small().color(tone(self.light, MUTED)));
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new(tr("采用左侧版本"))).clicked() { editor.result = editor.doc.current.clone().unwrap_or_default(); editor.side = Some(false); }
                        if ui.add_enabled(!busy, egui::Button::new(tr("采用右侧版本"))).clicked() { editor.result = editor.doc.incoming.clone().unwrap_or_default(); editor.side = Some(true); }
                        ui.checkbox(&mut editor.show_base, tr("显示共同基线"));
                    });
                    let height = (ctx.content_rect().height() * 0.48).clamp(160.0, 600.0);
                    ui.columns(3, |columns| {
                        source_pane(&mut columns[0], &editor.doc.current_label, editor.doc.current.as_deref(), height);
                        columns[1].strong(tr("合并结果"));
                        egui::ScrollArea::both().id_salt("git-conflict-result").max_height(height).show(&mut columns[1], |ui| {
                            if ui.add_enabled(!busy && editor.doc.editable, egui::TextEdit::multiline(&mut editor.result).font(egui::TextStyle::Monospace).desired_rows(22).desired_width(f32::INFINITY)).changed() { editor.side = None; }
                        });
                        source_pane(&mut columns[2], &editor.doc.incoming_label, editor.doc.incoming.as_deref(), height);
                    });
                    if editor.show_base {
                        egui::CollapsingHeader::new(tr("共同基线")).default_open(true).show(ui, |ui| { source_pane(ui, "Base", editor.doc.base.as_deref(), 140.0); });
                    }
                    if let Some(error) = &editor.error { ui.colored_label(tone(self.light, RED), error); }
                    dialog_ui::actions(ui, |ui| {
                        close |= ui.button(tr("稍后处理")).clicked();
                        if ui.add_enabled(!busy && (editor.doc.editable || editor.side.is_some()), egui::Button::new(tr("应用并标记已解决"))).clicked() {
                            let resolution = match editor.side { Some(false) => git::conflicts::Resolution::Current, Some(true) => git::conflicts::Resolution::Incoming, None => git::conflicts::Resolution::Edited(editor.result.clone()) };
                            action = Some(Action::Resolve(editor.doc.file.clone(), editor.doc.token.clone(), resolution)); editor.error = None;
                        }
                    });
                });
            if close || !open {
                self.git_page.conflict_editor = None;
            }
        }
        if let Some(action) = action {
            self.git_action(action);
        }
    }
}
