//! Shared soft-wrapped, virtual console renderer for tabs, tiles and detached windows.
use super::*;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};
#[derive(Default)]
pub(super) struct LayoutCache {
    key: String,
    revision: u64,
    rows: Vec<(u64, f32)>,
    offsets: Vec<f32>,
}
#[allow(clippy::too_many_arguments)]
fn job(
    text: &str,
    err: bool,
    styles: &[crate::console_text::Span],
    search: &search_ui::SearchUi,
    active: bool,
    font: f32,
    width: f32,
    light: bool,
) -> egui::text::LayoutJob {
    let matches = crate::console_search::spans(text, &search.query);
    let base = if err || text.contains("ERROR") || text.contains("[error]") {
        tone(light, RED)
    } else if text.contains("WARN") || text.contains("[warn]") {
        Color32::from_rgb(193, 154, 78)
    } else if light {
        Color32::from_gray(45)
    } else {
        crate::appearance::TEXT
    };
    let mut job = search_ui::job(
        text,
        &matches,
        egui::FontId::monospace(font),
        base,
        active,
        light,
    );
    let links = crate::console_text::links(text);
    // Split existing search sections at style/link boundaries, preserving search backgrounds.
    let mut sections = Vec::new();
    for section in &job.sections {
        let mut cuts = vec![section.byte_range.start, section.byte_range.end];
        for range in styles
            .iter()
            .map(|s| &s.range)
            .chain(links.iter().map(|l| &l.range))
        {
            for p in [range.start, range.end] {
                if section.byte_range.contains(&p) {
                    cuts.push(p);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for pair in cuts.windows(2) {
            let mut format = section.format.clone();
            if let Some(style) = styles
                .iter()
                .find(|s| s.range.contains(&pair[0]))
                .map(|s| s.style)
            {
                if let Some([r, g, b]) = style.color {
                    format.color = Color32::from_rgb(r, g, b);
                }
                format.italics = style.italic;
                if style.bold {
                    format.extra_letter_spacing = 0.35;
                }
                if style.underline {
                    format.underline = egui::Stroke::new(1.0_f32, format.color);
                }
            }
            if links.iter().any(|l| l.range.contains(&pair[0])) {
                format.color = if light {
                    Color32::from_rgb(30, 110, 145)
                } else {
                    Color32::from_rgb(112, 190, 206)
                };
                format.underline = egui::Stroke::new(1.0_f32, format.color.gamma_multiply(0.55));
            }
            sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: pair[0]..pair[1],
                format,
            });
        }
    }
    job.sections = sections;
    job.wrap.max_width = width.max(1.0);
    job.wrap.break_anywhere = true;
    job
}
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    run: &crate::runtime::Run,
    view: &crate::console_search::View,
    search: &search_ui::SearchUi,
    cache: &mut LayoutCache,
    prefs: &Preferences,
    light: bool,
    follow: bool,
    jump: bool,
) -> Option<String> {
    let width = (ui.available_width() - ui.spacing().scroll.bar_width - 8.0).max(1.0);
    let key = format!(
        "{width}:{}:{}:{light}:{:?}:{}:{:?}:{:?}",
        prefs.console_font_size,
        prefs.console_line_height,
        search.query,
        search.open,
        view.active_row,
        prefs.console_font
    );
    let reset = cache.key != key;
    if reset || cache.revision != run.lines.revision || cache.rows.len() != view.rows.len() {
        cache.offsets.clear();
        cache.offsets.push(0.0);
        let gap = prefs.console_font_size * (prefs.console_line_height - 1.0);
        for (row, &index) in view.rows.iter().enumerate() {
            if let Some((err, text)) = run.lines.get(index) {
                let mut hash = DefaultHasher::new();
                text.hash(&mut hash);
                err.hash(&mut hash);
                for span in run.lines.styles(index) {
                    span.range.hash(&mut hash);
                    span.style.hash(&mut hash);
                }
                let signature = hash.finish();
                let height = if !reset && cache.rows.get(row).is_some_and(|r| r.0 == signature) {
                    cache.rows[row].1
                } else {
                    let job = job(
                        text.trim_end_matches('\n'),
                        *err,
                        run.lines.styles(index),
                        search,
                        search.open && view.active_row == Some(index),
                        prefs.console_font_size,
                        width,
                        light,
                    );
                    ui.fonts_mut(|f| f.layout_job(job))
                        .size()
                        .y
                        .max(prefs.console_font_size)
                        + gap
                };
                if row < cache.rows.len() {
                    cache.rows[row] = (signature, height);
                } else {
                    cache.rows.push((signature, height));
                }
                cache
                    .offsets
                    .push(cache.offsets.last().copied().unwrap_or(0.0) + height);
            }
        }
        cache.rows.truncate(view.rows.len());
        cache.key = key;
        cache.revision = run.lines.revision;
    }
    let total = cache.offsets.last().copied().unwrap_or(0.0);
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("console", run.id))
        .auto_shrink([false, false])
        .stick_to_bottom(follow && !jump);
    if jump && let Some(row) = view.active_visible_row {
        scroll = scroll.vertical_scroll_offset(
            (cache.offsets.get(row).copied().unwrap_or(0.0) - ui.available_height() / 3.0).max(0.0),
        );
    }
    let mut target = None;
    scroll.show_viewport(ui, |ui, viewport| {
        ui.set_min_size(egui::vec2(width, total));
        let start = cache
            .offsets
            .partition_point(|y| *y < viewport.min.y)
            .saturating_sub(1);
        let end = cache
            .offsets
            .partition_point(|y| *y <= viewport.max.y)
            .min(view.rows.len());
        let origin = ui.min_rect().min;
        for row in start..end {
            let index = view.rows[row];
            if let Some((err, text)) = run.lines.get(index) {
                let text = text.trim_end_matches('\n');
                let job = job(
                    text,
                    *err,
                    run.lines.styles(index),
                    search,
                    search.open && view.active_row == Some(index),
                    prefs.console_font_size,
                    width,
                    light,
                );
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let rect = egui::Rect::from_min_size(
                    origin + egui::vec2(0.0, cache.offsets[row]),
                    egui::vec2(width, cache.rows[row].1),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    let response = ui.add(
                        egui::Label::new(galley.clone())
                            .selectable(true)
                            .sense(egui::Sense::click_and_drag()),
                    );
                    if response.hovered()
                        && let Some(pos) = response.hover_pos()
                    {
                        let cursor = galley.cursor_from_pos(pos - response.rect.min);
                        let byte = text
                            .char_indices()
                            .nth(cursor.index)
                            .map_or(text.len(), |(i, _)| i);
                        if let Some(link) = crate::console_text::links(text)
                            .into_iter()
                            .find(|l| l.range.contains(&byte))
                        {
                            response
                                .clone()
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .on_hover_text(tf(
                                    "Control + 点击打开\n{0}",
                                    &[(
                                        "0",
                                        (crate::presentation::command_preview(&link.target))
                                            .to_string(),
                                    )],
                                ));
                            if ui.input(|i| i.modifiers.ctrl)
                                && (response.clicked() || response.secondary_clicked())
                            {
                                target = Some(link.target);
                            }
                        }
                    }
                });
            }
        }
    });
    target
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_console_lines_wrap_without_eliding_search_or_link_text() {
        let context = egui::Context::default();
        let _ = context.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let search = search_ui::SearchUi::default();
                let text = format!("https://localhost:8082/{}", "中a".repeat(120));
                let layout = job(&text, false, &[], &search, false, 12.0, 150.0, false);
                let galley = ui.fonts_mut(|f| f.layout_job(layout));
                assert!(galley.rows.len() > 5);
                assert!(galley.size().x <= 150.1);
                assert!(!galley.elided);
                assert_eq!(galley.text(), text);
            });
        });
    }
}
