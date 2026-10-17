//! Two-step selection of existing remote targets; this never edits repository configuration.
use super::*;
pub(super) struct Picker {
    project: u64,
    current: Option<String>,
    candidate: Option<String>,
    confirming: bool,
}
impl Picker {
    pub(super) fn new(project: u64, current: Option<String>) -> Self {
        Self {
            project,
            candidate: current.clone(),
            current,
            confirming: false,
        }
    }
}
impl DevStart {
    pub(super) fn git_remote_dialog(&mut self, ctx: &egui::Context) {
        let Some(picker) = &mut self.git_page.remote_picker else {
            return;
        };
        let Some(details) = &self.git_page.details else {
            self.git_page.remote_picker = None;
            return;
        };
        if details.repository.project_id != picker.project {
            self.git_page.remote_picker = None;
            return;
        }
        let current = details
            .remotes
            .iter()
            .find(|r| Some(&r.key) == picker.current.as_ref());
        let candidate = details
            .remotes
            .iter()
            .find(|r| Some(&r.key) == picker.candidate.as_ref());
        let mut cancel = false;
        let mut apply = None;
        let response = egui::Modal::new(egui::Id::new(("git-remote-picker", picker.confirming)))
            .frame(dialog_ui::frame(ctx))
            .show(ctx, |ui| {
                dialog_ui::content(ui);
                ui.set_width((ctx.content_rect().width() - 80.0).clamp(280.0, 560.0));
                ui.heading(tr("切换远程仓库"));
                ui.label(
                    RichText::new(&details.repository.name)
                        .small()
                        .color(tone(self.light, MUTED)),
                );
                ui.label(tr("当前远程仓库"));
                if let Some(remote) = current {
                    ui.label(format!("{} · {}", remote.name, remote.url));
                }
                ui.add_space(8.0);
                if picker.confirming {
                    ui.label(tr("是否切换到以下远程仓库？"));
                    if let Some(remote) = candidate {
                        ui.label(format!("{} · {}", remote.name, remote.url));
                    }
                    ui.label(
                        RichText::new(tr(
                            "切换后，更新和推送将使用所选地址。此操作不修改 Git 远程配置。",
                        ))
                        .small()
                        .color(tone(self.light, MUTED)),
                    );
                } else {
                    ui.label(tr("可选远程仓库"));
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for remote in &details.remotes {
                                ui.radio_value(
                                    &mut picker.candidate,
                                    Some(remote.key.clone()),
                                    format!("{} · {}", remote.name, remote.url),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            }
                        });
                }
                dialog_ui::actions(ui, |ui| {
                    if ui.button(tr("取消")).clicked() {
                        cancel = true;
                    }
                    let valid = candidate.is_some() && picker.candidate != picker.current;
                    if ui
                        .add_enabled(valid, egui::Button::new(tr("确认")))
                        .on_hover_cursor(if valid {
                            egui::CursorIcon::PointingHand
                        } else {
                            egui::CursorIcon::Default
                        })
                        .clicked()
                    {
                        if picker.confirming {
                            apply = picker.candidate.clone();
                        } else {
                            picker.confirming = true;
                        }
                    }
                });
            });
        if let Some(key) = apply {
            // Only a key from the currently displayed repository can become its UI target.
            self.git_page.remote_choices.insert(picker.project, key);
            self.git_page.remote_picker = None;
        } else if cancel || response.should_close() {
            self.git_page.remote_picker = None;
        }
    }
}
