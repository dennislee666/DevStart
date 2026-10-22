//! Terminal interaction is line-oriented; the backend owns the persistent PTY and transcript.
use super::*;
#[derive(Default)]
pub(super) struct TerminalView {
    pub(super) session: Option<u64>,
    generation: u64,
    history_cursor: Option<usize>,
    focus: bool,
    command: String,
}
#[derive(Default)]
pub(super) struct Outcome {
    pub(super) completed: bool,
    pub(super) reopen: bool,
    pub(super) cleared: bool,
}
impl TerminalView {
    pub(super) fn request_focus(&mut self) {
        self.focus = true;
    }
    pub(super) fn attach(&mut self, id: u64) {
        if self.session != Some(id) {
            *self = Self {
                session: Some(id),
                focus: true,
                ..Default::default()
            };
        }
    }
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        manager: &mut crate::project_terminal::Manager,
        light: bool,
        extra: &str,
        height: f32,
        fill: bool,
    ) -> Result<Outcome, String> {
        let Some(id) = self.session else {
            return Ok(Outcome::default());
        };
        let snapshot = manager.read(id)?;
        let mut line = None;
        let mut interrupt = false;
        let mut clear = false;
        let mut restart = false;
        egui::Frame::new()
            .fill(if light {
                Color32::from_gray(245)
            } else {
                Color32::from_rgb(12, 13, 15)
            })
            .inner_margin(10.0)
            .corner_radius(4.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr("终端")).small().color(tone(light, MUTED)));
                    if snapshot.busy {
                        ui.spinner();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        clear = small(ui, icon::ERASER, tr("清空终端"));
                        ui.add_enabled_ui(snapshot.active, |ui| {
                            interrupt = small(ui, icon::STOP, tr("中断 · Ctrl+C"));
                        });
                    });
                });
                egui::ScrollArea::vertical()
                    .id_salt(("project-pty", id))
                    .max_height(height)
                    .auto_shrink([true, !fill])
                    .min_scrolled_height(if fill { height } else { 0.0 })
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if !extra.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(extra).monospace().color(tone(light, MUTED)),
                                )
                                .wrap()
                                .selectable(true),
                            );
                        }
                        ui.add(
                            egui::Label::new(terminal_job(ui, &snapshot.segments, light))
                                .wrap()
                                .selectable(true),
                        );
                    });
                if snapshot.active {
                    ui.horizontal(|ui| {
                        let prompt = &snapshot.prompt;
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let measured = ui
                            .painter()
                            .layout_no_wrap(
                                prompt.to_owned(),
                                egui::TextStyle::Monospace.resolve(ui.style()),
                                tone(light, RUNNING_GREEN),
                            )
                            .size()
                            .x;
                        let width = measured.min(ui.available_width() * 0.45);
                        ui.add_sized(
                            [width, ui.text_style_height(&egui::TextStyle::Monospace)],
                            egui::Label::new(
                                RichText::new(prompt)
                                    .monospace()
                                    .color(tone(light, RUNNING_GREEN)),
                            )
                            .truncate(),
                        )
                        .on_hover_text(prompt);
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.command)
                                .id_salt(("pty-input", id))
                                .font(egui::TextStyle::Monospace)
                                .text_color(tone(light, STARTING_BLUE))
                                .password(!snapshot.echo)
                                .frame(false)
                                .desired_width(ui.available_width()),
                        );
                        if self.focus {
                            response.request_focus();
                            self.focus = false;
                        }
                        if response.has_focus() {
                            interrupt |= ui
                                .input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::C));
                            if snapshot.echo && !snapshot.busy && !snapshot.history.is_empty() {
                                if ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                                }) {
                                    let index = self
                                        .history_cursor
                                        .unwrap_or(snapshot.history.len())
                                        .saturating_sub(1);
                                    self.history_cursor = Some(index);
                                    self.command = snapshot.history[index].clone();
                                }
                                if ui.input_mut(|i| {
                                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                                }) {
                                    let index = self
                                        .history_cursor
                                        .map_or(snapshot.history.len(), |i| i + 1);
                                    if index < snapshot.history.len() {
                                        self.history_cursor = Some(index);
                                        self.command = snapshot.history[index].clone();
                                    } else {
                                        self.history_cursor = None;
                                        self.command.clear();
                                    }
                                }
                            }
                        }
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            line = Some(std::mem::take(&mut self.command));
                            self.history_cursor = None;
                            response.request_focus();
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(tr("终端已退出"))
                                .small()
                                .color(tone(light, MUTED)),
                        );
                        restart = ui
                            .button(tr("重开终端"))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked();
                    });
                }
            });
        if let Some(line) = line {
            manager.line(id, &line)?;
        } else if interrupt {
            self.command.clear();
            manager.interrupt(id)?;
        } else if clear {
            manager.clear(id)?;
        }
        let refreshed = snapshot.generation != self.generation;
        let was_ready = self.generation > 0;
        self.generation = snapshot.generation;
        Ok(Outcome {
            completed: refreshed && was_ready && !snapshot.busy,
            reopen: restart,
            cleared: clear,
        })
    }
}
impl DevStart {
    pub(super) fn project_terminal(&mut self, ui: &mut egui::Ui) {
        let Some(service) = &mut self.service else {
            return;
        };
        let result = self.git_page.terminal.show(
            ui,
            &mut service.terminals,
            self.light,
            &self.git_page.output,
            260.0,
            false,
        );
        match result {
            Err(error) => self.report(Err(error)),
            Ok(outcome) => {
                if outcome.cleared {
                    self.git_page.output.clear();
                }
                if (outcome.reopen || outcome.completed)
                    && !self.git_request_pending()
                    && let Some(project) = self.git_page.selected
                {
                    self.git_select(project, 0);
                }
            }
        }
    }
}

