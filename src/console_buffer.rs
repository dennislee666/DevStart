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
    pub dropped: usize,
    pub shortened: bool,
}
impl ConsoleBuffer {
    pub fn append(&mut self, error: bool, text: &str) {
        self.search.dirty_from = self
            .search
            .dirty_from
            .min(self.lines.len().saturating_sub(1));
        for part in text.split_inclusive('\n') {
            if self.lines.back().is_none_or(|(_, s)| s.ends_with('\n')) {
                self.lines.push_back((error, String::new()));
            }
            let (err, line) = self.lines.back_mut().expect("line created");
            self.bytes -= line.len();
            *err |= error;
            line.push_str(part);
            if line.len() > MAX_LINE_BYTES {
                let mut start = line.len() - (MAX_LINE_BYTES - 4);
                while !line.is_char_boundary(start) {
                    start += 1;
                }
                *line = format!("… {}", &line[start..]);
                self.shortened = true;
            }
            self.bytes += line.len();
            while self.lines.len() > MAX_LINES || self.bytes > MAX_BYTES {
                self.evict_oldest();
            }
        }
    }
    pub fn evict_oldest(&mut self) -> bool {
        if let Some((_, line)) = self.lines.pop_front() {
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
        self.bytes = 0;
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
        self.bytes
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
