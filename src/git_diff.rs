//! Presentation model for bounded Git unified patches. Parsing stays outside the UI.
use std::path::Path;
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Context,
    Added,
    Removed,
    Header,
}
pub struct Row {
    pub old: Option<usize>,
    pub new: Option<usize>,
    pub kind: Kind,
    pub text: String,
}
pub struct Document {
    pub key: String,
    pub name: String,
    pub location: String,
    pub rows: Vec<Row>,
}
impl Document {
    pub fn new(key: String, location: String, patch: String) -> Self {
        let name = Path::new(&location)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let (mut old, mut new, mut in_hunk) = (0, 0, false);
        let rows = patch
            .lines()
            .map(|line| {
                if line.starts_with("@@ ") {
                    let fields: Vec<_> = line.split_whitespace().collect();
                    let number = |i: usize| {
                        fields
                            .get(i)
                            .and_then(|s| s.get(1..))
                            .and_then(|s| s.split(',').next())
                            .and_then(|s| s.parse().ok())
                    };
                    if let (Some(a), Some(b)) = (number(1), number(2)) {
                        old = a;
                        new = b;
                        in_hunk = true;
                    }
                    return Row {
                        old: None,
                        new: None,
                        kind: Kind::Header,
                        text: line.into(),
                    };
                }
                let (a, b, kind, text) = if in_hunk && line.starts_with('+') {
                    let b = new;
                    new += 1;
                    (None, Some(b), Kind::Added, &line[1..])
                } else if in_hunk && line.starts_with('-') {
                    let a = old;
                    old += 1;
                    (Some(a), None, Kind::Removed, &line[1..])
                } else if in_hunk && line.starts_with(' ') {
                    let (a, b) = (old, new);
                    old += 1;
                    new += 1;
                    (Some(a), Some(b), Kind::Context, &line[1..])
                } else {
                    if line.starts_with("diff --git ") {
                        in_hunk = false;
                    }
                    (None, None, Kind::Header, line)
                };
                Row {
                    old: a,
                    new: b,
                    kind,
                    text: text.into(),
                }
            })
            .collect();
        Self {
            key,
            name,
            location,
            rows,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn line_numbers_follow_each_hunk() {
        let d=Document::new("a".into(),"/src/a.rs".into(),"--- a/a.rs\n+++ b/a.rs\n@@ -4,2 +4,2 @@\n-old\n+new\n same\n@@ -20 +30 @@\n-x\n+y\n\\ No newline at end of file".into());
        assert_eq!(d.name, "a.rs");
        assert_eq!(d.rows[3].old, Some(4));
        assert_eq!(d.rows[4].new, Some(4));
        assert_eq!(d.rows[5].new, Some(5));
        assert_eq!(d.rows[7].old, Some(20));
        assert_eq!(d.rows[8].new, Some(30));
        assert_eq!(d.rows[9].new, None);
    }
}