/// Render backend-provided semantic/ANSI segments; user data is never classified by the UI.
fn terminal_job(
    ui: &egui::Ui,
    segments: &[crate::terminal_style::Segment],
    light: bool,
) -> egui::text::LayoutJob {
    use crate::terminal_style::Role;
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = ui.available_width();
    let mut palette = HashMap::new();
    for segment in segments {
        let base = match segment.role {
            Role::Output => ui.visuals().text_color(),
            Role::Prompt => tone(light, RUNNING_GREEN),
            Role::Input => tone(light, STARTING_BLUE),
            Role::Error => tone(light, RED),
            Role::Warning => {
                if light {
                    Color32::from_rgb(145, 99, 24)
                } else {
                    Color32::from_rgb(220, 183, 104)
                }
            }
            Role::Success => tone(light, RUNNING_GREEN),
            Role::Hint => tone(light, ACCENT),
        };
        let color = segment.style.color.map_or(base, |rgb| {
            *palette
                .entry(rgb)
                .or_insert_with(|| readable_ansi(rgb, light))
        });
        job.append(
            &segment.text,
            0.0,
            egui::TextFormat {
                font_id: egui::TextStyle::Monospace.resolve(ui.style()),
                color,
                italics: segment.style.italic,
                extra_letter_spacing: if segment.style.bold { 0.35 } else { 0.0 },
                underline: if segment.style.underline {
                    egui::Stroke::new(1.0_f32, color)
                } else {
                    egui::Stroke::NONE
                },
                ..Default::default()
            },
        );
    }
    job
}
/// Keep the producer's hue, adjusting only colors that disappear against the current terminal surface.
fn readable_ansi(rgb: [u8; 3], light: bool) -> Color32 {
    fn luminance(rgb: [u8; 3]) -> f32 {
        let c = |n: u8| {
            let v = f32::from(n) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * c(rgb[0]) + 0.7152 * c(rgb[1]) + 0.0722 * c(rgb[2])
    }
    let bg = luminance(if light { [245, 245, 245] } else { [12, 13, 15] });
    for step in 0..=20 {
        let alpha = step as f32 / 20.0;
        let target = if light { 0.0 } else { 255.0 };
        let value = rgb.map(|c| (f32::from(c) * (1.0 - alpha) + target * alpha).round() as u8);
        let fg = luminance(value);
        if (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05) >= 4.5 {
            return Color32::from_rgb(value[0], value[1], value[2]);
        }
    }
    if light {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}
