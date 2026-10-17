//! Shared visual tokens and typography. Process and configuration logic live elsewhere.
use crate::preferences::{ConsoleFont, Density, Preferences, Theme, UiFont};
use eframe::egui::{self, Color32, FontFamily, FontId, Vec2};
pub const BG: Color32 = Color32::from_rgb(16, 17, 18);
pub const SIDE: Color32 = Color32::from_rgb(21, 22, 23);
pub const SURFACE: Color32 = Color32::from_rgb(27, 28, 30);
pub const BORDER: Color32 = Color32::from_rgb(44, 46, 49);
pub const MUTED: Color32 = Color32::from_rgb(139, 144, 153);
pub const TEXT: Color32 = Color32::from_rgb(218, 221, 226);
pub const ACCENT: Color32 = Color32::from_rgb(148, 191, 208);
pub const GREEN: Color32 = Color32::from_rgb(129, 201, 174);
/// Lifecycle dots use semantic colors independent of the theme accent.
pub const STARTING_BLUE: Color32 = Color32::from_rgb(96, 165, 250);
pub const RUNNING_GREEN: Color32 = Color32::from_rgb(74, 201, 120);
pub const RED: Color32 = Color32::from_rgb(229, 137, 140);
pub fn light(p: &Preferences, system: Option<egui::Theme>) -> bool {
    p.theme == Theme::Light || p.theme == Theme::System && system == Some(egui::Theme::Light)
}
pub fn fonts(p: &Preferences) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    let mut load = |name: &str, paths: &[&str]| -> Option<String> {
        for path in paths {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert(name.into(), egui::FontData::from_owned(bytes).into());
                return Some(name.into());
            }
        }
        None
    };
    let system = load(
        "system-ui",
        &[
            "/System/Library/Fonts/SFNS.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ],
    );
    let mono = load(
        "system-mono",
        &[
            "/System/Library/Fonts/SFNSMono.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        ],
    );
    let menlo = load("menlo", &["/System/Library/Fonts/Menlo.ttc"]);
    let monaco = load("monaco", &["/System/Library/Fonts/Monaco.ttf"]);
    let cjk = load(
        "cjk",
        &[
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        ],
    );
    if let Some(name) = system {
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, name);
    }
    if let Some(name) = mono {
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, name);
    }
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(name) = &cjk {
            fonts.families.entry(family).or_default().push(name.clone());
        }
    }
    let base_mono = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    fonts
        .families
        .insert(FontFamily::Name("preview-system".into()), base_mono.clone());
    for (key, name) in [("preview-menlo", &menlo), ("preview-monaco", &monaco)] {
        let mut family = base_mono.clone();
        if let Some(name) = name {
            family.insert(0, name.clone());
        }
        fonts.families.insert(FontFamily::Name(key.into()), family);
    }
    let selected = match p.console_font {
        ConsoleFont::System => None,
        ConsoleFont::Menlo => menlo,
        ConsoleFont::Monaco => monaco,
    };
    if let Some(name) = selected {
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, name);
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    fonts.font_data.insert(
        "phosphor-fill".into(),
        egui_phosphor::Variant::Fill.font_data().into(),
    );
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (family, font) in [
        ("action-fill", "phosphor-fill"),
        ("action-outline", "phosphor"),
    ] {
        let mut names = vec![font.to_owned()];
        names.extend(
            fallback
                .iter()
                .filter(|name| name.as_str() != font)
                .cloned(),
        );
        fonts
            .families
            .insert(FontFamily::Name(family.into()), names);
    }
    fonts
}
pub fn style(p: &Preferences, light: bool) -> egui::Style {
    let mut style = egui::Style {
        visuals: if light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        },
        ..Default::default()
    };
    let border = if light {
        Color32::from_gray(217)
    } else {
        BORDER
    };
    let text = if light { Color32::from_gray(42) } else { TEXT };
    let hover = if light {
        Color32::from_gray(230)
    } else {
        Color32::from_rgb(39, 41, 44)
    };
    style.visuals.panel_fill = if light { Color32::from_gray(250) } else { BG };
    style.visuals.window_fill = if light { Color32::from_gray(247) } else { SIDE };
    style.visuals.extreme_bg_color = if light { Color32::WHITE } else { BG };
    style.visuals.faint_bg_color = if light { Color32::from_gray(243) } else { SIDE };
    style.visuals.override_text_color = Some(text);
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, border);
    style.visuals.widgets.inactive.bg_fill = if light {
        Color32::from_gray(243)
    } else {
        SURFACE
    };
    style.visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, border);
    for widget in [
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.weak_bg_fill = hover;
        widget.bg_fill = hover;
        widget.bg_stroke = egui::Stroke::new(1.0_f32, border);
    }
    style.visuals.selection.bg_fill = if light {
        Color32::from_rgb(218, 231, 226)
    } else {
        Color32::from_rgb(44, 49, 48)
    };
    style.visuals.selection.stroke = egui::Stroke::new(1.0_f32, text);
    style.visuals.window_stroke = egui::Stroke::new(1.0_f32, border);
    style.visuals.window_corner_radius = egui::CornerRadius::same(6);
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(4);
    }
    style.spacing.item_spacing = Vec2::new(
        8.0,
        if p.density == Density::Compact {
            4.0
        } else {
            8.0
        },
    );
    style.spacing.button_padding = Vec2::new(7.0, 4.0);
    style.spacing.interact_size = Vec2::new(24.0, p.row_height());
    // Apply one delay to all hover helpers, without extra velocity smoothing or instant switching.
    style.interaction.tooltip_delay = 0.3;
    style.interaction.tooltip_grace_time = 0.0;
    style.interaction.show_tooltips_only_when_still = false;
    let family = if p.ui_font == UiFont::System {
        FontFamily::Proportional
    } else {
        FontFamily::Monospace
    };
    for text_style in [egui::TextStyle::Body, egui::TextStyle::Button] {
        style
            .text_styles
            .insert(text_style, FontId::new(p.ui_font_size, family.clone()));
    }
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(p.ui_font_size + 4.0, family.clone()),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        FontId::new((p.ui_font_size - 1.0).max(10.0), family),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        FontId::monospace(p.console_font_size),
    );
    style
}
