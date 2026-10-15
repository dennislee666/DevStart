//! Console search controls and highlight formatting; matching/indexing is backend-owned.
use super::*;
#[derive(Default)]
pub(super) struct SearchUi {
    pub open: bool,
    pub focus: bool,
    pub query: crate::console_search::Query,
    pub current: Option<usize>,
    pub matched: usize,
}
impl SearchUi {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.focus = self.open;
        if !self.open {
            self.query = Default::default();
            self.current = None;
            self.matched = 0;
        }
    }
}
#[derive(Default)]
pub(super) struct Input {
    pub changed: bool,
    pub direction: i32,
}
pub(super) fn controls(ui: &mut egui::Ui, state: &mut SearchUi, id: u64) -> Input {
    let mut input = Input::default();
    if !state.open {
        return input;
    }
    let previous = state.query.clone();
    ui.horizontal_wrapped(|ui| {
        let width = (ui.available_width() - 300.0).clamp(90.0, 400.0);
        let response = ui
            .add(
                egui::TextEdit::singleline(&mut state.query.keyword)
                    .id_salt(("console-find", id))
                    .hint_text("搜索日志…")
                    .char_limit(256)
                    .desired_width(width),
            )
            .on_hover_text("在当前保留的最近日志中搜索，按 Enter / Shift+Enter 跳转");
        if state.focus {
            response.request_focus();
            state.focus = false;
        }
        if (response.has_focus() || response.lost_focus())
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            input.direction = if ui.input(|i| i.modifiers.shift) {
                -1
            } else {
                1
            };
            response.request_focus();
        }
        if ui
            .selectable_label(state.query.case_sensitive, "Aa")
            .on_hover_text("区分大小写")
            .clicked()
        {
            state.query.case_sensitive = !state.query.case_sensitive;
        }
        if ui
            .selectable_label(
                state.query.only_matches,
                format!("{} 只看匹配", icon::FUNNEL),
            )
            .on_hover_text("仅显示包含关键词的日志行")
            .clicked()
        {
            state.query.only_matches = !state.query.only_matches;
        }
        if action_button(
            ui,
            state.matched > 0,
            icon::CARET_UP,
            "上一处 · Shift+Enter",
            None,
        )
        .clicked()
        {
            input.direction = -1;
        }
        if action_button(
            ui,
            state.matched > 0,
            icon::CARET_DOWN,
            "下一处 · Enter",
            None,
        )
        .clicked()
        {
            input.direction = 1;
        }
        ui.label(
            RichText::new(if state.matched == 0 {
                "0 行".into()
            } else if state.query.only_matches {
                format!("{} 行", state.matched)
            } else {
                format!(
                    "{}/{} 行",
                    state.current.map_or(0, |v| v + 1),
                    state.matched
                )
            })
            .small(),
        );
        if small(ui, icon::X, "关闭搜索，恢复完整视图")
            || response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            state.toggle();
        }
    });
    input.changed = previous != state.query;
    if input.changed {
        state.current = None;
    }
    input
}
pub(super) fn job(
    text: &str,
    spans: &[std::ops::Range<usize>],
    font: egui::FontId,
    color: Color32,
    active: bool,
    light: bool,
) -> egui::text::LayoutJob {
    let mut result = egui::text::LayoutJob::default();
    let base = egui::TextFormat {
        font_id: font,
        color,
        background: if active {
            if light {
                Color32::from_gray(236)
            } else {
                SURFACE
            }
        } else {
            Color32::TRANSPARENT
        },
        ..Default::default()
    };
    let highlight = egui::TextFormat {
        background: if light {
            Color32::from_rgb(255, 231, 153)
        } else {
            Color32::from_rgb(91, 73, 29)
        },
        ..base.clone()
    };
    let mut from = 0;
    for range in spans {
        result.append(&text[from..range.start], 0.0, base.clone());
        result.append(&text[range.clone()], 0.0, highlight.clone());
        from = range.end;
    }
    result.append(&text[from..], 0.0, base);
    result
}
