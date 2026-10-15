//! Typed appearance preferences; persistence and validation stay in the service layer.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Dark,
    Light,
    System,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiFont {
    System,
    Monospace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleFont {
    System,
    Menlo,
    Monaco,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    Comfortable,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub theme: Theme,
    pub ui_font: UiFont,
    pub ui_font_size: f32,
    pub console_font: ConsoleFont,
    pub console_font_size: f32,
    pub console_line_height: f32,
    pub density: Density,
    pub sidebar_collapsed: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            ui_font: UiFont::System,
            ui_font_size: 12.0,
            console_font: ConsoleFont::System,
            console_font_size: 12.0,
            console_line_height: 1.25,
            density: Density::Compact,
            sidebar_collapsed: false,
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value, min, max) in [
            ("界面字号", self.ui_font_size, 10.0, 18.0),
            ("Console字号", self.console_font_size, 10.0, 24.0),
            ("Console行距", self.console_line_height, 1.0, 1.8),
        ] {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(format!("{name}需在{min}–{max}之间"));
            }
        }
        Ok(())
    }
    pub fn row_height(&self) -> f32 {
        self.ui_font_size
            + if self.density == Density::Compact {
                14.0
            } else {
                22.0
            }
    }
}
