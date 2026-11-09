//! Group dialogs collect intent only; backend owns validation and catalog changes.
use super::*;
#[derive(Default)]
pub(super) struct GroupsUi {
    pub manage_open: bool,
    pub renaming: Option<(u64, String)>,
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
    pub(super) fn group_management_dialog(&mut self, ctx: &egui::Context) {
        if !self.groups_ui.manage_open {
            return;
        }
        let groups = self
            .service
            .as_ref()
            .map(Service::groups_for_launch_filter)
            .unwrap_or_default();
        let custom_count = groups.iter().filter(|group| group.id != 1).count();
        let mut close = false;
        let mut start_rename = None;
        let mut save_rename = None;
        let mut cancel_rename = false;
        let mut move_group = None;
        egui::Modal::new(egui::Id::new("group-management"))
            .frame(dialog_ui::frame(ctx))
            .show(ctx, |ui| {
                dialog_ui::content(ui);
                ui.set_width(500.0_f32.min(ctx.content_rect().width() - 60.0));
                ui.horizontal(|ui| {
                    ui.heading(tr("分组管理"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close |= small(ui, icon::X, tr("关闭窗口"));
                    });
                });
                ui.label(
                    RichText::new(tr("只能重命名和调整自建分组；全部和默认分组固定。"))
                        .small()
                        .color(tone(self.light, MUTED)),
                );
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(tone(self.light, SURFACE))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(tr("全部")).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(tr("固定"))
                                            .small()
                                            .color(tone(self.light, MUTED)),
                                    );
                                },
                            );
                        });
                    });
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        let mut custom_position = 0;
                        for group in &groups {
                            if group.id == 1 {
                                continue;
                            }
                            let editing = self
                                .groups_ui
                                .renaming
                                .as_ref()
                                .is_some_and(|(id, _)| *id == group.id);
                            egui::Frame::new()
                                .corner_radius(6.0)
                                .inner_margin(egui::Margin::symmetric(10, 5))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        if editing {
                                            if let Some((_, name)) =
                                                self.groups_ui.renaming.as_mut()
                                            {
                                                ui.add_sized(
                                                    Vec2::new(220.0, 28.0),
                                                    egui::TextEdit::singleline(name)
                                                        .hint_text(tr("分组名称")),
                                                );
                                                if ui.button(tr("保存")).clicked() {
                                                    save_rename = Some((group.id, name.clone()));
                                                }
                                                if ui.button(tr("取消")).clicked() {
                                                    cancel_rename = true;
                                                }
                                            }
                                        } else {
                                            ui.label(
                                                RichText::new(crate::i18n::group_name(group))
                                                    .strong(),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if ui
                                                        .add_enabled(
                                                            custom_position + 1 < custom_count,
                                                            egui::Button::new("↓").frame(false),
                                                        )
                                                        .on_hover_text(tr("下移"))
                                                        .clicked()
                                                    {
                                                        move_group = Some((group.id, 1));
                                                    }
                                                    if ui
                                                        .add_enabled(
                                                            custom_position > 0,
                                                            egui::Button::new("↑").frame(false),
                                                        )
                                                        .on_hover_text(tr("上移"))
                                                        .clicked()
                                                    {
                                                        move_group = Some((group.id, -1));
                                                    }
                                                    if ui.button(tr("重命名")).clicked() {
                                                        start_rename =
                                                            Some((group.id, group.name.clone()));
                                                    }
                                                },
                                            );
                                        }
                                    });
                                });
                            custom_position += 1;
                        }
                    });
                if let Some(default_group) = groups.iter().find(|group| group.id == 1) {
                    ui.add_space(4.0);
                    egui::Frame::new()
                        .fill(tone(self.light, SURFACE))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(crate::i18n::group_name(default_group)).strong(),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(tr("固定"))
                                                .small()
                                                .color(tone(self.light, MUTED)),
                                        );
                                    },
                                );
                            });
                        });
                }
                ui.add_space(8.0);
                dialog_ui::actions(ui, |ui| {
                    close |= ui.button(tr("关闭")).clicked();
                });
            });
        if let Some((id, name)) = start_rename {
            self.groups_ui.renaming = Some((id, name));
        }
        if cancel_rename {
            self.groups_ui.renaming = None;
        }
        if let Some((id, name)) = save_rename {
            let result = self
                .service
                .as_mut()
                .ok_or_else(|| tr("工作空间不可用").to_owned())
                .and_then(|service| service.rename_group(id, &name));
            match result {
                Ok(()) => {
                    self.groups_ui.renaming = None;
                    self.git_page.push_notice(true, tr("分组名称已更新").into());
                }
                Err(error) => self.git_page.push_notice(false, error),
            }
        }
        if let Some((id, direction)) = move_group {
            let result = self
                .service
                .as_mut()
                .ok_or_else(|| tr("工作空间不可用").to_owned())
                .and_then(|service| service.move_group(id, direction));
            if let Err(error) = result {
                self.git_page.push_notice(false, error);
            }
        }
        if close {
            self.groups_ui.manage_open = false;
            self.groups_ui.renaming = None;
        }
    }

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
