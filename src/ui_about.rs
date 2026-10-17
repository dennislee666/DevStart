//! Centered, localized product information; links open only after explicit clicks.
use super::*;
impl DevStart {
    pub(super) fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let response = egui::Modal::new(egui::Id::new("about-devstart"))
            .frame(dialog_ui::frame(ctx))
            .show(ctx, |ui| {
                dialog_ui::content(ui);
                ui.set_width((ctx.content_rect().width() - 96.0).clamp(240.0, 480.0));
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new((self.brand_icon.id(), Vec2::splat(44.0))).alt_text("DevStart"));
                    ui.vertical(|ui| {
                        ui.label(RichText::new("DevStart").size(24.0).strong());
                        ui.label(RichText::new(tf("版本 {version}", &[("version",crate::app_info::display_version().into())])).small().color(tone(self.light,MUTED)));
                    });
                });
                ui.add_space(8.0);
                ui.label(RichText::new(tr("适合 AI Coding 时代的多项目启动工作台。")).strong());
                ui.label(tr("项目多也不乱。快速找到项目，启动、停止、看日志，再切回手头的工作。"));
                ui.label(tr("需要相关服务时，找到对应项目、点击启动，再切回当前项目。项目依赖与启动顺序由你掌握。"));
                ui.add_space(6.0);
                ui.separator();
                let labels = [tr("作者"), "GitHub", tr("反馈与捐赠")];
                let font = egui::TextStyle::Body.resolve(ui.style());
                let label_width = labels.iter().map(|label| {
                    ui.painter().layout_no_wrap((*label).to_owned(), font.clone(), tone(self.light, MUTED)).size().x
                }).fold(0.0_f32, f32::max) + 4.0;
                about_detail(ui, labels[0], label_width, self.light, |ui| { ui.label(crate::app_info::AUTHOR); });
                about_detail(ui, labels[1], label_width, self.light, |ui| {
                    ui.hyperlink_to("github.com/dennislee666/DevStart", crate::app_info::REPOSITORY);
                });
                about_detail(ui, labels[2], label_width, self.light, |ui| {
                    ui.hyperlink_to(crate::app_info::CONTACT, crate::app_info::CONTACT_URL);
                });
                ui.label(RichText::new(tr("欢迎提出建议，也欢迎捐赠支持。")).small().color(tone(self.light,MUTED)));
                dialog_ui::actions(ui,|ui| { if ui.button(tr("关闭")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() { self.about_open=false; } });
            });
        if response.should_close() {
            self.about_open = false;
        }
    }
}

/// A measured label column leaves the link/value cell its full remaining width.
/// Fixed-width rows avoid Grid's first-frame intrinsic-width wrapping feedback.
fn about_detail(
    ui: &mut egui::Ui,
    label: &str,
    label_width: f32,
    light: bool,
    value: impl FnOnce(&mut egui::Ui),
) {
    let height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(label_width, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(label_width);
                ui.set_max_width(label_width);
                ui.add(egui::Label::new(RichText::new(label).color(tone(light, MUTED))).truncate());
            },
        );
        value(ui);
    });
}
