use super::*;
impl DevStart {
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
        let before = (manager.selected, manager.palette, manager.sessions.len());
        let mut close = if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::W)) {
            manager.selected
        } else {
            None
        };
        let mut create = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::T));
        ui.horizontal(|ui| {
            create |= ui
                .button(format!("{} {}", icon::PLUS, tr("新建终端")))
                .clicked();
            ui.separator();
            ui.selectable_value(
                &mut manager.palette,
                crate::desktop_terminal::Palette::Mac,
                "mac",
            );
            ui.selectable_value(
                &mut manager.palette,
                crate::desktop_terminal::Palette::Linux,
                "Linux 彩色",
            );
        });
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
        if create {
            if let Err(e) = manager.open(ui.ctx()) {
                self.error = Some(e);
            }
        }
        if let Some(id) = close {
            manager.close(id);
        }
        if before != (manager.selected, manager.palette, manager.sessions.len()) {
            if let Err(e) = manager.save(&service.store.root) {
                self.error = Some(e);
            }
        }
        ui.separator();
        let theme = manager.theme();
        if let Some(session) = manager
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
                .set_focus(true)
                .set_size(size);
            ui.add(view);
        }
    }
}
