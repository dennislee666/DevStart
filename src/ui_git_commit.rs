//! Shared history list and read-only commit-file browser. Repository operations stay in git_commit.
use super::*;
#[derive(Default)]
pub(super) struct HistoryEvent {
    pub commit: Option<String>,
    pub offset: Option<usize>,
}
impl DevStart {
    pub(super) fn git_history_contents(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        busy: bool,
    ) -> HistoryEvent {
        let mut event = HistoryEvent::default();
        ui.strong(tr("提交历史"));
        egui::ScrollArea::vertical()
            .id_salt("git-history-list")
            .max_height(height - 54.0)
            .show(ui, |ui| {
                if let Some(d) = &self.git_page.details {
                    if d.history.is_empty() {
                        ui.label(tr("暂无提交"));
                    }
                    for (index, commit) in d.history.iter().enumerate() {
                        let fill = if self.light {
                            if index % 2 == 0 {
                                Color32::from_gray(244)
                            } else {
                                Color32::from_gray(252)
                            }
                        } else if index % 2 == 0 {
                            SURFACE
                        } else {
                            BG
                        };
                        let selected = self.git_page.browsing_commit.as_deref() == Some(&commit.id);
                        let frame = egui::Frame::new()
                            .fill(fill)
                            .inner_margin(egui::Margin {
                                left: 8,
                                right: 28,
                                top: 8,
                                bottom: 8,
                            })
                            .corner_radius(3.0)
                            .stroke(if selected {
                                egui::Stroke::new(1.0_f32, tone(self.light, ACCENT))
                            } else {
                                egui::Stroke::NONE
                            })
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(&commit.subject).strong());
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 5.0;
                                    ui.label(
                                        RichText::new(format!("{} {}", icon::USER, commit.author))
                                            .small()
                                            .color(tone(self.light, MUTED)),
                                    );
                                    ui.label(
                                        RichText::new(&commit.id[..commit.id.len().min(10)])
                                            .monospace()
                                            .small()
                                            .color(tone(self.light, MUTED).gamma_multiply(0.7)),
                                    )
                                    .on_hover_text(&commit.id);
                                });
                                ui.label(
                                    RichText::new(format!(
                                        "{} {}",
                                        icon::CALENDAR,
                                        commit.display_date()
                                    ))
                                    .small()
                                    .color(tone(self.light, MUTED)),
                                )
                                .on_hover_text(&commit.date);
                            });
                        // A subtle disclosure marker belongs to the clickable row, not a separate button.
                        let center = egui::pos2(
                            frame.response.rect.right() - 13.0,
                            frame.response.rect.center().y,
                        );
                        ui.painter().add(egui::Shape::line(
                            vec![
                                center + egui::vec2(-2.5, -4.0),
                                center + egui::vec2(1.5, 0.0),
                                center + egui::vec2(-2.5, 4.0),
                            ],
                            egui::Stroke::new(
                                1.2_f32,
                                tone(self.light, MUTED).gamma_multiply(0.65),
                            ),
                        ));
                        let response = frame
                            .response
                            .interact(egui::Sense::click())
                            .on_hover_cursor(if busy {
                                egui::CursorIcon::Default
                            } else {
                                egui::CursorIcon::PointingHand
                            })
                            .on_hover_text(tr("查看本次提交的文件"));
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::SelectableLabel,
                                ui.is_enabled() && !busy,
                                selected,
                                &commit.subject,
                            )
                        });
                        if !busy && response.clicked() {
                            event.commit = Some(commit.id.clone());
                        }
                    }
                }
            });
        if let Some(d) = &self.git_page.details {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!busy && d.offset > 0, egui::Button::new(tr("上一页")))
                    .clicked()
                {
                    event.offset = Some(d.offset.saturating_sub(50));
                }
                if ui
                    .add_enabled(
                        !busy && d.history.len() == 50,
                        egui::Button::new(tr("下一页")),
                    )
                    .clicked()
                {
                    event.offset = Some(d.offset + 50);
                }
            });
        }
        event
    }
    pub(super) fn git_commit_files_view(
        &mut self,
        ui: &mut egui::Ui,
        busy: bool,
    ) -> Option<(String, String)> {
        let mut request = None;
        let (header, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::hover());
        let back_width = 80.0_f32.min(header.width() * 0.25);
        let back = ui
            .put(
                egui::Rect::from_min_size(header.min, egui::vec2(back_width, header.height())),
                egui::Button::new(button_label(
                    ui,
                    &format!("{} {}", icon::ARROW_LEFT, tr("返回")),
                    true,
                )),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tr("返回工作区"))
            .clicked();
        // Symmetric reserved space keeps the title centered in the entire pane.
        ui.put(
            header.shrink2(egui::vec2(back_width + 8.0, 0.0)),
            egui::Label::new(button_label(
                ui,
                &format!("{} {}", icon::LIST_BULLETS, tr("提交文件")),
                true,
            ))
            .halign(egui::Align::Center)
            .truncate(),
        );
        if let (Some(inspection), Some(details)) =
            (&self.git_page.commit_files, &self.git_page.details)
        {
            let enabled = !busy && !details.repository.branch.starts_with("detached ");
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(button_label(
                        ui,
                        &format!("{} {}", icon::ARROW_COUNTER_CLOCKWISE, tr("恢复到当前版本")),
                        enabled,
                    )),
                )
                .on_hover_text(tr(
                    "将所选历史版本恢复为当前分支的最新本地提交，不修改远程；执行前需确认",
                ))
                .on_hover_cursor(if enabled {
                    egui::CursorIcon::PointingHand
                } else {
                    egui::CursorIcon::Default
                })
                .clicked()
            {
                self.git_page.confirm = Some(Action::RestoreVersion(
                    inspection.commit.id.clone(),
                    details.repository.branch.clone(),
                    details.repository.version.clone(),
                ));
            }
        }
        if let Some(inspection) = &self.git_page.commit_files {
            ui.label(RichText::new(&inspection.commit.subject).strong());
            ui.label(
                RichText::new(&inspection.commit.id)
                    .small()
                    .color(tone(self.light, MUTED)),
            );
            ui.label(
                RichText::new(inspection.base.as_deref().unwrap_or("Empty tree"))
                    .small()
                    .color(tone(self.light, MUTED)),
            );
            egui::ScrollArea::vertical()
                .id_salt("commit-files")
                .auto_shrink([false, false])
                .show_rows(ui, 32.0, inspection.files.len(), |ui, range| {
                    for file in &inspection.files[range] {
                        ui.horizontal(|ui| {
                            let color = if file.status.starts_with('A') {
                                RUNNING_GREEN
                            } else if file.status.starts_with('D') {
                                RED
                            } else {
                                MUTED
                            };
                            ui.label(
                                RichText::new(&file.status)
                                    .monospace()
                                    .color(tone(self.light, color)),
                            );
                            if let (Some(added), Some(deleted)) = (file.additions, file.deletions) {
                                ui.label(
                                    RichText::new(format!("+{added}"))
                                        .monospace()
                                        .small()
                                        .color(tone(self.light, RUNNING_GREEN)),
                                );
                                ui.label(
                                    RichText::new(format!("−{deleted}"))
                                        .monospace()
                                        .small()
                                        .color(tone(self.light, RED)),
                                );
                            } else {
                                ui.label(
                                    RichText::new(tr("二进制"))
                                        .small()
                                        .color(tone(self.light, MUTED)),
                                );
                            }
                            let selected =
                                self.git_page.selected_commit_file.as_deref() == Some(&file.path);
                            let response = ui
                                .add_enabled(
                                    !busy,
                                    egui::Button::new(&file.file_name)
                                        .selected(selected)
                                        .frame(false)
                                        .wrap_mode(egui::TextWrapMode::Truncate)
                                        .min_size(Vec2::new(0.0, 26.0)),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_ui(|ui| file_preview_tooltip(ui, &file.full_path));
                            if response.clicked() {
                                self.git_page.selected_commit_file = Some(file.path.clone());
                            }
                            if response.double_clicked() {
                                request = Some((inspection.commit.id.clone(), file.path.clone()));
                            }
                        });
                    }
                });
        } else if busy {
            ui.spinner();
        }
        if back {
            self.git_page.error = None;
            self.git_page.browsing_commit = None;
            self.git_page.commit_files = None;
            self.git_page.selected_commit_file = None;
        }
        request
    }
    pub(super) fn git_inspect_commit(&mut self, commit: String) {
        let Some(project) = self
            .git_page
            .selected
            .and_then(|id| {
                self.service
                    .as_ref()?
                    .catalog
                    .projects
                    .iter()
                    .find(|p| p.id == id)
            })
            .cloned()
        else {
            return;
        };
        self.git_page.browsing_commit = Some(commit.clone());
        self.git_page.commit_files = None;
        self.git_page.selected_commit_file = None;
        self.git_page.request(move || {
            Reply::Commit(git::commits::files(&project.path, &commit).map(Box::new))
        });
    }
    pub(super) fn git_historical_diff(&mut self, commit: String, file: String) {
        let Some(project) = self
            .git_page
            .selected
            .and_then(|id| {
                self.service
                    .as_ref()?
                    .catalog
                    .projects
                    .iter()
                    .find(|p| p.id == id)
            })
            .cloned()
        else {
            return;
        };
        self.git_page.request(move || {
            Reply::Diff(
                git::commits::diff(&project.path, &commit, &file).map(|text| {
                    crate::git_diff::Document::new(
                        format!("{}:{commit}:{file}", project.path.display()),
                        project.path.join(&file).display().to_string(),
                        text,
                    )
                }),
            )
        });
    }
}

