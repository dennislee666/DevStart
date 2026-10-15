//! Shared dialog geometry keeps confirmations and forms centered and comfortably spaced.
use eframe::egui::{
    Align, Align2, Context, Frame, InnerResponse, Layout, Ui, Vec2, WidgetText, Window,
};

pub(super) fn frame(ctx: &Context) -> Frame {
    Frame::window(&ctx.style()).inner_margin(16)
}

pub(super) fn window<'open>(ctx: &Context, title: impl Into<WidgetText>) -> Window<'open> {
    Window::new(title)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .frame(frame(ctx))
        .collapsible(false)
        .resizable(false)
        .auto_sized()
        .default_width(420.0)
        .max_width((ctx.content_rect().width() - 64.0).max(240.0))
        .max_height((ctx.content_rect().height() - 96.0).max(160.0))
        .vscroll(true)
}

pub(super) fn content(ui: &mut Ui) {
    let width = 400.0_f32.min((ui.ctx().content_rect().width() - 96.0).max(200.0));
    ui.set_min_width(width);
    ui.set_max_width((ui.ctx().content_rect().width() - 96.0).clamp(200.0, 500.0));
    ui.style_mut().wrap_mode = Some(eframe::egui::TextWrapMode::Wrap);
    ui.spacing_mut().item_spacing = Vec2::new(10.0, 10.0);
    ui.spacing_mut().button_padding = Vec2::new(12.0, 6.0);
    ui.spacing_mut().text_edit_width = width;
}

pub(super) fn actions<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.add_space(8.0);
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), ui.spacing().interact_size.y),
        Layout::right_to_left(Align::Center),
        add,
    )
}
