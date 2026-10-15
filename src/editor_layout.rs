//! Code-editor presentation geometry. Soft wrapping never changes source or logical numbering.
use eframe::egui::{Galley, text::LayoutJob};

pub fn wrap(job: &mut LayoutJob, width: f32) {
    job.wrap.max_width = width.max(1.0);
    job.wrap.break_anywhere = true;
}
pub fn gutter_digits(text: &str) -> usize {
    text.split('\n').count().to_string().len().max(2)
}
/// Position one number on the first visual row of each source line, including empty lines.
pub fn line_numbers(galley: &Galley) -> Vec<(usize, f32)> {
    let mut number = 1;
    let mut first = true;
    let mut rows = Vec::new();
    for row in &galley.rows {
        if first {
            rows.push((number, row.pos.y));
        }
        first = row.ends_with_newline;
        if first {
            number += 1;
        }
    }
    rows
}
