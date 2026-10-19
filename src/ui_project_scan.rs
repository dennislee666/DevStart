//! Scan dialog only renders backend plans and collects explicit launch selections.
use super::*;
#[derive(Default)]
pub(super) struct ScanUi {
    pub open: bool,
    path: String,
    job: Option<u64>,
    report: Option<crate::project_scan::Report>,
    selected: HashSet<u64>,
    error: Option<String>,
    group: Option<u64>,
    new_group: String,
    confirming: bool,
}
impl DevStart {
    pub(super) fn project_scan_dialog(&mut self, ctx: &egui::Context) {
        if !self.scan_ui.open {
            return;
        }
        let Some(service) = &mut self.service else {
            return;
        };
        let draft = &mut self.scan_ui;
        if draft.report.is_none()
            && let Some(id) = draft.job
        {
            match service.scans.get(id) {
                Ok(job) if job.state == "completed" => {
                    draft.report = job.report;
                    if let Some(report) = &draft.report {
                        draft.selected = report
                            .projects
                            .iter()
                            .flat_map(|p| &p.launches)
                            .filter(|l| !l.existing)
                            .map(|l| l.id)
                            .collect();
                    }
                }
                Ok(job) if job.state != "running" => {
                    draft.error = job.error;
                    draft.job = None;
                }
                Err(error) => {
                    draft.error = Some(error);
                    draft.job = None;
                }
                _ => {}
            }
        }
        let running = draft.job.is_some() && draft.report.is_none();
        let mut close = false;
        let mut imported = None;
        let response = egui::Modal::new(egui::Id::new("project-auto-scan"))
            .frame(dialog_ui::frame(ctx))
            .show(ctx, |ui| {
                dialog_ui::content(ui);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.spacing_mut().interact_size.y = 24.0;
                ui.spacing_mut().button_padding = egui::vec2(8.0, 3.0);
                ui.set_width((ctx.content_rect().width() - 90.0).clamp(300.0, 820.0));
                ui.horizontal(|ui| {
                    ui.heading(if draft.confirming {
                        tr("确认添加")
                    } else {
                        tr("自动扫描项目")
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close |= small(ui, icon::X, tr("关闭窗口"));
                    });
                });
                if draft.confirming {
                    let group_name = draft
                        .group
                        .and_then(|id| {
                            service
                                .catalog
                                .groups
                                .iter()
                                .find(|g| g.id == id)
                                .map(|g| g.name.as_str())
                        })
                        .unwrap_or(draft.new_group.trim());
                    ui.label(tr("确认添加所选项目与启动项到以下分组？"));
                    ui.label(group_name);
                    if let Some(report) = &draft.report {
                        egui::ScrollArea::vertical()
                            .max_height(260.0)
                            .show(ui, |ui| {
                                for project in &report.projects {
                                    let launches: Vec<_> = project
                                        .launches
                                        .iter()
                                        .filter(|l| draft.selected.contains(&l.id))
                                        .collect();
                                    if !launches.is_empty() {
                                        ui.label(&project.name);
                                        for launch in launches {
                                            ui.label(
                                                RichText::new(&launch.script.name)
                                                    .small()
                                                    .color(tone(self.light, MUTED)),
                                            );
                                        }
                                    }
                                }
                            });
                    }
                    dialog_ui::actions(ui, |ui| {
                        if ui.button(tr("取消")).clicked() {
                            draft.confirming = false;
                        }
                        if ui.button(tr("确认添加")).clicked()
                            && let Some(id) = draft.job
                        {
                            let ids: Vec<_> = draft.selected.iter().copied().collect();
                            let target = match draft.group {
                                Some(id) => crate::project_scan::ImportGroup::Existing { id },
                                None => crate::project_scan::ImportGroup::New {
                                    name: draft.new_group.clone(),
                                },
                            };
                            match service.import_scan_grouped(id, &ids, Some(target)) {
                                Ok(result) => {
                                    imported = Some(result);
                                    close = true;
                                }
                                Err(error) => {
                                    draft.error = Some(error);
                                    draft.confirming = false;
                                }
                            }
                        }
                    });
                    return;
                }
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        !running,
                        egui::TextEdit::singleline(&mut draft.path)
                            .hint_text(tr("选择扫描目录"))
                            .desired_width((ui.available_width() - 240.0).max(80.0)),
                    );
                    if ui
                        .add_enabled(!running, egui::Button::new(tr("选择目录")))
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        draft.path = service::path_label(&path);
                    }
                    if ui
                        .add_enabled(
                            !running && !draft.path.trim().is_empty(),
                            egui::Button::new(button_label(
                                ui,
                                &format!("{} {}", icon::MAGNIFYING_GLASS, tr("开始扫描")),
                                true,
                            )),
                        )
                        .clicked()
                    {
                        match service
                            .scans
                            .start(std::path::Path::new(draft.path.trim()), &service.catalog)
                        {
                            Ok(id) => {
                                draft.job = Some(id);
                                draft.report = None;
                                draft.confirming = false;
                                if draft.group.is_none() {
                                    draft.group = service.catalog.groups.first().map(|g| g.id);
                                }
                                draft.selected.clear();
                                draft.error = None;
                            }
                            Err(e) => draft.error = Some(e),
                        }
                    }
                });
                if running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(tr("正在扫描项目与启动脚本…"));
                    });
                }
                if let Some(error) = &draft.error {
                    ui.colored_label(tone(self.light, RED), error);
                }
                if let Some(report) = &draft.report {
                    if report.truncated {
                        ui.colored_label(
                            tone(self.light, RED),
                            tr("已达到扫描上限，部分目录未扫描。请选择更具体的目录继续扫描。"),
                        );
                    }
                    if !report.warnings.is_empty() {
                        egui::CollapsingHeader::new(tr("扫描提示")).show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .max_height(100.0)
                                .show(ui, |ui| {
                                    for warning in &report.warnings {
                                        ui.label(warning);
                                    }
                                });
                        });
                    }
                    ui.label(tf(
                        "发现 {count} 个项目，勾选需要添加的启动项",
                        &[("count", report.projects.len().to_string())],
                    ));
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() * 0.45).max(120.0))
                        .show(ui, |ui| {
                            for project in &report.projects {
                                egui::Frame::new().inner_margin(6.0).show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        let mut all = project
                                            .launches
                                            .iter()
                                            .filter(|l| !l.existing)
                                            .all(|l| draft.selected.contains(&l.id));
                                        if ui
                                            .add_enabled(
                                                project.launches.iter().any(|l| !l.existing),
                                                egui::Checkbox::new(&mut all, &project.name),
                                            )
                                            .changed()
                                        {
                                            for launch in
                                                project.launches.iter().filter(|l| !l.existing)
                                            {
                                                if all {
                                                    draft.selected.insert(launch.id);
                                                } else {
                                                    draft.selected.remove(&launch.id);
                                                }
                                            }
                                        }
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {}",
                                                project.group, project.stack
                                            ))
                                            .small()
                                            .color(tone(self.light, MUTED)),
                                        );
                                    });
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(service::path_label(&project.path))
                                                .small()
                                                .color(tone(self.light, MUTED)),
                                        )
                                        .truncate(),
                                    );
                                    if project.launches.is_empty() {
                                        ui.label(
                                            RichText::new(tr("未识别到可添加的启动项"))
                                                .small()
                                                .color(tone(self.light, MUTED)),
                                        );
                                    }
                                    for launch in &project.launches {
                                        ui.push_id(launch.id, |ui| {
                                            ui.horizontal(|ui| {
                                                let mut selected =
                                                    draft.selected.contains(&launch.id);
                                                let name_width = (ui.available_width() * 0.28)
                                                    .clamp(90.0, 200.0);
                                                ui.style_mut().wrap_mode =
                                                    Some(egui::TextWrapMode::Truncate);
                                                if ui
                                                    .add_enabled_ui(!launch.existing, |ui| {
                                                        ui.add_sized(
                                                            [name_width, 24.0],
                                                            egui::Checkbox::new(
                                                                &mut selected,
                                                                &launch.script.name,
                                                            ),
                                                        )
                                                    })
                                                    .inner
                                                    .changed()
                                                {
                                                    if selected {
                                                        draft.selected.insert(launch.id);
                                                    } else {
                                                        draft.selected.remove(&launch.id);
                                                    }
                                                }
                                                if launch.existing {
                                                    ui.label(
                                                        RichText::new(tr("已添加"))
                                                            .small()
                                                            .color(tone(self.light, MUTED)),
                                                    );
                                                }
                                                ui.add_sized(
                                                    [(ui.available_width() - 80.0).max(20.0), 24.0],
                                                    egui::Label::new(
                                                        RichText::new(&launch.command)
                                                            .monospace()
                                                            .small()
                                                            .color(tone(self.light, MUTED)),
                                                    )
                                                    .truncate(),
                                                )
                                                .on_hover_text(&launch.command);
                                                ui.label(
                                                    launch
                                                        .port
                                                        .map(|p| p.to_string())
                                                        .unwrap_or_else(|| "--".into()),
                                                )
                                                .on_hover_text(
                                                    launch.port_source.as_deref().unwrap_or("--"),
                                                );
                                            });
                                        });
                                    }
                                });
                                ui.add_space(4.0);
                            }
                        });
                }
                if draft
                    .report
                    .as_ref()
                    .is_some_and(|r| !r.projects.is_empty())
                {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(tr("添加到分组"));
                        let name = draft
                            .group
                            .and_then(|id| {
                                service
                                    .catalog
                                    .groups
                                    .iter()
                                    .find(|g| g.id == id)
                                    .map(|g| g.name.clone())
                            })
                            .unwrap_or_else(|| tr("新建分组").to_string());
                        egui::ComboBox::from_id_salt("scan-target-group")
                            .selected_text(name)
                            .show_ui(ui, |ui| {
                                for group in &service.catalog.groups {
                                    ui.selectable_value(
                                        &mut draft.group,
                                        Some(group.id),
                                        &group.name,
                                    );
                                }
                                ui.selectable_value(&mut draft.group, None, tr("新建分组"));
                            });
                        if draft.group.is_none() {
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.new_group)
                                    .hint_text(tr("分组名称"))
                                    .desired_width(200.0),
                            );
                        }
                    });
                    dialog_ui::actions(ui, |ui| {
                        if ui.button(tr("取消")).clicked() {
                            close = true;
                        }
                        let valid = !draft.selected.is_empty()
                            && (draft.group.is_some() || !draft.new_group.trim().is_empty());
                        if ui
                            .add_enabled(valid, egui::Button::new(tr("确认添加")))
                            .clicked()
                        {
                            draft.confirming = true;
                        }
                    });
                }
            });
        if let Some(result) = imported {
            self.expanded.extend(result.project_ids);
        }
        if close || response.should_close() {
            if running && let Some(id) = draft.job {
                let _ = service.scans.cancel(id);
            }
            draft.confirming = false;
            draft.open = false;
        }
    }
}
