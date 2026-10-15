use eframe::egui::{self, Rect, Vec2};

#[derive(Clone)]
pub struct ConsoleWindow {
    pub follow: bool,
    pub placement: Option<Rect>,
    pub size: Vec2,
}
impl Default for ConsoleWindow {
    fn default() -> Self {
        Self {
            follow: true,
            placement: None,
            size: Vec2::new(760.0, 520.0),
        }
    }
}

/// Use the visible work area, excluding the menu bar and Dock. Convert Cocoa's
/// bottom-left coordinates to the top-left coordinates used by native viewports.
#[cfg(target_os = "macos")]
pub fn work_area(ctx: &egui::Context) -> Result<Rect, String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    let mtm = MainThreadMarker::new().ok_or("窗口布局必须在主线程执行")?;
    let screens = NSScreen::screens(mtm);
    let first = screens.firstObject().ok_or("未检测到可用屏幕")?;
    let primary = first.frame();
    let primary_top = primary.origin.y + primary.size.height;
    let center = ctx.input(|i| i.viewport().outer_rect.map(|rect| rect.center()));
    let mut fallback = None;
    for screen in screens.iter() {
        let frame = screen.frame();
        let whole = Rect::from_min_size(
            egui::pos2(
                frame.origin.x as f32,
                (primary_top - frame.origin.y - frame.size.height) as f32,
            ),
            egui::vec2(frame.size.width as f32, frame.size.height as f32),
        );
        let visible = screen.visibleFrame();
        let area = Rect::from_min_size(
            egui::pos2(
                visible.origin.x as f32,
                (primary_top - visible.origin.y - visible.size.height) as f32,
            ),
            egui::vec2(visible.size.width as f32, visible.size.height as f32),
        );
        if fallback.is_none() {
            fallback = Some(area);
        }
        if center.is_some_and(|point| whole.contains(point)) {
            return Ok(area);
        }
    }
    fallback.ok_or_else(|| "无法读取屏幕工作区域".into())
}
#[cfg(not(target_os = "macos"))]
pub fn work_area(ctx: &egui::Context) -> Result<Rect, String> {
    let size = ctx
        .input(|i| i.viewport().monitor_size)
        .ok_or("无法读取屏幕大小")?;
    Ok(Rect::from_min_size(egui::Pos2::ZERO, size))
}

/// Deterministic grid placement. Every window is visible within the work area;
/// aspect-ratio scoring only compares exact geometric ratios, with no model judgment.
pub fn tiles(area: Rect, count: usize) -> Vec<Rect> {
    if count == 0 {
        return vec![];
    }
    let target_ratio = 1.5;
    let columns = (1..=count)
        .min_by(|&a, &b| {
            let score = |cols: usize| {
                let rows = count.div_ceil(cols);
                let ratio = (area.width() / cols as f32) / (area.height() / rows as f32);
                (ratio / target_ratio).ln().abs()
            };
            score(a).total_cmp(&score(b))
        })
        .unwrap_or(1);
    let rows = count.div_ceil(columns);
    let cell = Vec2::new(area.width() / columns as f32, area.height() / rows as f32);
    let gap = 8.0_f32.min(cell.x / 10.0).min(cell.y / 10.0);
    (0..count)
        .map(|index| {
            Rect::from_min_size(
                area.min
                    + Vec2::new(
                        (index % columns) as f32 * cell.x,
                        (index / columns) as f32 * cell.y,
                    )
                    + Vec2::splat(gap / 2.0),
                cell - Vec2::splat(gap),
            )
        })
        .collect()
}

pub fn viewport_id(run: u64) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("dev-start-console", run))
}