impl DevStart {
    pub(super) fn git_diff_window(&mut self, ctx: &egui::Context) {
        use crate::git_diff::Kind;
        let page = &mut self.git_page;
        if !page.diff_open || page.diff_minimized || page.diffs.is_empty() {
            return;
        }
        let mut close_window = false;
        let mut close_tab = None;
        // Separate area identities preserve the normal window geometry while maximized.
        let mut window = egui::Window::new("Diff")
            .id(egui::Id::new(if page.diff_maximized {
                "git-diff-maximized"
            } else {
                "git-diff-viewer"
            }))
            .title_bar(false)
            .default_size(egui::vec2(960.0, 600.0))
            .min_size(egui::vec2(420.0, 260.0));
        if page.diff_maximized {
            window = window
                .fixed_rect(ctx.content_rect().shrink(8.0))
                .resizable(false)
                .movable(false);
        } else {
            window = window.resizable(true).movable(true);
        }
        window.show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Diff");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close_window = small(ui, icon::X, tr("关闭窗口"));
                    let (symbol, tooltip) = if page.diff_maximized {
                        (icon::CORNERS_IN, tr("还原窗口"))
                    } else {
                        (icon::CORNERS_OUT, tr("最大化"))
                    };
                    if small(ui, symbol, tooltip) {
                        page.diff_maximized = !page.diff_maximized;
                    }
                    if small(ui, icon::MINUS, tr("最小化")) {
                        page.diff_minimized = true;
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::horizontal()
                .id_salt("diff-tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, doc) in page.diffs.iter().enumerate() {
                            ui.push_id(&doc.key, |ui| {
                                ui.horizontal(|ui| {
                                    let response = ui
                                        .selectable_label(page.diff_active == i, &doc.name)
                                        .on_hover_text(format!("{}\n{}", doc.location, doc.key))
                                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                                    if response.clicked() {
                                        page.diff_active = i;
                                    }
                                    response.context_menu(|ui| {
                                        if ui.button(tr("关闭标签")).clicked() {
                                            close_tab = Some(i);
                                            ui.close();
                                        }
                                    });
                                    if small(ui, icon::X, tr("关闭标签")) {
                                        close_tab = Some(i);
                                    }
                                });
                            });
                        }
                    });
                });
            let doc = &page.diffs[page.diff_active];
            ui.add(
                egui::Label::new(
                    RichText::new(&doc.location)
                        .small()
                        .color(tone(self.light, MUTED)),
                )
                .truncate(),
            )
            .on_hover_text(&doc.location);
            ui.separator();
            egui::ScrollArea::both()
                .id_salt((&doc.key, "diff-lines"))
                .auto_shrink([false, false])
                .show_rows(
                    ui,
                    ui.text_style_height(&egui::TextStyle::Monospace) + 4.0,
                    doc.rows.len(),
                    |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for row in &doc.rows[range] {
                            let (fg, bg, sign) = match row.kind {
                                Kind::Added => (
                                    tone(self.light, RUNNING_GREEN),
                                    egui::Color32::from_rgba_unmultiplied(46, 160, 67, 30),
                                    "+",
                                ),
                                Kind::Removed => (
                                    tone(self.light, RED),
                                    egui::Color32::from_rgba_unmultiplied(248, 81, 73, 30),
                                    "−",
                                ),
                                Kind::Header => (
                                    tone(self.light, MUTED),
                                    egui::Color32::from_rgba_unmultiplied(56, 139, 253, 22),
                                    " ",
                                ),
                                Kind::Context => {
                                    (ui.visuals().text_color(), egui::Color32::TRANSPARENT, " ")
                                }
                            };
                            egui::Frame::new().fill(bg).show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    for n in [row.old, row.new] {
                                        ui.add_sized(
                                            [
                                                48.0,
                                                ui.text_style_height(&egui::TextStyle::Monospace)
                                                    + 4.0,
                                            ],
                                            egui::Label::new(
                                                RichText::new(
                                                    n.map(|n| n.to_string()).unwrap_or_default(),
                                                )
                                                .monospace()
                                                .color(tone(self.light, MUTED)),
                                            ),
                                        );
                                    }
                                    ui.label(RichText::new(sign).monospace().color(fg));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&row.text).monospace().color(fg),
                                        )
                                        .wrap_mode(egui::TextWrapMode::Extend)
                                        .selectable(true),
                                    );
                                });
                            });
                        }
                    },
                );
        });
        // Closing releases patch buffers; minimizing preserves them for the next file open.
        if close_window {
            page.diffs.clear();
            page.diff_active = 0;
            page.diff_open = false;
            page.diff_minimized = false;
            page.diff_maximized = false;
        } else if let Some(index) = close_tab {
            page.diffs.remove(index);
            if index < page.diff_active {
                page.diff_active -= 1;
            }
            page.diff_active = page.diff_active.min(page.diffs.len().saturating_sub(1));
            if page.diffs.is_empty() {
                page.diff_open = false;
                page.diff_minimized = false;
                page.diff_maximized = false;
            }
        }
    }
}
