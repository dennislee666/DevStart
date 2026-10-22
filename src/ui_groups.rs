//! Group dialogs collect intent only; backend owns validation and catalog changes.
use super::*;
#[derive(Default)]
pub(super) struct GroupsUi {
    pub delete: Option<(crate::group_service::DeletionTarget, String)>,
    phrase: String,
    success: &'static str,
    deleting: Option<crate::group_service::Deletion>,
    pending: Option<std::sync::mpsc::Receiver<Result<crate::group_service::RefreshPlan, String>>>,
}
impl GroupsUi {
    pub(super) fn pending_is_idle(&self) -> bool {
        self.pending.is_none() && self.deleting.is_none()
    }
}
impl DevStart {
    pub(super) fn refresh_group(&mut self, id: u64, ctx: &egui::Context) {
        if self.groups_ui.pending.is_some() {
            return;
        }
        let Some(service) = &self.service else {
            return;
        };
        let catalog = service.catalog.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        self.groups_ui.pending = Some(rx);
        std::thread::spawn(move || {
            let result = crate::group_service::prepare(&catalog, id);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }
    pub(super) fn group_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.groups_ui.pending {
            let reply = match rx.try_recv() {
                Ok(value) => Some(value),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Some(Err("分组扫描任务意外中断".into()))
                }
                Err(_) => None,
            };
            if let Some(result) = reply {
                self.groups_ui.pending = None;
                let result = result.and_then(|plan| {
                    self.service
                        .as_mut()
                        .ok_or("服务不可用")?
                        .apply_group_refresh(plan)
                });
                let (success, message) = match result {
                    Ok(count) => (true, format!("{}: {count}", tr("分组项目与启动项已更新"))),
                    Err(error) => {
                        eprintln!("[dev start groups] refresh failed: {error}");
                        (false, error)
                    }
                };
                self.git_page.push_notice(success, message);
            }
        }
        if let Some(deletion) = &self.groups_ui.deleting {
            let result = self
                .service
                .as_mut()
                .ok_or_else(|| "服务不可用".to_string())
                .and_then(|s| s.finish_deletion(deletion));
            match result {
                Ok(None) => {
                    egui::Area::new(egui::Id::new("group-deletion-progress"))
                        .anchor(egui::Align2::CENTER_TOP, [0.0, 48.0])
                        .show(ctx, |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(tr("正在停止关联实例并移除登记…"));
                                });
                            });
                        });
                    ctx.request_repaint_after(Duration::from_millis(50));
                }
                result => {
                    self.groups_ui.deleting = None;
                    match result {
                        Ok(Some(_)) => self
                            .git_page
                            .push_notice(true, tr(self.groups_ui.success).into()),
                        Err(error) => {
                            eprintln!("[dev start groups] deletion failed: {error}");
                            self.git_page.push_notice(false, error);
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        let Some((target, name)) = self.groups_ui.delete.clone() else {
            return;
        };
        let mut close = false;
        let mut execute = false;
        egui::Modal::new(egui::Id::new("delete-group"))
            .frame(dialog_ui::frame(ctx))
            .show(ctx, |ui| {
                dialog_ui::content(ui);
                ui.set_width(440.0_f32.min(ctx.content_rect().width() - 60.0));
                ui.horizontal(|ui| {
                    ui.heading(tr(target.phrase()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close |= small(ui, icon::X, tr("关闭窗口"));
                    });
                });
                ui.label(name);
                ui.label(tr(match target {
                    crate::group_service::DeletionTarget::Group(_) => {
                        "仅移除分组及其中项目、启动项在 DevStart 的登记。不会删除项目目录或源文件。"
                    }
                    crate::group_service::DeletionTarget::Project(_) => {
                        "仅移除项目及其启动项在 DevStart 的登记。不会删除项目目录或源文件。"
                    }
                }));
                ui.label(tr(
                    "如有关联运行实例，将先优雅停止，超时后强制关闭，再删除登记。",
                ));
                ui.label(tr(match target {
                    crate::group_service::DeletionTarget::Group(_) => "请输入“删除分组”以确认",
                    crate::group_service::DeletionTarget::Project(_) => "请输入“删除项目”以确认",
                }));
                ui.text_edit_singleline(&mut self.groups_ui.phrase);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    execute = ui
                        .add_enabled(
                            self.groups_ui.phrase == target.phrase(),
                            egui::Button::new(tr("确认")),
                        )
                        .clicked();
                    close |= ui.button(tr("取消")).clicked();
                });
            });
        if execute {
            let result = self
                .service
                .as_mut()
                .ok_or_else(|| "服务不可用".to_string())
                .and_then(|s| s.begin_deletion(target));
            match result {
                Ok(deletion) => {
                    close = true;
                    self.groups_ui.deleting = Some(deletion);
                    self.groups_ui.success = target.success();
                    ctx.request_repaint();
                }
                Err(error) => self.git_page.push_notice(false, error),
            }
        }
        if close {
            self.groups_ui.delete = None;
            self.groups_ui.phrase.clear();
        }
    }
}
