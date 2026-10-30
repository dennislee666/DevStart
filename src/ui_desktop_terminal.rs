use super::*;
impl DevStart {
    pub(super) fn desktop_terminal_quit_guard(&mut self, ctx: &egui::Context) {
        if self.editor_quit_pending {
            return;
        }
        let Some(service) = self.service.as_mut() else {
            return;
        };
        let manager = &mut service.desktop_terminals;
        manager.poll();
        if manager.quit_ready {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) && !manager.sessions.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            manager.request_quit(false);
        }
        if let Some(e) = manager.close_error.take() {
            self.error = Some(e);
        }
        if manager.pending_quit {
            let mut confirm = false;
            let mut cancel = false;
            let response = egui::Modal::new(egui::Id::new("terminal-quit-confirm"))
                .frame(dialog_ui::frame(ctx))
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.heading(tr("停止命令并退出？"));
                    ui.label(tr("终端中有正在运行的命令，是否停止并关闭？"));
                    dialog_ui::actions(ui, |ui| {
                        cancel = ui.button(tr("取消")).clicked();
                        confirm = ui.button(tr("停止并关闭")).clicked();
                    });
                });
            if confirm {
                manager.pending_quit = false;
                manager.request_quit(true);
            } else if cancel || response.should_close() {
                manager.pending_quit = false;
            }
        }
    }
    pub(super) fn desktop_terminal_page(&mut self, ui: &mut egui::Ui) {
        let Some(service) = self.service.as_mut() else {
            return;
        };
        let manager = &mut service.desktop_terminals;
        manager.poll();
        if let Err(e) = manager.initialize(ui.ctx(), &service.store.root) {
            self.error = Some(e);
            return;
        }
        if let Some(e) = manager.close_error.take() {
            self.error = Some(e);
        }
        let before = (
            manager.selected,
            manager.palette,
            manager.tiled,
            manager.sessions.len(),
            manager.font_size,
            manager.background.clone(),
        );
        let mut close = if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::W)) {
            manager.selected
        } else {
            None
        };
        let mut create = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::T));
        ui.horizontal_wrapped(|ui| {
            create |= ui
                .button(button_label(
                    ui,
                    &format!("{} {}", icon::PLUS, tr("新建终端")),
                    true,
                ))
                .clicked();
            ui.selectable_value(
                &mut manager.tiled,
                false,
                button_label(
                    ui,
                    &format!("{} {}", icon::TERMINAL_WINDOW, tr("标签模式")),
                    true,
                ),
            );
            ui.selectable_value(
                &mut manager.tiled,
                true,
                button_label(
                    ui,
                    &format!("{} {}", icon::SQUARES_FOUR, tr("平铺模式")),
                    true,
                ),
            );
            ui.separator();
            ui.selectable_value(
                &mut manager.palette,
                crate::desktop_terminal::Palette::Mac,
                tr("Mac风格"),
            );
            ui.selectable_value(
                &mut manager.palette,
                crate::desktop_terminal::Palette::Linux,
                tr("Linux风格"),
            );
            ui.separator();
            if ui.button(tr("默认字号")).clicked() {
                manager.font_size = 13.0;
            }
            if ui.button(tr("+字号")).clicked() {
                manager.font_size = (manager.font_size + 1.0).min(32.0);
            }
            if ui.button(tr("−字号")).clicked() {
                manager.font_size = (manager.font_size - 1.0).max(10.0);
            }
            ui.label(format!("{} px", manager.font_size as u32));
            ui.separator();
            if ui
                .selectable_label(manager.background.is_none(), tr("跟随风格"))
                .clicked()
            {
                manager.background = None;
            }
            for &(color, name) in crate::desktop_terminal::BACKGROUNDS {
                let selected = manager.background.as_deref() == Some(color);
                let color32 = Color32::from_hex(color).unwrap();
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(egui_phosphor::fill::SQUARE)
                                .family(egui::FontFamily::Name("action-fill".into()))
                                .size(18.0)
                                .color(color32),
                        )
                        .fill(Color32::TRANSPARENT)
                        .stroke(egui::Stroke::new(
                            1.0,
                            if selected {
                                tone(self.light, ACCENT)
                            } else {
                                tone(self.light, BORDER)
                            },
                        ))
                        .min_size(Vec2::splat(26.0)),
                    )
                    .on_hover_text(tr(name))
                    .clicked()
                {
                    manager.background = Some(color.to_owned());
                }
            }
        });
        if !manager.tiled {
            egui::ScrollArea::horizontal()
                .id_salt("desktop-terminal-tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for session in &manager.sessions {
                            let selected = manager.selected == Some(session.id);
                            egui::Frame::new()
                                .fill(if selected {
                                    tone(self.light, SURFACE)
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .stroke(egui::Stroke::new(
                                    1.0,
                                    if selected {
                                        tone(self.light, ACCENT).gamma_multiply(0.55)
                                    } else {
                                        Color32::TRANSPARENT
                                    },
                                ))
                                .corner_radius(5.0)
                                .inner_margin(4)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new(&session.name).strong(),
                                                )
                                                .frame(false),
                                            )
                                            .clicked()
                                        {
                                            manager.selected = Some(session.id);
                                        }
                                        if small(ui, icon::X, tr("关闭终端")) {
                                            close = Some(session.id);
                                        }
                                    });
                                });
                        }
                    });
                });
        }
        if create {
            if let Err(e) = manager.open(ui.ctx()) {
                self.error = Some(e);
            }
        }
        if let Some(id) = close {
            manager.request_close(id, false);
        }
        ui.separator();
        let theme = manager.theme();
        let font = egui_term::TerminalFont::new(egui_term::FontSettings {
            font_type: egui::FontId::monospace(manager.font_size),
        });
        let light = self.light;
        if manager.tiled && !manager.sessions.is_empty() {
            let columns = ((ui.available_width() / 480.0).floor() as usize)
                .max(1)
                .min(manager.sessions.len());
            let rows = manager.sessions.len().div_ceil(columns);
            let width = (ui.available_width() - (columns - 1) as f32 * 12.0) / columns as f32;
            let height = (ui.available_height() / rows as f32 - 12.0).max(300.0);
            egui::ScrollArea::vertical()
                .id_salt("desktop-terminal-tiles")
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(12.0);
                    for row in manager.sessions.chunks_mut(columns) {
                        ui.horizontal_top(|ui| {
                            for session in row {
                                ui.allocate_ui_with_layout(
                                    Vec2::new(width, height),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        let selected = manager.selected == Some(session.id);
                                        egui::Frame::new()
                                            .fill(tone(light, SIDE))
                                            .stroke(egui::Stroke::new(
                                                1.0,
                                                if selected {
                                                    tone(light, ACCENT)
                                                } else {
                                                    Color32::TRANSPARENT
                                                },
                                            ))
                                            .inner_margin(8)
                                            .show(ui, |ui| {
                                                ui.set_width((width - 16.0).max(80.0));
                                                ui.horizontal(|ui| {
                                                    if ui
                                                        .add(
                                                            egui::Button::new(&session.name)
                                                                .frame(false),
                                                        )
                                                        .clicked()
                                                    {
                                                        manager.selected = Some(session.id);
                                                    }
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            if small(ui, icon::X, tr("关闭终端"))
                                                            {
                                                                close = Some(session.id);
                                                            }
                                                            if small(
                                                                ui,
                                                                icon::CORNERS_OUT,
                                                                tr("标签模式"),
                                                            ) {
                                                                manager.selected = Some(session.id);
                                                                manager.tiled = false;
                                                            }
                                                        },
                                                    );
                                                });
                                                let size = Vec2::new(
                                                    ui.available_width(),
                                                    (height - 60.0).max(100.0),
                                                );
                                                let view = egui_term::TerminalView::new(
                                                    ui,
                                                    &mut session.backend,
                                                )
                                                .set_theme(theme.clone())
                                                .set_font(font.clone())
                                                .set_focus(selected)
                                                .set_size(size);
                                                let response = ui.add(view);
                                                if response.clicked() {
                                                    manager.selected = Some(session.id);
                                                }
                                            });
                                    },
                                );
                            }
                        });
                    }
                });
            if let Some(id) = close {
                manager.request_close(id, false);
            }
        } else if let Some(session) = manager
            .sessions
            .iter_mut()
            .find(|s| Some(s.id) == manager.selected)
        {
            if session.exited {
                ui.label(tr("终端已退出，可关闭此标签或新建终端。"));
            }
            let size = ui.available_size();
            let view = egui_term::TerminalView::new(ui, &mut session.backend)
                .set_theme(theme)
                .set_font(font)
                .set_focus(true)
                .set_size(size);
            ui.add(view);
        }
        if let Some(id) = manager.pending_close {
            let mut confirm = false;
            let mut cancel = false;
            let modal = egui::Modal::new(egui::Id::new("terminal-close-confirm"))
                .frame(dialog_ui::frame(ui.ctx()))
                .show(ui.ctx(), |ui| {
                    dialog_ui::content(ui);
                    ui.heading(tr("停止并关闭终端？"));
                    ui.label(tr("终端中有正在运行的命令，是否停止并关闭？"));
                    if let Some(session) = manager.sessions.iter().find(|s| s.id == id) {
                        ui.label(&session.name);
                    }
                    dialog_ui::actions(ui, |ui| {
                        cancel = ui.button(tr("取消")).clicked();
                        confirm = ui.button(tr("停止并关闭")).clicked();
                    });
                });
            if confirm {
                manager.pending_close = None;
                manager.request_close(id, true);
            } else if cancel || modal.should_close() {
                manager.pending_close = None;
            }
        }
        if before
            != (
                manager.selected,
                manager.palette,
                manager.tiled,
                manager.sessions.len(),
                manager.font_size,
                manager.background.clone(),
            )
            || manager.dirty
        {
            if let Err(e) = manager.save(&service.store.root) {
                self.error = Some(e);
            }
        }
    }
}
