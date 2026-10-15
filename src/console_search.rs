//! Literal log queries and bounded row indexes, independent of UI and file logging.
use serde::{Deserialize, Serialize};
use std::ops::Range;
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub keyword: String,
    pub case_sensitive: bool,
    pub only_matches: bool,
}
impl Query {
    pub fn validate(&self) -> Result<(), String> {
        if self.keyword.chars().count() > 256 {
            return Err("搜索关键词最多256个字符".into());
        }
        Ok(())
    }
}
#[derive(Default)]
pub struct Index {
    query: Option<Query>,
    pub dirty_from: usize,
    dropped: usize,
    hits: Vec<usize>,
    needle: String,
}
pub struct View {
    pub rows: Vec<usize>,
    pub matched_lines: usize,
    pub current: Option<usize>,
    pub active_row: Option<usize>,
    pub active_visible_row: Option<usize>,
}
impl Index {
    pub fn invalidate(&mut self) {
        self.query = None;
        self.hits.clear();
        self.dirty_from = 0;
    }
    pub fn view(
        &mut self,
        lines: &std::collections::VecDeque<(bool, String)>,
        dropped: usize,
        query: &Query,
        current: Option<usize>,
        direction: i32,
    ) -> Result<View, String> {
        query.validate()?;
        let changed = self.query.as_ref().is_none_or(|old| {
            old.keyword != query.keyword || old.case_sensitive != query.case_sensitive
        });
        if changed || self.dropped != dropped {
            self.hits.clear();
            self.dirty_from = 0;
            self.needle = if query.case_sensitive {
                query.keyword.clone()
            } else {
                query.keyword.to_lowercase()
            };
        }
        let from = self.dirty_from.min(lines.len());
        self.hits.retain(|row| *row < from);
        if !self.needle.is_empty() {
            for (row, (_, text)) in lines.iter().enumerate().skip(from) {
                if contains(text, &self.needle, query.case_sensitive) {
                    self.hits.push(row);
                }
            }
        }
        self.dirty_from = lines.len();
        self.dropped = dropped;
        self.query = Some(query.clone());
        let len = self.hits.len();
        let current = if len == 0 {
            None
        } else {
            let position = current
                .unwrap_or(if direction < 0 { 0 } else { len - 1 })
                .min(len - 1);
            Some(if direction < 0 {
                (position + len - 1) % len
            } else if direction > 0 {
                (position + 1) % len
            } else {
                current.unwrap_or(0).min(len - 1)
            })
        };
        let filtered = query.only_matches && !query.keyword.is_empty();
        let rows = if filtered {
            self.hits.clone()
        } else {
            (0..lines.len()).collect()
        };
        let active_row = current.map(|c| self.hits[c]);
        let active_visible_row = if filtered { current } else { active_row };
        Ok(View {
            rows,
            matched_lines: len,
            current,
            active_row,
            active_visible_row,
        })
    }
}
fn contains(text: &str, needle: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        text.contains(needle)
    } else {
        text.to_lowercase().contains(needle)
    }
}
/// Highlight only visible lines, with a fixed span budget. All ranges refer to original UTF-8 bytes.
pub fn spans(text: &str, query: &Query) -> Vec<Range<usize>> {
    if query.keyword.is_empty() {
        return vec![];
    }
    if query.case_sensitive {
        return text
            .match_indices(&query.keyword)
            .take(64)
            .map(|(i, s)| i..i + s.len())
            .collect();
    }
    let needle = query.keyword.to_lowercase();
    if text.is_ascii() {
        return text
            .to_ascii_lowercase()
            .match_indices(&needle)
            .take(64)
            .map(|(i, s)| i..i + s.len())
            .collect();
    }
    let mut folded = String::new();
    let mut mapping = Vec::new();
    for (start, ch) in text.char_indices() {
        let from = folded.len();
        for lower in ch.to_lowercase() {
            folded.push(lower);
        }
        mapping.push((from, folded.len(), start, start + ch.len_utf8()));
    }
    let mut result: Vec<Range<usize>> = vec![];
    for (from, value) in folded.match_indices(&needle).take(64) {
        let end = from + value.len();
        let first = mapping.partition_point(|(_, to, _, _)| *to <= from);
        let last = mapping
            .partition_point(|(start, _, _, _)| *start < end)
            .saturating_sub(1);
        if first < mapping.len() && last < mapping.len() {
            let range = mapping[first].2..mapping[last].3;
            if let Some(previous) = result.last_mut()
                && previous.end >= range.start
            {
                previous.end = previous.end.max(range.end);
            } else {
                result.push(range);
            }
        }
    }
    result
}
