//! Two-step local merge selection. Git state and mutations stay in the backend.
use super::*;
pub(super) struct Picker {
    project: u64,
    source: String,
    target: String,
    confirming: bool,
    error: Option<String>,
}
impl Picker {
    pub(super) fn new(project: u64, current: &str, branches: &[String]) -> Self {
        Self {
            project,
            source: branches
                .iter()
                .find(|b| b.as_str() != current)
                .cloned()
                .unwrap_or_default(),
            target: current.into(),
            confirming: false,
            error: None,
        }
    }
}
impl DevStart {
    pub(super) fn git_merge_dialog(&mut self, ctx: &egui::Context) {
        let Some(picker) = &mut self.git_page.merge_picker else {
            return;
        };
        let Some(details) = &self.git_page.details else {
            return;
        };
        if self.git_page.selected != Some(picker.project) {
            self.git_page.merge_picker = None;
            return;
        }
        let mut close = false;
        let mut execute = None;
        let response = egui::Modal::new(egui::Id::new(("git-merge-picker", picker.confirming)))
            .frame(dialog_ui::frame(ctx))
            .show(ctx, |ui| {
                dialog_ui::content(ui);
                ui.set_width(440.0);
                ui.horizontal(|ui| {
                    ui.heading(tr("合并分支"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close |= small(ui, icon::X, tr("关闭窗口"));
                    });
                });
                ui.label(
                    RichText::new(&details.repository.name)
                        .small()
                        .color(tone(self.light, MUTED)),
                );
                ui.add_space(10.0);
                if picker.confirming {
                    ui.label(tf(
                        "您确认将 {source} 分支合并到 {target} 分支吗？",
                        &[
                            ("source", picker.source.clone()),
                            ("target", picker.target.clone()),
                        ],
                    ));
                } else {
                    egui::Grid::new("merge-branch-fields")
                        .spacing([12.0, 12.0])
                        .show(ui, |ui| {
                            ui.add_sized(
                                [80.0, 24.0],
                                egui::Label::new(tr("从分支"))
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                            egui::ComboBox::from_id_salt("merge-source")
                                .selected_text(&picker.source)
                                .width(280.0)
                                .show_ui(ui, |ui| {
                                    for branch in &details.branches {
                                        ui.selectable_value(
                                            &mut picker.source,
                                            branch.clone(),
                                            branch,
                                        );
                                    }
                                });
                            ui.end_row();
                            ui.add_sized(
                                [80.0, 24.0],
                                egui::Label::new(tr("合并到"))
                                    .wrap_mode(egui::TextWrapMode::Extend),
                            );
                            egui::ComboBox::from_id_salt("merge-target")
                                .selected_text(&picker.target)
                                .width(280.0)
                                .show_ui(ui, |ui| {
                                    for branch in &details.branches {
                                        ui.selectable_value(
                                            &mut picker.target,
                                            branch.clone(),
                                            branch,
                                        );
                                    }
                                });
                            ui.end_row();
                        });
                }
                ui.add_space(8.0);
                if git::merging::validate_pair(&picker.source, &picker.target).is_ok() {
                for text in [
                    tf("来源分支 {source} 不会被修改。", &[("source", picker.source.clone())]),
                    tf("目标分支 {target} 将接收 {source} 的提交并更新本地文件，必要时生成合并提交。", &[("source", picker.source.clone()), ("target", picker.target.clone())]),
                    tf("操作后停留在 {target} 分支；同一仓库的其他项目也会看到文件变化。", &[("target", picker.target.clone())]),
                    tr("不推送远程，远程分支保持不变。若有冲突，将保留现场供手动处理。").into(),
                ] {
                    ui.label(RichText::new(text).small().color(tone(self.light, MUTED)));
                }
                }
                if picker.error.is_some() && git::merging::validate_pair(&picker.source, &picker.target).is_ok() { picker.error = None; }
                if let Some(error) = &picker.error { ui.colored_label(tone(self.light, RED), tr(error)); }
                ui.add_space(14.0);
                let valid = !picker.source.is_empty()
                    && !picker.target.is_empty()
                    && self.git_page.pending.is_none();
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 140.0).max(0.0) / 2.0);
                        if ui
                            .add_enabled(valid, egui::Button::new(tr("确认")))
                            .clicked()
                        {
                            if let Err(error) = git::merging::validate_pair(&picker.source, &picker.target) {
                                picker.error = Some(error);
                                picker.confirming = false;
                            } else if picker.confirming {
                                execute = Some(Action::Merge(
                                    picker.source.clone(),
                                    picker.target.clone(),
                                ));
                            } else {
                                picker.confirming = true;
                            }
                        }
                        if ui.button(tr("取消")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if execute.is_some() || close || response.should_close() {
            self.git_page.merge_picker = None;
        }
        if let Some(action) = execute {
            self.git_action(action);
        }
    }
}
