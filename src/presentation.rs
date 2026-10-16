use crate::runtime::State;
use unicode_segmentation::UnicodeSegmentation;

pub struct RunTab {
    pub console_id: u64,
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

/// Display actual monitored listeners only; configured ports do not prove a live listener.
pub fn run_state_display(run: &crate::runtime::Run) -> String {
    run_state_display_for_locale(run, crate::i18n::Locale::Chinese)
}
pub fn run_state_display_for_locale(
    run: &crate::runtime::Run,
    locale: crate::i18n::Locale,
) -> String {
    if run.state != State::Running {
        return locale.text(run.state.label()).into();
    }
    let ports: std::collections::BTreeSet<_> = run.listening_ports.iter().copied().collect();
    let ports = if ports.is_empty() {
        "--".into()
    } else {
        ports
            .into_iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    if locale == crate::i18n::Locale::Chinese {
        format!("{}：{ports}", run.state.label())
    } else {
        format!("{}: {ports}", locale.text(run.state.label()))
    }
}
