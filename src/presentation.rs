use crate::runtime::State;
use unicode_segmentation::UnicodeSegmentation;

pub struct RunTab {
    pub id: u64,
    pub label: String,
    pub full_name: String,
    pub tooltip: String,
    pub state: State,
}
/// Visible graphemes, including the three dots, never exceed twelve characters.
pub fn tab_label(full: &str) -> String {
    let chars: Vec<_> = full.graphemes(true).collect();
    if chars.len() <= 12 {
        full.into()
    } else {
        format!("{}...", chars[..9].concat())
    }
}

/// Bound presentation text without truncating UTF-8; execution argv stays intact.
pub fn command_preview(command: &str) -> String {
    let mut chars = command.chars();
    let preview: String = chars.by_ref().take(100).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

pub fn run_tooltip(name: &str, command: &str, project_path: &str) -> String {
    format!("{name}\n{}\n{project_path}", command_preview(command))
}

pub struct PortCell {
    pub label: String,
    pub tooltip: String,
}

/// Bound list labels while preserving complete source and alias metadata.
pub fn launch_label(full: &str) -> String {
    let mut graphemes = full.graphemes(true);
    let head: String = graphemes.by_ref().take(47).collect();
    if full.graphemes(true).count() > 50 {
        format!("{head}...")
    } else {
        full.to_owned()
    }
}
