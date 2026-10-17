//! Bounded terminal presentation metadata. ANSI is authoritative; plain-text severity uses exact prefixes.
use crate::console_text::{Ansi, Span, Style};
use serde::Serialize;
use std::ops::Range;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Output,
    Prompt,
    Input,
    Error,
    Warning,
    Success,
    Hint,
}
#[derive(Clone, Debug, Serialize)]
pub struct Segment {
    pub text: String,
    pub style: Style,
    pub role: Role,
}
struct Mark {
    range: Range<usize>,
    role: Role,
}
#[derive(Default)]
pub(super) struct Colors {
    ansi: Ansi,
    spans: Vec<Span>,
    roles: Vec<Mark>,
    prompt: Option<usize>,
    echo: Option<(usize, Vec<u8>)>,
    revision: u64,
    cache: Option<(u64, usize, Vec<Segment>)>,
}
impl Colors {
    pub fn sgr(&mut self, codes: &[u8]) {
        self.ansi
            .parse(&format!("\u{1b}[{}m", String::from_utf8_lossy(codes)));
    }
    pub fn push(&mut self, index: usize) {
        let style = self.ansi.current_style();
        if style != Style::default() {
            if let Some(last) = self.spans.last_mut()
                && last.range.end == index
                && last.style == style
            {
                last.range.end += 1;
            } else {
                self.spans.push(Span {
                    range: index..index + 1,
                    style,
                });
            }
            if self.spans.len() > 8192 {
                self.spans.drain(..self.spans.len() - 8192);
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn start_prompt(&mut self, index: usize) {
        self.prompt = Some(index);
    }
    pub fn expect_input(&mut self, index: usize, text: &str, has_prompt: bool) {
        if has_prompt
            && let Some(start) = self.prompt.take()
            && start < index
        {
            self.roles.push(Mark {
                range: start..index,
                role: Role::Prompt,
            });
        }
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(10);
        self.echo = Some((index, bytes));
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn fed(&mut self, bytes: &[u8]) {
        if let Some((start, expected)) = &self.echo {
            if *start > bytes.len() {
                self.echo = None;
                return;
            }
            let received = &bytes[*start..];
            let n = received.len().min(expected.len());
            if received[..n] != expected[..n] {
                self.echo = None;
            } else if received.len() >= expected.len() {
                self.roles.push(Mark {
                    range: *start..start + expected.len(),
                    role: Role::Input,
                });
                self.echo = None;
                self.revision = self.revision.wrapping_add(1);
            }
        }
        if self.roles.len() > 4096 {
            self.roles.drain(..self.roles.len() - 4096);
        }
    }
    pub fn truncate(&mut self, end: usize) {
        self.spans.retain_mut(|s| {
            s.range.end = s.range.end.min(end);
            s.range.start < s.range.end
        });
        self.roles.retain_mut(|s| {
            s.range.end = s.range.end.min(end);
            s.range.start < s.range.end
        });
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn trim(&mut self, n: usize) {
        self.spans.retain_mut(|s| {
            s.range = s.range.start.saturating_sub(n)..s.range.end.saturating_sub(n);
            s.range.start < s.range.end
        });
        self.roles.retain_mut(|s| {
            s.range = s.range.start.saturating_sub(n)..s.range.end.saturating_sub(n);
            s.range.start < s.range.end
        });
        self.prompt = self.prompt.map(|p| p.saturating_sub(n));
        if let Some((start, _)) = &mut self.echo {
            if *start < n {
                self.echo = None;
            } else {
                *start -= n;
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn clear(&mut self) {
        let ansi = std::mem::take(&mut self.ansi);
        *self = Self::default();
        self.ansi = ansi;
    }
    pub fn segments(&mut self, bytes: &[u8], end: usize) -> Vec<Segment> {
        let end = end.min(bytes.len());
        if let Some((revision, len, cache)) = &self.cache
            && *revision == self.revision
            && *len == end
        {
            return cache.clone();
        }
        let mut lines = vec![];
        let mut start = 0;
        for line in bytes[..end].split_inclusive(|b| *b == 10) {
            let text = String::from_utf8_lossy(line);
            lines.push(Mark {
                range: start..start + line.len(),
                role: plain_role(&text),
            });
            start += line.len();
        }
        let mut cuts = vec![0, end];
        for range in self
            .spans
            .iter()
            .map(|s| &s.range)
            .chain(self.roles.iter().map(|s| &s.range))
            .chain(lines.iter().map(|s| &s.range))
        {
            if range.start < end {
                cuts.push(range.start);
                cuts.push(range.end.min(end));
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        let (mut ansi, mut role, mut line) = (0, 0, 0);
        let mut result: Vec<Segment> = vec![];
        for pair in cuts.windows(2) {
            let p = pair[0];
            if p == pair[1] {
                continue;
            }
            while ansi < self.spans.len() && self.spans[ansi].range.end <= p {
                ansi += 1;
            }
            while role < self.roles.len() && self.roles[role].range.end <= p {
                role += 1;
            }
            while line < lines.len() && lines[line].range.end <= p {
                line += 1;
            }
            let style = self
                .spans
                .get(ansi)
                .filter(|s| s.range.contains(&p))
                .map_or(Style::default(), |s| s.style);
            let kind = self
                .roles
                .get(role)
                .filter(|s| s.range.contains(&p))
                .map(|s| s.role)
                .or_else(|| lines.get(line).map(|s| s.role))
                .unwrap_or(Role::Output);
            let text = String::from_utf8_lossy(&bytes[p..pair[1]]);
            if let Some(last) = result.last_mut()
                && last.style == style
                && last.role == kind
            {
                last.text.push_str(&text);
            } else {
                result.push(Segment {
                    text: text.into_owned(),
                    style,
                    role: kind,
                });
            }
        }
        self.cache = Some((self.revision, end, result.clone()));
        result
    }
}
fn plain_role(text: &str) -> Role {
    let s = text.trim_start();
    if ["error:", "fatal:", "[ERROR]", "[error]", "ERROR "]
        .iter()
        .any(|p| s.starts_with(p))
    {
        Role::Error
    } else if ["warning:", "[WARN]", "[warning]", "WARNING "]
        .iter()
        .any(|p| s.starts_with(p))
    {
        Role::Warning
    } else if ["[SUCCESS]", "SUCCESS ", "READY ", "Already up to date."]
        .iter()
        .any(|p| s.starts_with(p))
    {
        Role::Success
    } else if ["hint:", "[INFO]", "[info]"]
        .iter()
        .any(|p| s.starts_with(p))
    {
        Role::Hint
    } else {
        Role::Output
    }
}
