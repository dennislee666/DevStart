//! Bounded display cache; the independent raw log file remains complete.
use std::collections::VecDeque;
pub const MAX_LINES: usize = 3000;
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LINE_BYTES: usize = 16 * 1024;
pub const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_TOTAL_LINES: usize = 30_000;
#[derive(Default)]
pub struct ConsoleBuffer {
    lines: VecDeque<(bool, String)>,
    search: crate::console_search::Index,
    bytes: usize,
    style_bytes: usize,
    styles: VecDeque<Vec<crate::console_text::Span>>,
    ansi: [crate::console_text::Ansi; 2],
    pub revision: u64,
    pub dropped: usize,
    pub shortened: bool,
}
impl ConsoleBuffer {
    pub fn append(&mut self, error: bool, text: &str) {
        self.revision = self.revision.wrapping_add(1);
        for (style, text) in self.ansi[usize::from(error)].parse(text) {
            self.append_plain(error, &text, style);
        }
    }
    pub fn styles(&self, index: usize) -> &[crate::console_text::Span] {
        self.styles.get(index).map_or(&[], Vec::as_slice)
    }
    fn append_plain(&mut self, error: bool, text: &str, style: crate::console_text::Style) {
        self.search.dirty_from = self
            .search
            .dirty_from
            .min(self.lines.len().saturating_sub(1));
        for part in text.split_inclusive('\n') {
            if self.lines.back().is_none_or(|(_, s)| s.ends_with('\n')) {
                self.lines.push_back((error, String::new()));
                self.styles.push_back(Vec::new());
            }
            let (err, line) = self.lines.back_mut().expect("line created");
            self.bytes -= line.len();
            *err |= error;
            let offset = line.len();
            line.push_str(part);
            let spans = self.styles.back_mut().expect("line styles");
            self.style_bytes -= spans.len() * std::mem::size_of::<crate::console_text::Span>();
            if style != Default::default() && spans.len() < 64 {
                if let Some(last) = spans
                    .last_mut()
                    .filter(|s| s.style == style && s.range.end == offset)
                {
                    last.range.end = line.len();
                } else {
                    spans.push(crate::console_text::Span {
                        range: offset..line.len(),
                        style,
                    });
                }
            }
            if line.len() > MAX_LINE_BYTES {
                let mut start = line.len() - (MAX_LINE_BYTES - 4);
                while !line.is_char_boundary(start) {
                    start += 1;
                }
                *line = format!("… {}", &line[start..]);
                spans.clear();
                self.shortened = true;
            }
            self.style_bytes += spans.len() * std::mem::size_of::<crate::console_text::Span>();
            self.bytes += line.len();
            while self.lines.len() > MAX_LINES || self.bytes + self.style_bytes > MAX_BYTES {
                self.evict_oldest();
            }
        }
    }
    pub fn evict_oldest(&mut self) -> bool {
        if let Some((_, line)) = self.lines.pop_front() {
            if let Some(spans) = self.styles.pop_front() {
                self.style_bytes -= spans.len() * std::mem::size_of::<crate::console_text::Span>();
            }
            self.revision = self.revision.wrapping_add(1);
            self.bytes -= line.len();
            self.dropped += 1;
            true
        } else {
            false
        }
    }
    pub fn clear(&mut self) {
        self.search.invalidate();
        self.lines.clear();
        self.styles.clear();
        self.ansi = Default::default();
        self.revision = self.revision.wrapping_add(1);
        self.bytes = 0;
        self.style_bytes = 0;
        self.dropped = 0;
        self.shortened = false;
    }
    pub fn query(
        &mut self,
        query: &crate::console_search::Query,
        current: Option<usize>,
        direction: i32,
    ) -> Result<crate::console_search::View, String> {
        self.search
            .view(&self.lines, self.dropped, query, current, direction)
    }
    pub fn bytes(&self) -> usize {
        self.bytes + self.style_bytes
    }
    pub fn len(&self) -> usize {
        self.lines.len()
    }
    #[cfg(test)]
    pub fn iter(&self) -> impl Iterator<Item = &(bool, String)> {
        self.lines.iter()
    }
    pub fn get(&self, index: usize) -> Option<&(bool, String)> {
        self.lines.get(index)
    }
}
