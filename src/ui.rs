use crate::appearance::{
    ACCENT, BG, BORDER, GREEN, MUTED, RED, RUNNING_BADGE_BG, RUNNING_BADGE_BORDER,
    RUNNING_BADGE_FG, RUNNING_BADGE_LIGHT_BG, RUNNING_BADGE_LIGHT_BORDER, RUNNING_BADGE_LIGHT_FG,
    RUNNING_GREEN, RUNNING_ROW_DARK, RUNNING_ROW_LIGHT, SIDE, STARTING_BLUE, SURFACE,
};
use crate::i18n::{tf, tf_count, tr};
use crate::preferences::Preferences;
use crate::{
    runtime::State,
    service::{self, Filter, Service},
};
use eframe::egui::{self, Color32, RichText, Vec2};
use egui_phosphor::regular as icon;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    time::Duration,
};
#[path = "ui_about.rs"]
mod about_ui;
#[path = "ui_console_render.rs"]
mod console_render;
#[path = "ui_dialog.rs"]
mod dialog_ui;
#[path = "ui_editor.rs"]
mod editor_ui;
#[path = "ui_git.rs"]
mod git_ui;
#[path = "ui_groups.rs"]
mod groups_ui;
#[path = "ui_project_scan.rs"]
mod scan_ui;
#[path = "ui_console_search.rs"]
mod search_ui;
#[path = "ui_settings.rs"]
mod settings_ui;
#[path = "ui_terminal.rs"]
mod terminal_ui;

#[derive(Clone)]
struct ScriptInfoDraft {
    id: u64,
    name: String,
    project_name: String,
    description: String,
    default_name: String,
    reset_name: bool,
    default_project_name: String,
    reset_project_name: bool,
}
#[derive(Clone, Copy)]
enum CloseRunTarget {
    Window,
    Tab,
}
#[derive(Clone, Copy)]
struct CloseRunRequest {
    id: u64,
    target: CloseRunTarget,
}
pub struct DevStart {
    scan_ui: scan_ui::ScanUi,
    groups_ui: groups_ui::GroupsUi,
    git_page: git_ui::GitPage,
    console_terminal_tabs: HashSet<u64>,
    console_terminals: HashMap<u64, terminal_ui::TerminalView>,
    brand_icon: egui::TextureHandle,
    about_open: bool,
    native_menu: Option<crate::application_menu::NativeMenu>,
    menu_initialized: bool,
    service: Option<Service>,
    selected_editor: Option<u64>,
    editor_reset_ids: Vec<u64>,
    editor_quit_pending: bool,
    editor_quit_confirmed: bool,
    editor_pending: Option<editor_ui::EditorAction>,
    editor_versions: Option<(u64, Vec<crate::editor::Version>)>,
    api: Option<crate::api_transport::Server>,
    error: Option<String>,
    filter: Filter,
    query: String,
    launch_sort: service::LaunchSort,
    project_group_filter: Option<u64>,
    expanded: HashSet<u64>,
    selected_run: Option<u64>,
    open_console_as_tab: Option<u64>,
    close_run: Option<CloseRunRequest>,
    delete_script: Option<u64>,
    bulk_confirm: Option<crate::bulk::Plan>,
    collapsed_groups: HashSet<u64>,
    tiled: bool,
    preset: Option<(u64, String, String, String)>,
    group_dialog: bool,
    group_name: String,
    rename_entity: Option<(bool, u64, String)>,
    import_dialog: bool,
    import_path: String,
    import_group: u64,
    script_info: Option<ScriptInfoDraft>,
    add_script: Option<(u64, String, String)>,
    console_queries: HashMap<u64, search_ui::SearchUi>,
    console_layouts: HashMap<(egui::ViewportId, u64), console_render::LayoutCache>,
    console_windows: BTreeMap<u64, crate::windows::ConsoleWindow>,
    light: bool,
    applied_preferences: Option<Preferences>,
    applied_system_theme: Option<egui::Theme>,
    settings_draft: Option<Preferences>,
    settings_saved: bool,
    system_locale: crate::i18n::Locale,
    locale_checked: std::time::Instant,
}
impl DevStart {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let light = false;
        // Decode the bundled app icon once; all sidebar frames share its GPU texture.
        let icon =
            eframe::icon_data::from_png_bytes(include_bytes!("../assets/devstart-window-icon.png"))
                .expect("embedded DevStart icon is valid PNG");
        let brand_icon = cc.egui_ctx.load_texture(
            "devstart-app-mark",
            egui::ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let (service, mut error) = match Service::load() {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e)),
        };
        let api = service.as_ref().and_then(|s| {
            match crate::api_transport::Server::bind(&s.store.root) {
                Ok(api) => Some(api),
                Err(e) => {
                    eprintln!("[dev start api] {e}");
                    error = Some(tf("本地API不可用: {e}", &[("e", (e).to_string())]));
                    None
                }
            }
        });
        let expanded = service
            .as_ref()
            .map(|s| s.catalog.projects.iter().map(|p| p.id).collect())
            .unwrap_or_default();
        let mut app = Self {
            scan_ui: Default::default(),
            groups_ui: Default::default(),
            git_page: git_ui::GitPage::default(),
            console_terminal_tabs: Default::default(),
            console_terminals: Default::default(),
            brand_icon,
            about_open: false,
            native_menu: None,
            menu_initialized: false,
            service,
            selected_editor: None,
            editor_reset_ids: vec![],
            editor_quit_pending: false,
            editor_quit_confirmed: false,
            editor_pending: None,
            editor_versions: None,
            api,
            error,
            filter: Filter::All,
            query: String::new(),
            launch_sort: service::LaunchSort::default(),
            project_group_filter: None,
            expanded,
            selected_run: None,
            open_console_as_tab: None,
            close_run: None,
            delete_script: None,
            bulk_confirm: None,
            collapsed_groups: HashSet::new(),
            tiled: true,
            preset: None,
            group_dialog: false,
            group_name: String::new(),
            rename_entity: None,
            import_dialog: false,
            import_path: String::new(),
            import_group: 1,
            script_info: None,
            add_script: None,
            console_queries: HashMap::new(),
            console_layouts: HashMap::new(),
            console_windows: BTreeMap::new(),
            light,
            applied_preferences: None,
            applied_system_theme: None,
            settings_draft: None,
            settings_saved: false,
            system_locale: crate::system_language::detect(),
            locale_checked: std::time::Instant::now(),
        };
        let args: Vec<_> = std::env::args().collect();
        for pair in args.windows(2) {
            if pair[0] == "--start-script" {
                let result = pair[1]
                    .parse::<u64>()
                    .map_err(|e| e.to_string())
                    .and_then(|id| {
                        app.service
                            .as_mut()
                            .ok_or(tr("工作空间不可用").to_string())?
                            .launch(id)
                    });
                match result {
                    Ok(id) => {
                        app.selected_run = Some(id);
                        app.filter = Filter::Running;
                    }
                    Err(e) => {
                        eprintln!("[dev start] {e}");
                        app.error = Some(e);
                    }
                }
            }
        }
        app.theme(&cc.egui_ctx);
        app
    }
    fn preferences(&self) -> Preferences {
        self.service
            .as_ref()
            .map(|s| s.catalog.preferences.clone())
            .unwrap_or_default()
    }
    fn theme(&mut self, ctx: &egui::Context) {
        let preferences = self.preferences();
        let system = ctx.input(|i| i.raw.system_theme);
        self.light = crate::appearance::light(&preferences, system);
        if self.applied_preferences.as_ref().is_none_or(|old| {
            old.ui_font != preferences.ui_font || old.console_font != preferences.console_font
        }) {
            ctx.set_fonts(crate::appearance::fonts(&preferences));
        }
        ctx.set_theme(if self.light {
            egui::ThemePreference::Light
        } else {
            egui::ThemePreference::Dark
        });
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(
            if preferences.theme == crate::preferences::Theme::System {
                egui::SystemTheme::SystemDefault
            } else if self.light {
                egui::SystemTheme::Light
            } else {
                egui::SystemTheme::Dark
            },
        ));
        ctx.set_style(crate::appearance::style(&preferences, self.light));
        self.applied_preferences = Some(preferences);
        self.applied_system_theme = system;
    }
    fn toggle_sidebar(&mut self) {
        let collapsed = !self.preferences().sidebar_collapsed;
        let result = self
            .service
            .as_mut()
            .ok_or(tr("工作空间不可用").to_string())
            .and_then(|s| s.update_preferences(serde_json::json!({"sidebar_collapsed":collapsed})))
            .map(|_| ());
        self.report(result);
    }
    fn open_settings(&mut self) {
        self.settings_draft = Some(self.preferences());
        self.settings_saved = false;
        self.filter = Filter::Settings;
    }
    fn report(&mut self, result: Result<(), String>) {
        if let Err(e) = result {
            eprintln!("[dev start] {e}");
            self.error = Some(e);
        }
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        let collapsed = self.preferences().sidebar_collapsed;
        let row_height = self.preferences().row_height();
        egui::SidePanel::left("navigation")
            .exact_width(if collapsed { 48.0 } else { 208.0 })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(if self.light {
                        Color32::from_gray(244)
                    } else {
                        SIDE
                    })
                    .inner_margin(egui::Margin::symmetric(if collapsed { 7 } else { 10 }, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if small(
                        ui,
                        icon::SIDEBAR_SIMPLE,
                        if collapsed {
                            tr("展开侧栏 · ⌘B")
                        } else {
                            tr("收起侧栏 · ⌘B")
                        },
                    ) {
                        self.toggle_sidebar();
                    }
                    if !collapsed {
                        ui.add(
                            egui::Image::new((self.brand_icon.id(), Vec2::splat(22.0)))
                                .alt_text("DevStart"),
                        );
                        ui.label(RichText::new("DevStart").strong());
                    }
                });
                ui.add_space(12.0);
                for (filter, symbol, name) in [
                    (Filter::All, icon::STACK, tr("项目")),
                    (Filter::Git, icon::GIT_BRANCH, "Git"),
                    (Filter::Favorites, icon::STAR, tr("收藏")),
                    (Filter::Running, icon::TERMINAL_WINDOW, tr("运行中")),
                ] {
                    let count = self.service.as_ref().map_or(0, |s| s.active_count());
                    let label = if collapsed {
                        symbol.to_string()
                    } else {
                        format!("{symbol}   {name}")
                    };
                    let mut button = egui::Button::new(button_label(ui, &label, ui.is_enabled()))
                        .selected(self.filter == filter)
                        .frame_when_inactive(self.filter == filter);
                    if !collapsed && filter != Filter::Running {
                        button = button.right_text("");
                    }
                    if !collapsed && filter == Filter::Running {
                        button = button.right_text(
                            RichText::new(count.to_string()).color(tone(self.light, MUTED)),
                        );
                    }
                    let response = ui
                        .add_sized([ui.available_width(), row_height + 4.0], button)
                        .on_hover_text(if filter == Filter::Running {
                            tf("运行中 · {count}", &[("count", (count).to_string())])
                        } else {
                            name.into()
                        });
                    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if response.clicked() {
                        self.filter = filter;
                        if filter == Filter::Running {
                            self.selected_run = None;
                            self.tiled = true;
                        }
                    }
                }
                if !collapsed {
                    let sessions = self
                        .service
                        .as_ref()
                        .map(|s| s.localize_tabs(s.sidebar_sessions(), crate::i18n::current()))
                        .unwrap_or_default();
                    if !sessions.is_empty() {
                        ui.add_space(16.0);
                        ui.separator();
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(tr("启动会话"))
                                .small()
                                .color(tone(self.light, MUTED)),
                        );
                        egui::ScrollArea::vertical()
                            .id_salt("session-history")
                            .max_height(
                                (ui.available_height() - row_height * 4.0)
                                    .max(row_height)
                                    .min(row_height * 8.0),
                            )
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                for tab in sessions {
                                    let response =
                                        ui.add_sized(
                                            [ui.available_width(), row_height],
                                            egui::Button::new(button_label(
                                                ui,
                                                &format!(
                                                    "{}   {}",
                                                    icon::TERMINAL_WINDOW,
                                                    tab.full_name
                                                ),
                                                ui.is_enabled(),
                                            ))
                                            .truncate()
                                            .right_text(RichText::new("   ").size(8.0))
                                            .selected(
                                                self.filter == Filter::Running
                                                    && self.selected_run == Some(tab.id),
                                            )
                                            .frame_when_inactive(
                                                self.filter == Filter::Running
                                                    && self.selected_run == Some(tab.id),
                                            ),
                                        )
                                        .on_hover_text(
                                            format!("{}\n{}", tab.tooltip, tr(tab.state.label())),
                                        );
                                    let response =
                                        response.on_hover_cursor(egui::CursorIcon::PointingHand);
                                    if tab.state.active() {
                                        paint_run_marker(ui, &response, tab.state, self.light);
                                    } else {
                                        ui.painter().text(
                                            egui::pos2(
                                                response.rect.right() - 12.0,
                                                response.rect.center().y,
                                            ),
                                            egui::Align2::CENTER_CENTER,
                                            icon::CLOCK_COUNTER_CLOCKWISE,
                                            egui::FontId::proportional(13.0),
                                            tone(self.light, MUTED),
                                        );
                                    }
                                    if response.clicked() {
                                        if let Some(s) = &mut self.service {
                                            let result = s.runtime.open_tab(tab.id);
                                            self.report(result);
                                        }
                                        self.filter = Filter::Running;
                                        self.selected_run = Some(tab.id);
                                        self.tiled = false;
                                    }
                                }
                            });
                    }
                }
                if !collapsed {
                    self.editor_sidebar(ui, row_height);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    let settings = if collapsed {
                        icon::GEAR_SIX.into()
                    } else {
                        tf("{0}   设置", &[("0", (icon::GEAR_SIX).to_string())])
                    };
                    if ui
                        .add_sized(
                            [ui.available_width(), row_height],
                            egui::Button::new(button_label(ui, &settings, ui.is_enabled()))
                                .right_text(if collapsed { "" } else { "⌘," })
                                .selected(self.filter == Filter::Settings)
                                .frame_when_inactive(self.filter == Filter::Settings),
                        )
                        .on_hover_text(tr("设置 · ⌘,"))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.open_settings();
                    }
                    let data = if collapsed {
                        icon::FOLDER_OPEN.into()
                    } else {
                        tf("{0}   数据目录", &[("0", (icon::FOLDER_OPEN).to_string())])
                    };
                    if ui
                        .add_sized(
                            [ui.available_width(), row_height],
                            egui::Button::new(button_label(ui, &data, ui.is_enabled()))
                                .right_text("")
                                .frame(false),
                        )
                        .on_hover_text(tr("打开数据目录"))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                        && let Some(s) = &self.service
                    {
                        self.report(service::open_file(&s.store.root));
                    }
                    if !collapsed {
                        ui.label(
                            RichText::new(tr("本地工作空间"))
                                .small()
                                .color(tone(self.light, MUTED)),
                        );
                    }
                });
            });
    }
    fn top(&mut self, ctx: &egui::Context) {
        if self.filter == Filter::Editor {
            return;
        }
        let running = self.filter == Filter::Running;
        let row_height = self.preferences().row_height();
        let has_tabs = self
            .service
            .as_ref()
            .is_some_and(|s| s.runtime.runs.iter().any(|r| r.tab_open));
        egui::TopBottomPanel::top("workspace-toolbar")
            .exact_height(
                row_height
                    + 12.0
                    + if running && !self.tiled && has_tabs {
                        row_height + 4.0
                    } else {
                        0.0
                    },
            )
            .frame(
                egui::Frame::new()
                    .fill(if self.light {
                        Color32::from_gray(238)
                    } else {
                        SIDE
                    })
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if running {
                        ui.spacing_mut().button_padding = Vec2::new(8.0, 3.0);
                        ui.spacing_mut().item_spacing.y = 3.0;
                    } else {
                        ui.label(
                            RichText::new(match self.filter {
                                Filter::Favorites => tr("收藏"),
                                Filter::Settings => tr("设置"),
                                Filter::Git => "Git",
                                _ => tr("项目"),
                            })
                            .strong(),
                        );
                    }
                    if self.filter == Filter::All
                        && ui
                            .button(button_label(
                                ui,
                                &format!("{} {}", icon::MAGNIFYING_GLASS, tr("自动扫描项目")),
                                true,
                            ))
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                    {
                        self.scan_ui.open = true;
                    }
                    if !running
                        && !matches!(self.filter, Filter::Settings | Filter::Editor | Filter::Git)
                    {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(button_label(
                                    ui,
                                    &tf("{0} 添加项目", &[("0", (icon::PLUS).to_string())]),
                                    ui.is_enabled(),
                                ))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.import_dialog = true;
                            }
                            if ui
                                .button(button_label(
                                    ui,
                                    &tf("{0} 新建分组", &[("0", (icon::FOLDER_PLUS).to_string())]),
                                    ui.is_enabled(),
                                ))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.group_dialog = true;
                            }
                            ui.add(
                                egui::TextEdit::singleline(&mut self.query)
                                    .hint_text(tr("搜索项目或启动项"))
                                    .desired_width(210.0),
                            );
                        });
                    }

                    if running {
                        if ui
                            .selectable_label(
                                self.tiled,
                                button_label(
                                    ui,
                                    &tf("{0} 平铺视图", &[("0", (icon::SQUARES_FOUR).to_string())]),
                                    ui.is_enabled(),
                                ),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.tiled = true;
                        }
                        if ui
                            .selectable_label(
                                !self.tiled,
                                button_label(
                                    ui,
                                    &tf(
                                        "{0} 标签视图",
                                        &[("0", (icon::TERMINAL_WINDOW).to_string())],
                                    ),
                                    ui.is_enabled(),
                                ),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.tiled = false;
                        }
                        for (action, symbol, label) in [
                            (crate::bulk::Action::Stop, icon::STOP, tr("全部停止")),
                            (crate::bulk::Action::Start, icon::PLAY, tr("全部启动")),
                            (
                                crate::bulk::Action::Restart,
                                icon::ARROW_CLOCKWISE,
                                tr("全部重启"),
                            ),
                        ] {
                            if let Some(plan) = self.service.as_ref().map(|s| s.bulk_plan(action)) {
                                let enabled = plan.enabled() && ui.is_enabled();
                                let hint = if !plan.blocked.is_empty() {
                                    format!("{label}\n{}", plan.blocked.join("；"))
                                } else if plan.targets.is_empty() {
                                    tr("没有符合条件的标签启动项").into()
                                } else {
                                    label.into()
                                };
                                let response = ui
                                    .add_enabled(
                                        enabled,
                                        egui::Button::new(button_label(
                                            ui,
                                            &format!("{symbol} {label}"),
                                            enabled,
                                        ))
                                        .frame(false),
                                    )
                                    .on_hover_text(&hint)
                                    .on_disabled_hover_text(&hint);
                                let response = if enabled {
                                    response.on_hover_cursor(egui::CursorIcon::PointingHand)
                                } else {
                                    response
                                };
                                if response.clicked() {
                                    self.bulk_confirm = Some(plan);
                                }
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(run_state_legend(ui, self.light)));
                        });
                    }
                });
                if running && !self.tiled && has_tabs {
                    ui.spacing_mut().button_padding = Vec2::new(9.0, 3.0);
                    ui.spacing_mut().item_spacing.y = 3.0;
                    self.run_tabs(ui);
                }
            });
    }
    fn launch(&mut self, id: u64) -> Option<u64> {
        if let Some(service) = &mut self.service {
            match service.launch(id) {
                Ok(run) => return Some(run),
                Err(e) => {
                    eprintln!("[dev start] {e}");
                    self.error = Some(e);
                }
            }
        }
        None
    }
    fn activate_run_tab(&mut self, run_id: u64) {
        let result = self
            .service
            .as_mut()
            .ok_or_else(|| tr("工作空间不可用").to_owned())
            .and_then(|service| service.runtime.open_tab(run_id));
        if let Err(error) = result {
            self.report(Err(error));
            return;
        }
        self.selected_run = Some(run_id);
        self.tiled = false;
        self.filter = Filter::Running;
    }
    fn open_launch_in_tabs(&mut self, script_id: u64, active: Option<u64>) {
        let run = active.or_else(|| self.launch(script_id));
        if let Some(run) = run {
            self.activate_run_tab(run);
        }
    }
    fn run_tabs(&mut self, ui: &mut egui::Ui) {
        let tabs = self
            .service
            .as_ref()
            .map(|s| s.localize_tabs(s.run_tabs(), crate::i18n::current()))
            .unwrap_or_default();
        egui::ScrollArea::horizontal()
            .id_salt("running-tabs")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for tab in tabs {
                        ui.push_id(tab.console_id, |ui| {
                            let selected = self.selected_run == Some(tab.id);
                            let response = ui.add(
                                egui::Button::new(run_tab_button_label(
                                    ui,
                                    &tab,
                                    self.light,
                                    ui.is_enabled(),
                                ))
                                .selected(selected)
                                .fill(if selected {
                                    if self.light { Color32::WHITE } else { SURFACE }
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .stroke(egui::Stroke::new(
                                    1.0_f32,
                                    if selected {
                                        tone(self.light, BORDER)
                                    } else {
                                        Color32::TRANSPARENT
                                    },
                                ))
                                .min_size(Vec2::new(110.0, self.preferences().row_height())),
                            );
                            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
                            if std::env::var_os("DEV_START_TRACE_TABS").is_some() {
                                eprintln!(
                                    "[tab-hover] id={} hovered={} rect={:?} pointer={:?}",
                                    tab.id,
                                    response.hovered(),
                                    response.rect,
                                    ui.input(|i| i.pointer.hover_pos())
                                );
                            }
                            response.clone().on_hover_ui(|ui| {
                                ui.set_max_width(420.0);
                                ui.label(&tab.tooltip);
                            });
                            response.context_menu(|ui| {
                                if ui.button(tr("独立 Console 窗口")).clicked() {
                                    self.console_windows.entry(tab.console_id).or_default();
                                    ui.close();
                                }
                                if ui.button(tr("平铺独立窗口")).clicked() {
                                    self.tile_consoles(ui.ctx());
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .button(button_label(
                                        ui,
                                        &tf("{0} 关闭标签", &[("0", (icon::X).to_string())]),
                                        ui.is_enabled(),
                                    ))
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    if tab.state.active() {
                                        self.close_run = Some(CloseRunRequest {
                                            id: tab.id,
                                            target: CloseRunTarget::Tab,
                                        });
                                    } else if let Some(s) = &mut self.service {
                                        let result = s.runtime.close_tab(tab.id, false);
                                        self.report(result);
                                    }
                                    ui.close();
                                }
                            });
                            if response.clicked() {
                                self.selected_run = Some(tab.id);
                                self.tiled = false;
                            }
                        });
                    }
                });
            });
    }
    fn running_content(&mut self, ui: &mut egui::Ui) {
        if self.tiled {
            self.tiled_consoles(ui);
            if let Some(id) = self.open_console_as_tab.take() {
                self.selected_run = Some(id);
                self.tiled = false;
                self.filter = Filter::Running;
            }
            return;
        }
        if let Some(id) = self.selected_run {
            ui.push_id(("tab-console", id), |ui| self.console(ui, id));
        } else {
            ui.add_space(24.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icon::TERMINAL_WINDOW)
                        .size(18.0)
                        .color(tone(self.light, MUTED)),
                );
                ui.label(RichText::new(tr("暂无启动记录")).color(tone(self.light, MUTED)));
            });
            if ui.button(tr("选择项目启动")).clicked() {
                self.filter = Filter::All;
            }
        }
    }
    fn tiled_consoles(&mut self, ui: &mut egui::Ui) {
        let mut tabs = self
            .service
            .as_ref()
            .map(|s| s.localize_tabs(s.run_tabs(), crate::i18n::current()))
            .unwrap_or_default();
        if tabs.is_empty() {
            ui.label(tr("暂无启动记录"));
            return;
        }
        sort_tiled_tabs_newest_first(&mut tabs);
        let columns = ((ui.available_width() / 480.0).floor() as usize)
            .max(1)
            .min(tabs.len());
        let rows = tabs.len().div_ceil(columns);
        let width = (ui.available_width() - (columns - 1) as f32 * 12.0) / columns as f32;
        let height = (ui.available_height() / rows as f32 - 12.0).max(300.0);
        egui::ScrollArea::vertical()
            .id_salt("tiled-console-grid")
            .show(ui, |ui| {
                for row in tabs.chunks(columns) {
                    ui.horizontal_top(|ui| {
                        for tab in row {
                            ui.allocate_ui_with_layout(
                                Vec2::new(width, height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_max_height(height);
                                    let frame = egui::Frame::new()
                                        .fill(if self.light {
                                            Color32::from_gray(247)
                                        } else {
                                            SIDE
                                        })
                                        .inner_margin(10.0)
                                        .show(ui, |ui| {
                                            ui.push_id(("tile-console", tab.id), |ui| {
                                                self.console(ui, tab.id)
                                            });
                                        });
                                    // Observe pointer presses without placing an input-capturing overlay.
                                    if ui.input(|i| i.pointer.any_pressed())
                                        && ui.rect_contains_pointer(frame.response.rect)
                                    {
                                        self.selected_run = Some(tab.id);
                                    }
                                    if self.selected_run == Some(tab.id) {
                                        ui.painter().rect_stroke(
                                            frame.response.rect,
                                            4.0,
                                            egui::Stroke::new(
                                                1.5_f32,
                                                tone(self.light, STARTING_BLUE),
                                            ),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                },
                            );
                        }
                    });
                    ui.add_space(12.0);
                }
            });
    }
    fn projects(&mut self, ui: &mut egui::Ui) {
        if self.filter == Filter::All
            && self
                .service
                .as_ref()
                .is_some_and(|service| service.catalog.projects.is_empty())
        {
            let height = ui.available_height();
            ui.vertical_centered(|ui| {
                ui.add_space(((height - 76.0) * 0.5).max(0.0));
                ui.label(
                    RichText::new(tr("自动扫描识别项目与启动项")).color(tone(self.light, MUTED)),
                );
                ui.add_space(8.0);
                if ui
                    .add_sized([88.0, 32.0], egui::Button::new(tr("开始")))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.scan_ui.open = true;
                }
            });
            return;
        }
        if matches!(self.filter, Filter::All | Filter::Favorites) {
            let group_filter = (self.filter == Filter::All)
                .then_some(self.project_group_filter)
                .flatten();
            let rows = self.service.as_ref().map(|service| {
                service.launch_table_content(
                    self.filter,
                    &self.query,
                    self.launch_sort,
                    group_filter,
                )
            });
            match rows {
                Some(Ok(rows)) => self.launch_table(ui, rows),
                Some(Err(error)) => self.report(Err(error)),
                None => {
                    ui.label(tr("工作空间不可用"));
                }
            };
            return;
        }
        let Some(service) = &self.service else {
            ui.label(tr("工作空间不可用"));
            return;
        };
        let grouped = service.grouped_projects(self.filter, &self.query);
        egui::ScrollArea::vertical()
            .id_salt("grouped-projects")
            .show(ui, |ui| {
                // Reserve a stable trailing gutter, including when the scrollbar overlays content.
                ui.set_max_width((ui.available_width() - 15.0).max(0.0));
                let launch_columns = launch_widths(
                    (ui.available_width() - ui.spacing().indent * 2.0).max(0.0),
                    ui.spacing().item_spacing.x,
                );
                for (group, group_projects) in grouped {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(icon::FOLDER)
                                .size(16.0)
                                .color(tone(self.light, ACCENT)),
                        );
                        let expanded = !self.collapsed_groups.contains(&group.id);
                        let toggle = ui
                            .push_id(("group-toggle", group.id), |ui| {
                                ui.add(
                                    egui::Button::new(
                                        RichText::new(crate::i18n::group_name(&group)).strong(),
                                    )
                                    .right_text(
                                        button_icon(
                                            if expanded {
                                                icon::CARET_DOWN
                                            } else {
                                                icon::CARET_RIGHT
                                            },
                                            true,
                                        )
                                        .size(14.0),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text(if expanded {
                                    tr("收起分组")
                                } else {
                                    tr("展开分组")
                                })
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            })
                            .inner;
                        if toggle {
                            if expanded {
                                self.collapsed_groups.insert(group.id);
                            } else {
                                self.collapsed_groups.remove(&group.id);
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.push_id(("group-actions", group.id), |ui| {
                                icon_menu(ui, icon::DOTS_THREE, |ui| {
                                    if ui.button(tr("重命名分组")).clicked() {
                                        self.rename_entity =
                                            Some((true, group.id, group.name.clone()));
                                        ui.close();
                                    }
                                    if ui
                                        .add_enabled(
                                            self.groups_ui.pending_is_idle(),
                                            egui::Button::new(tr("重新扫描更新项目启动脚本")),
                                        )
                                        .clicked()
                                    {
                                        self.refresh_group(group.id, ui.ctx());
                                        ui.close();
                                    }
                                    if group.id != 1
                                        && ui
                                            .add_enabled(
                                                self.groups_ui.pending_is_idle(),
                                                egui::Button::new(tr("删除分组")),
                                            )
                                            .clicked()
                                    {
                                        self.groups_ui.delete =
                                            Some((crate::group_service::DeletionTarget::Group(group.id), group.name.clone()));
                                        ui.close();
                                    }
                                })
                                .response
                                .on_hover_text(tr("分组操作"));
                            });
                        });
                    });
                    if self.collapsed_groups.contains(&group.id) {
                        continue;
                    }
                    if group_projects.is_empty() {
                        ui.indent(("empty-group", group.id), |ui| {
                            ui.label(
                                RichText::new(tr("暂无项目"))
                                    .small()
                                    .color(tone(self.light, MUTED)),
                            );
                        });
                    }
                    if group_projects.iter().any(|project| !project.scripts.is_empty()) {
                        ui.indent(("group-launch-header", group.id), |ui| {
                            ui.indent("launch-options", |ui| {
                                launch_header(ui, launch_columns);
                            });
                        });
                    }
                    for project in &group_projects {
                        ui.indent(("project", project.id), |ui| {
                            ui.push_id(project.id, |ui| {
                                ui.horizontal(|ui| {
                                    let expanded = self.expanded.contains(&project.id);
                                    let arrow_clicked = small(
                                        ui,
                                        if expanded {
                                            icon::CARET_DOWN
                                        } else {
                                            icon::CARET_RIGHT
                                        },
                                        if expanded {
                                            tr("收起启动项")
                                        } else {
                                            tr("展开启动项")
                                        },
                                    );
                                    let name_clicked = ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(project.display_label()).strong(),
                                            )
                                            .frame(false),
                                        )
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .on_hover_text(service::path_label(&project.path))
                                        .clicked();
                                    let path_width = (ui.available_width() - 150.0).max(40.0);
                                    let path_clicked = ui
                                        .allocate_ui_with_layout(
                                            Vec2::new(path_width, 24.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.add(
                                                    egui::Button::new(
                                                        RichText::new(service::path_label(
                                                            &project.path,
                                                        ))
                                                        .small()
                                                        .color(tone(self.light, MUTED)),
                                                    )
                                                    .frame(false)
                                                    .truncate(),
                                                )
                                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                                .on_hover_text(project.path.display().to_string())
                                                .clicked()
                                            },
                                        )
                                        .inner;
                                    if arrow_clicked || name_clicked || path_clicked {
                                        if expanded {
                                            self.expanded.remove(&project.id);
                                        } else {
                                            self.expanded.insert(project.id);
                                        }
                                    }
                                    if self
                                        .service
                                        .as_ref()
                                        .is_some_and(|s| s.has_external_run(project.id))
                                    {
                                        ui.label(
                                            RichText::new(tr("外部运行 · 参数未识别"))
                                                .small()
                                                .color(tone(self.light, ACCENT)),
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            icon_menu(ui, icon::DOTS_THREE, |ui| {
                                                if ui.button(tr("重命名项目")).clicked() {
                                                    self.rename_entity = Some((
                                                        false,
                                                        project.id,
                                                        project.name.clone(),
                                                    ));
                                                    ui.close();
                                                }
                                                if ui.button(tr("重新读取启动脚本")).clicked()
                                                {
                                                    let result = self
                                                        .service
                                                        .as_mut()
                                                        .unwrap()
                                                        .rescan(project.id);
                                                    self.report(result);
                                                    ui.close();
                                                }
                                                if ui.button(tr("添加启动脚本")).clicked() {
                                                    self.add_script = Some((
                                                        project.id,
                                                        String::new(),
                                                        "/bin/bash".into(),
                                                    ));
                                                    ui.close();
                                                }
                                                if ui.button(tr("打开项目目录")).clicked() {
                                                    self.report(service::open_file(&project.path));
                                                    ui.close();
                                                }
                                                if ui.add_enabled(self.groups_ui.pending_is_idle(), egui::Button::new(tr("删除项目"))).clicked() {
                                                    self.groups_ui.delete = Some((crate::group_service::DeletionTarget::Project(project.id), project.name.clone()));
                                                    ui.close();
                                                }
                                                ui.separator();
                                                ui.add_space(2.0);
                                                ui.add(egui::Label::new(
                                                    RichText::new(tr("移动到分组"))
                                                        .size(11.0)
                                                        .color(tone(self.light, MUTED)),
                                                ));
                                                ui.add_space(2.0);
                                                let groups = self
                                                    .service
                                                    .as_ref()
                                                    .unwrap()
                                                    .catalog
                                                    .groups
                                                    .clone();
                                                for g in groups {
                                                    if ui
                                                        .button(crate::i18n::group_name(&g))
                                                        .clicked()
                                                    {
                                                        let result = self
                                                            .service
                                                            .as_mut()
                                                            .unwrap()
                                                            .move_project(project.id, g.id);
                                                        self.report(result);
                                                        ui.close();
                                                    }
                                                }
                                            });
                                            if favorite_button(
                                                ui,
                                                project.favorite,
                                                tr("收藏项目"),
                                                self.light,
                                            ) {
                                                let result = self
                                                    .service
                                                    .as_mut()
                                                    .unwrap()
                                                    .project_favorite(project.id);
                                                self.report(result);
                                            }
                                        },
                                    );
                                });
                                ui.add_space(2.0);
                                ui.separator();
                                ui.add_space(2.0);
                                if self.expanded.contains(&project.id) {
                                    ui.indent("launch-options", |ui| {
                                        for script in &project.scripts {
                                            self.launch_row(ui, script, launch_columns);
                                        }
                                        if project.scripts.is_empty() {
                                            ui.label(
                                                RichText::new(tr("暂无启动项"))
                                                    .small()
                                                    .color(tone(self.light, MUTED)),
                                            );
                                        }
                                    });
                                }
                            });
                        });
                    }
                    ui.add_space(14.0);
                }
            });
    }
    fn launch_table(&mut self, ui: &mut egui::Ui, content: service::LaunchTableContent) {
        egui::ScrollArea::both()
            .id_salt("launch-table")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let spacing = ui.spacing().item_spacing.x;
                let grouped = matches!(&content, service::LaunchTableContent::Projects(_));
                let combined = grouped && self.project_group_filter.is_none();
                let sort = if grouped && !combined {
                    self.launch_sort.for_project_sections()
                } else {
                    self.launch_sort
                };
                let columns: &[usize] = if grouped {
                    &[1, 0, 3, 4, 5, 6]
                } else {
                    &[0, 1, 2, 3, 4, 5, 6]
                };
                let widths =
                    launch_table_column_widths(ui.available_width(), spacing, grouped, combined);
                let table_width = widths.iter().sum::<f32>() + spacing * (columns.len() - 1) as f32;
                ui.set_min_width(table_width);
                ui.set_max_width(table_width);
                if self.filter == Filter::All {
                    self.project_group_filter_bar(ui);
                    ui.add_space(4.0);
                }
                let row_height = self.preferences().row_height();
                ui.horizontal(|ui| {
                    for &index in columns {
                        let (label, column) = match index {
                            0 => (
                                if grouped {
                                    tr("启动项")
                                } else {
                                    tr("启动项名")
                                },
                                None,
                            ),
                            1 => {
                                if combined {
                                    (tr("组名 / 项目"), Some(service::LaunchSortColumn::Group))
                                } else {
                                    (tr("项目"), Some(service::LaunchSortColumn::Project))
                                }
                            }
                            2 => (tr("组名"), Some(service::LaunchSortColumn::Group)),
                            3 => (tr("启动命令"), None),
                            4 => (tr("状态"), Some(service::LaunchSortColumn::Status)),
                            5 => (tr("端口"), Some(service::LaunchSortColumn::Port)),
                            _ => (tr("操作"), None),
                        };
                        let width = widths[index];
                        ui.allocate_ui_with_layout(
                            Vec2::new(width, row_height),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(width);
                                if let Some(column) = column {
                                    let selected = sort.column == column;
                                    let direction = sort.direction;
                                    let indicator = if selected {
                                        match direction {
                                            service::SortDirection::Ascending => "↑",
                                            service::SortDirection::Descending => "↓",
                                        }
                                    } else {
                                        "↕"
                                    };
                                    let response = ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(format!("{label} {indicator}"))
                                                    .small()
                                                    .color(tone(
                                                        self.light,
                                                        if selected { ACCENT } else { MUTED },
                                                    )),
                                            )
                                            .frame(false)
                                            .min_size(Vec2::new(width, row_height)),
                                        )
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .on_hover_text(tr("点击列名切换排序方向"));
                                    if response.clicked() {
                                        self.launch_sort = if selected {
                                            service::LaunchSort {
                                                column,
                                                direction: match direction {
                                                    service::SortDirection::Ascending => {
                                                        service::SortDirection::Descending
                                                    }
                                                    service::SortDirection::Descending => {
                                                        service::SortDirection::Ascending
                                                    }
                                                },
                                            }
                                        } else {
                                            service::LaunchSort {
                                                column,
                                                direction: service::SortDirection::Ascending,
                                            }
                                        };
                                    }
                                } else {
                                    ui.add(
                                        egui::Label::new(RichText::new(label).small().weak())
                                            .truncate(),
                                    );
                                }
                            },
                        );
                    }
                });
                ui.separator();
                if content.is_empty() {
                    ui.add_space(8.0);
                    let empty = if self.filter == Filter::Favorites {
                        tr("暂无收藏的启动项")
                    } else {
                        tr("没有匹配的项目启动项")
                    };
                    ui.label(RichText::new(empty).small().color(tone(self.light, MUTED)));
                    return;
                }
                match content {
                    service::LaunchTableContent::Flat(rows) => {
                        for row in &rows {
                            self.launch_table_row(ui, row, widths, false);
                            ui.separator();
                        }
                    }
                    service::LaunchTableContent::Projects(projects) => {
                        for project in projects {
                            self.launch_project_title(
                                ui,
                                &project,
                                widths[1],
                                table_width,
                                combined,
                            );
                            if self.expanded.contains(&project.project_id) {
                                for row in &project.rows {
                                    self.launch_table_row(ui, row, widths, true);
                                    ui.separator();
                                }
                            }
                            ui.add_space(8.0);
                        }
                    }
                }
            });
    }
    fn launch_project_title(
        &mut self,
        ui: &mut egui::Ui,
        project: &service::LaunchProjectSection,
        width: f32,
        table_width: f32,
        combined: bool,
    ) {
        let label = if combined {
            format!(
                "{} / {}",
                crate::i18n::group_name(&project.group),
                project.project_name
            )
        } else {
            project.project_name.clone()
        };
        let row = project.action_target();
        ui.push_id(("launch-project-title", project.project_id), |ui| {
            egui::Frame::new()
                .fill(if self.light {
                    Color32::from_gray(245)
                } else {
                    Color32::from_gray(24)
                })
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(0, 6))
                .show(ui, |ui| {
                    ui.set_min_width(table_width);
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(width, 32.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.set_min_width(width);
                                ui.horizontal(|ui| {
                                    let expanded = self.expanded.contains(&project.project_id);
                                    let arrow_clicked = small(
                                        ui,
                                        if expanded {
                                            icon::CARET_DOWN
                                        } else {
                                            icon::CARET_RIGHT
                                        },
                                        if expanded {
                                            tr("收起启动项")
                                        } else {
                                            tr("展开启动项")
                                        },
                                    );
                                    let name_clicked = ui
                                        .add(
                                            egui::Button::new(RichText::new(&label).strong())
                                                .frame(false)
                                                .truncate(),
                                        )
                                        .on_hover_text(&label)
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked();
                                    if arrow_clicked || name_clicked {
                                        if expanded {
                                            self.expanded.remove(&project.project_id);
                                        } else {
                                            self.expanded.insert(project.project_id);
                                        }
                                    }
                                });
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(service::path_label(&project.project_path))
                                            .size(10.0)
                                            .color(tone(self.light, MUTED)),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(project.project_path.display().to_string());
                            },
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            icon_menu(ui, icon::DOTS_THREE, |ui| {
                                self.group_action_items(ui, &project.group)
                            })
                            .response
                            .on_hover_text(tr("分组操作"))
                            .widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    tr("分组操作"),
                                )
                            });
                            self.project_action_menu(ui, &row, 28.0, false);
                        });
                    });
                });
        });
    }
    fn project_group_filter_bar(&mut self, ui: &mut egui::Ui) {
        let groups = self
            .service
            .as_ref()
            .map(Service::groups_for_launch_filter)
            .unwrap_or_default();
        if self
            .project_group_filter
            .is_some_and(|selected| !groups.iter().any(|group| group.id == selected))
        {
            self.project_group_filter = None;
        }

        egui::ScrollArea::horizontal()
            .id_salt("project-group-filter")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let mut options = Vec::with_capacity(groups.len() + 1);
                    options.push((None, tr("全部").to_string()));
                    options.extend(
                        groups.iter().map(|group| {
                            (Some(group.id), crate::i18n::group_name(group).to_string())
                        }),
                    );
                    for (group_id, label) in options {
                        let selected = self.project_group_filter == group_id;
                        let color = tone(self.light, if selected { ACCENT } else { MUTED });
                        let response = ui
                            .add(
                                egui::Button::new(RichText::new(label).small().color(color))
                                    .fill(if selected {
                                        tone(self.light, SURFACE)
                                    } else {
                                        Color32::TRANSPARENT
                                    })
                                    .stroke(egui::Stroke::new(
                                        if selected { 1.0 } else { 0.0 },
                                        if selected {
                                            tone(self.light, ACCENT)
                                        } else {
                                            Color32::TRANSPARENT
                                        },
                                    ))
                                    .corner_radius(6.0)
                                    .min_size(Vec2::new(42.0, 28.0)),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                        if response.clicked() {
                            self.project_group_filter = group_id;
                        }
                    }
                });
            });
    }
    fn launch_table_row(
        &mut self,
        ui: &mut egui::Ui,
        row: &service::LaunchTableRow,
        widths: [f32; 7],
        grouped: bool,
    ) {
        let script_id = row.script.as_ref().map(|script| script.id).unwrap_or(0);
        ui.push_id((row.project_id, script_id), |ui| {
            let row_height = self
                .preferences()
                .row_height()
                .max(if grouped { 30.0 } else { 40.0 });
            let active = row
                .script
                .as_ref()
                .and_then(|script| self.service.as_ref()?.active_run(script.id));
            let failed = row.script.as_ref().is_some_and(|script| {
                self.service
                    .as_ref()
                    .and_then(|service| service.latest_state(script.id))
                    == Some(State::Failed)
            });
            let port = row
                .script
                .as_ref()
                .and_then(|script| {
                    self.service.as_ref().map(|service| {
                        service.running_port_cell_for_locale(script.id, crate::i18n::current())
                    })
                })
                .unwrap_or(crate::presentation::PortCell {
                    label: "--".into(),
                    tooltip: tr("无法确定启动项端口").into(),
                });
            let status = row.script.as_ref().map_or("--", |script| {
                self.service
                    .as_ref()
                    .map_or("--", |service| service.script_status_label(script.id))
            });
            let is_running = status == "运行中";
            let launch_name = row.script.as_ref().map_or_else(
                || "—".to_owned(),
                |script| {
                    let name = crate::presentation::launch_label(script.display_label());
                    if failed {
                        tf("{0} · 失败", &[("0", name)])
                    } else {
                        name
                    }
                },
            );
            let command = row
                .script
                .as_ref()
                .map(crate::model::Script::tab_command)
                .unwrap_or_else(|| "—".into());
            let project_path = service::path_label(&row.project_path);
            let text = [
                launch_name,
                row.project_name.clone(),
                crate::i18n::group_name(&row.group).to_owned(),
                command,
                tr(status).to_owned(),
                port.label.clone(),
            ];
            egui::Frame::new()
                .fill(if is_running {
                    if self.light {
                        RUNNING_ROW_LIGHT
                    } else {
                        RUNNING_ROW_DARK
                    }
                } else {
                    Color32::TRANSPARENT
                })
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(0, if is_running { 2 } else { 0 }))
                .show(ui, |ui| {
                    if is_running {
                        ui.set_min_width(ui.available_width());
                    }
                    ui.horizontal(|ui| {
                        let columns: &[usize] = if grouped {
                            &[1, 0, 3, 4, 5]
                        } else {
                            &[0, 1, 2, 3, 4, 5]
                        };
                        for &index in columns {
                            let value = &text[index];
                            let layout = if index == 1 {
                                egui::Layout::top_down(egui::Align::Min)
                                    .with_main_align(egui::Align::Center)
                            } else {
                                egui::Layout::left_to_right(egui::Align::Center)
                            };
                            ui.allocate_ui_with_layout(
                                Vec2::new(widths[index], row_height),
                                layout,
                                |ui| {
                                    ui.set_min_width(widths[index]);
                                    if index == 1 && grouped {
                                        return;
                                    }
                                    if index == 1 {
                                        let name =
                                            ui.add(
                                                egui::Label::new(RichText::new(value).color(tone(
                                                    self.light,
                                                    crate::appearance::TEXT,
                                                )))
                                                .halign(egui::Align::Min)
                                                .truncate(),
                                            );
                                        name.on_hover_text(&row.project_name);
                                        let path = ui.add(
                                            egui::Label::new(
                                                RichText::new(&project_path)
                                                    .size(10.0)
                                                    .color(tone(self.light, MUTED)),
                                            )
                                            .halign(egui::Align::Min)
                                            .truncate(),
                                        );
                                        path.on_hover_text(row.project_path.display().to_string());
                                        return;
                                    }
                                    if index == 4 && is_running {
                                        let (badge_bg, badge_fg, badge_border) = if self.light {
                                            (
                                                RUNNING_BADGE_LIGHT_BG,
                                                RUNNING_BADGE_LIGHT_FG,
                                                RUNNING_BADGE_LIGHT_BORDER,
                                            )
                                        } else {
                                            (
                                                RUNNING_BADGE_BG,
                                                RUNNING_BADGE_FG,
                                                RUNNING_BADGE_BORDER,
                                            )
                                        };
                                        let font = egui::FontId::proportional(12.0);
                                        let text_width = ui.fonts_mut(|fonts| {
                                            fonts
                                                .layout_no_wrap(
                                                    value.to_owned(),
                                                    font.clone(),
                                                    badge_fg,
                                                )
                                                .size()
                                                .x
                                        });
                                        let (badge_rect, _) = ui.allocate_exact_size(
                                            Vec2::new(text_width + 12.0, 18.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().rect(
                                            badge_rect,
                                            egui::CornerRadius::same(9),
                                            badge_bg,
                                            egui::Stroke::new(1.0, badge_border),
                                            egui::StrokeKind::Inside,
                                        );
                                        ui.put(
                                            badge_rect,
                                            egui::Label::new(
                                                RichText::new(value).color(badge_fg).font(font),
                                            )
                                            .halign(egui::Align::Center),
                                        );
                                        return;
                                    }
                                    let label = if index == 0 && failed {
                                        RichText::new(value).color(tone(self.light, RED))
                                    } else if index == 3 {
                                        RichText::new(value)
                                            .monospace()
                                            .size(10.5)
                                            .color(tone(self.light, MUTED))
                                    } else {
                                        RichText::new(value).color(tone(
                                            self.light,
                                            if index == 5 {
                                                MUTED
                                            } else {
                                                crate::appearance::TEXT
                                            },
                                        ))
                                    };
                                    let response =
                                        ui.add(egui::Label::new(label).truncate().sense(
                                            if index == 0 && row.script.is_some() {
                                                egui::Sense::click()
                                            } else {
                                                egui::Sense::hover()
                                            },
                                        ));
                                    match index {
                                        0 => {
                                            if let Some(script) = &row.script {
                                                let response = response.on_hover_text(format!(
                                                    "{}\n{}\n{}",
                                                    script.display_label(),
                                                    script.description,
                                                    tr("双击在标签视图中打开")
                                                ));
                                                if response.double_clicked() {
                                                    self.open_launch_in_tabs(script.id, active);
                                                }
                                            } else {
                                                response.on_hover_text(tr("项目暂无启动项"));
                                            }
                                        }
                                        2 => {
                                            response
                                                .on_hover_text(crate::i18n::group_name(&row.group));
                                        }
                                        3 => {
                                            if let Some(script) = &row.script {
                                                response.on_hover_text(script.tab_command());
                                            }
                                        }
                                        5 => {
                                            response.on_hover_text(&port.tooltip);
                                        }
                                        _ => {}
                                    }
                                },
                            );
                        }
                        ui.allocate_ui_with_layout(
                            Vec2::new(widths[6], row_height),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(widths[6]);
                                ui.horizontal(|ui| {
                                    self.launch_table_actions(ui, row, active, row_height, grouped);
                                });
                            },
                        );
                    });
                });
        });
    }
    fn launch_table_actions(
        &mut self,
        ui: &mut egui::Ui,
        row: &service::LaunchTableRow,
        active: Option<u64>,
        row_height: f32,
        grouped: bool,
    ) {
        if let Some(script) = &row.script {
            let favorite_project = (self.filter == Filter::Favorites && row.project_favorite)
                .then_some(row.project_id);
            self.launch_actions(ui, script, active, row_height, favorite_project);
        } else {
            for _ in 0..6 {
                launch_action_slot(ui, row_height, |ui| ui.add_space(0.0));
            }
        }
        if !grouped {
            self.project_action_menu(ui, row, row_height, true);
        }
    }
    fn project_action_menu(
        &mut self,
        ui: &mut egui::Ui,
        row: &service::LaunchTableRow,
        row_height: f32,
        include_group: bool,
    ) {
        let groups = self
            .service
            .as_ref()
            .map(|service| service.catalog.groups.clone())
            .unwrap_or_default();
        launch_action_slot(ui, row_height, |ui| {
            icon_menu(ui, icon::FOLDER, |ui| {
                if ui.button(tr("重命名项目")).clicked() {
                    self.rename_entity = Some((false, row.project_id, row.project_name.clone()));
                    ui.close();
                }
                if ui.button(tr("重新读取启动脚本")).clicked() {
                    let result = self.service.as_mut().unwrap().rescan(row.project_id);
                    self.report(result);
                    ui.close();
                }
                if ui.button(tr("添加启动脚本")).clicked() {
                    self.add_script = Some((row.project_id, String::new(), "/bin/bash".into()));
                    ui.close();
                }
                if ui.button(tr("打开项目目录")).clicked() {
                    self.report(service::open_file(&row.project_path));
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.groups_ui.pending_is_idle(),
                        egui::Button::new(tr("删除项目")),
                    )
                    .clicked()
                {
                    self.groups_ui.delete = Some((
                        crate::group_service::DeletionTarget::Project(row.project_id),
                        row.project_name.clone(),
                    ));
                    ui.close();
                }
                if ui
                    .button(if row.project_favorite {
                        tr("取消收藏项目")
                    } else {
                        tr("收藏项目")
                    })
                    .clicked()
                {
                    let result = self
                        .service
                        .as_mut()
                        .unwrap()
                        .project_favorite(row.project_id);
                    self.report(result);
                    ui.close();
                }
                ui.separator();
                ui.add(egui::Label::new(
                    RichText::new(tr("移动到分组"))
                        .size(11.0)
                        .color(tone(self.light, MUTED)),
                ));
                for group in groups {
                    if ui
                        .button(crate::i18n::group_name(&group))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        let result = self
                            .service
                            .as_mut()
                            .unwrap()
                            .move_project(row.project_id, group.id);
                        self.report(result);
                        ui.close();
                    }
                }
                if include_group {
                    ui.separator();
                    self.group_action_items(ui, &row.group);
                }
            })
            .response
            .on_hover_text(if include_group {
                tr("项目及分组操作")
            } else {
                tr("项目操作")
            })
            .widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tr("项目操作"))
            });
        });
    }
    fn group_action_items(&mut self, ui: &mut egui::Ui, group: &crate::model::Group) {
        if ui.button(tr("添加项目")).clicked() {
            self.import_group = group.id;
            self.import_dialog = true;
            ui.close();
        }
        if ui.button(tr("重命名分组")).clicked() {
            self.rename_entity = Some((true, group.id, group.name.clone()));
            ui.close();
        }
        if ui
            .add_enabled(
                self.groups_ui.pending_is_idle(),
                egui::Button::new(tr("重新扫描更新项目启动脚本")),
            )
            .clicked()
        {
            self.refresh_group(group.id, ui.ctx());
            ui.close();
        }
        if group.id != 1
            && ui
                .add_enabled(
                    self.groups_ui.pending_is_idle(),
                    egui::Button::new(tr("删除分组")),
                )
                .clicked()
        {
            self.groups_ui.delete = Some((
                crate::group_service::DeletionTarget::Group(group.id),
                group.name.clone(),
            ));
            ui.close();
        }
    }
    fn launch_row(
        &mut self,
        ui: &mut egui::Ui,
        script: &crate::model::Script,
        (name_width, command_width, status_width, port_width): (f32, f32, f32, f32),
    ) {
        let row_height = self.preferences().row_height();
        ui.push_id(script.id, |ui| {
            let port = self
                .service
                .as_ref()
                .map(|s| s.running_port_cell_for_locale(script.id, crate::i18n::current()))
                .unwrap_or(crate::presentation::PortCell {
                    label: "--".into(),
                    tooltip: tr("工作空间不可用").into(),
                });
            let active = self.service.as_ref().and_then(|s| s.active_run(script.id));
            let failed = self
                .service
                .as_ref()
                .and_then(|s| s.latest_state(script.id))
                == Some(State::Failed);
            let title = if failed {
                tf(
                    "{0} · 失败",
                    &[(
                        "0",
                        (crate::presentation::launch_label(script.display_label())).to_string(),
                    )],
                )
            } else {
                crate::presentation::launch_label(script.display_label())
            };
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(name_width, self.preferences().row_height()),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        // Reserve each column even when its text is short.
                        ui.set_min_width(name_width);
                        let name = if failed {
                            RichText::new(&title).color(tone(self.light, RED))
                        } else {
                            RichText::new(&title)
                        };
                        ui.add(egui::Label::new(name).truncate())
                            .on_hover_text(format!(
                                "{}\n{}",
                                script.display_label(),
                                script.description
                            ));
                    },
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(command_width, self.preferences().row_height()),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(command_width);
                        ui.add(
                            egui::Label::new(
                                RichText::new(script.tab_command())
                                    .monospace()
                                    .size(11.5)
                                    .color(tone(self.light, MUTED)),
                            )
                            .truncate(),
                        );
                    },
                );
                let status = self
                    .service
                    .as_ref()
                    .map_or("--", |s| s.script_status_label(script.id));
                ui.allocate_ui_with_layout(
                    Vec2::new(status_width, self.preferences().row_height()),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(status_width);
                        ui.label(RichText::new(tr(status)).color(tone(
                            self.light,
                            if status == "运行中" { GREEN } else { MUTED },
                        )));
                    },
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(port_width, self.preferences().row_height()),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(port_width);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&port.label)
                                    .monospace()
                                    .color(tone(self.light, MUTED)),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&port.tooltip);
                    },
                );
                self.launch_actions(ui, script, active, row_height, None);
            });
        });
    }
    fn launch_actions(
        &mut self,
        ui: &mut egui::Ui,
        script: &crate::model::Script,
        active: Option<u64>,
        row_height: f32,
        favorite_project: Option<u64>,
    ) {
        let launch_hint = if let Some(reason) = script.launch_block_reason() {
            tf("未启用：{reason}", &[("reason", (tr(reason)).to_string())])
        } else if active.is_some() {
            tr("已运行，可停止或重启").into()
        } else {
            tr("启动").into()
        };
        let launch_clicked = launch_action_slot(ui, row_height, |ui| {
            action_button(
                ui,
                active.is_none() && script.launch_block_reason().is_none(),
                icon::PLAY,
                &launch_hint,
                Some(tone(self.light, GREEN)),
            )
            .clicked()
        });
        if launch_clicked {
            let _ = self.launch(script.id);
        }
        let stop_clicked = launch_action_slot(ui, row_height, |ui| {
            action_button(ui, active.is_some(), icon::STOP, tr("停止"), None).clicked()
        });
        if stop_clicked && let Some(run) = active {
            self.service.as_mut().unwrap().runtime.stop(run);
        }
        let restart_clicked = launch_action_slot(ui, row_height, |ui| {
            action_button(
                ui,
                self.service
                    .as_ref()
                    .is_some_and(|service| service.can_restart_script(script.id)),
                icon::ARROW_CLOCKWISE,
                tr("重启"),
                None,
            )
            .clicked()
        });
        if restart_clicked {
            match self.service.as_mut().unwrap().restart(script.id) {
                Ok(_) => {}
                Err(error) => self.report(Err(error)),
            }
        }
        let is_favorite = favorite_project.is_some() || script.favorite;
        let favorite_clicked = launch_action_slot(ui, row_height, |ui| {
            favorite_button(
                ui,
                is_favorite,
                if favorite_project.is_some() {
                    tr("取消收藏项目")
                } else if script.favorite {
                    tr("取消收藏启动项")
                } else {
                    tr("收藏启动项")
                },
                self.light,
            )
        });
        if favorite_clicked {
            let result = if let Some(project_id) = favorite_project {
                self.service.as_mut().unwrap().project_favorite(project_id)
            } else {
                self.service.as_mut().unwrap().script_favorite(script.id)
            };
            self.report(result);
        }
        let edit_clicked = launch_action_slot(ui, row_height, |ui| {
            small(ui, icon::PENCIL_SIMPLE, tr("编辑原脚本文件"))
        });
        if edit_clicked {
            self.open_editor(script.id);
        }
        launch_action_slot(ui, row_height, |ui| {
            icon_menu(ui, icon::DOTS_THREE, |ui| {
                if let Some(run) = active
                    && ui.button(tr("查看 Console")).clicked()
                {
                    self.selected_run = Some(run);
                    self.filter = Filter::Running;
                    ui.close();
                }
                if ui.button(tr("另存参数预设")).clicked() {
                    self.preset = Some((
                        script.id,
                        tf("{0} · 新参数", &[("0", (script.name).to_string())]),
                        script.args.join("\n"),
                        script
                            .expected_port
                            .map(|port| port.to_string())
                            .unwrap_or_default(),
                    ));
                    ui.close();
                }
                if ui.button(tr("修改信息")).clicked() {
                    self.script_info = Some(ScriptInfoDraft {
                        id: script.id,
                        name: script.display_label().to_owned(),
                        project_name: self
                            .service
                            .as_ref()
                            .unwrap()
                            .catalog
                            .projects
                            .iter()
                            .find(|p| p.scripts.iter().any(|s| s.id == script.id))
                            .map(|p| p.display_label().to_owned())
                            .unwrap_or_default(),
                        description: script.description.clone(),
                        default_name: script.name.clone(),
                        reset_name: script.display_name.is_none(),
                        default_project_name: self
                            .service
                            .as_ref()
                            .unwrap()
                            .catalog
                            .projects
                            .iter()
                            .find(|p| p.scripts.iter().any(|s| s.id == script.id))
                            .map(|p| p.name.clone())
                            .unwrap_or_default(),
                        reset_project_name: self
                            .service
                            .as_ref()
                            .unwrap()
                            .catalog
                            .projects
                            .iter()
                            .find(|p| p.scripts.iter().any(|s| s.id == script.id))
                            .is_some_and(|p| p.display_name.is_none()),
                    });
                    ui.close();
                }
                if ui.button(tr("根据执行情况生成简介")).clicked() {
                    let result = self
                        .service
                        .as_mut()
                        .unwrap()
                        .generate_description(script.id);
                    self.report(result);
                    ui.close();
                }
                if ui
                    .button(if favorite_project.is_some() {
                        tr("取消收藏项目")
                    } else if script.favorite {
                        tr("取消收藏")
                    } else {
                        tr("收藏启动项")
                    })
                    .clicked()
                {
                    let result = if let Some(project_id) = favorite_project {
                        self.service.as_mut().unwrap().project_favorite(project_id)
                    } else {
                        self.service.as_mut().unwrap().script_favorite(script.id)
                    };
                    self.report(result);
                    ui.close();
                }
                if ui
                    .button(tr("删除启动项"))
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.delete_script = Some(script.id);
                    ui.close();
                }
            });
        });
    }
    fn console(&mut self, ui: &mut egui::Ui, id: u64) {
        let root_console = ui.ctx().viewport_id() == egui::ViewportId::ROOT;
        let console_id = self
            .service
            .as_ref()
            .and_then(|s| s.runtime.runs.iter().find(|r| r.id == id))
            .map_or(id, |r| r.console_id);
        let mut terminal_active = root_console && self.console_terminal_tabs.contains(&console_id);
        let mut terminal_selected = false;
        let can_find =
            self.selected_run == Some(id) || ui.ctx().viewport_id() != egui::ViewportId::ROOT;
        let find = !terminal_active
            && can_find
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
        let search = self.console_queries.entry(id).or_default();
        if find {
            search.open = true;
            search.focus = true;
        }
        let Some(s) = &mut self.service else {
            return;
        };
        let Some(index) = s.runtime.runs.iter().position(|r| r.id == id) else {
            return;
        };
        let mut close_console = None;
        let mut stop = false;
        let mut restart = None;
        let mut clear = false;
        let mut open = false;
        let mut toggle_favorite = false;
        let run_script_id = s.runtime.runs[index].script_id;
        let favorite_state = s.script(run_script_id).map(|script| script.favorite);
        let run = &s.runtime.runs[index];
        egui::Frame::new()
            .fill(if self.light {
                Color32::from_rgb(240, 242, 244)
            } else {
                SURFACE
            })
            .corner_radius(4)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let root_console = ui.ctx().viewport_id() == egui::ViewportId::ROOT;
                let tiled_root = self.tiled && root_console;
                let tab_root = !self.tiled && root_console;
                let available_width = ui.available_width();
                let compact = ui.available_height() < 420.0
                    || available_width < 620.0
                    || tiled_root && available_width < 780.0;
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.spacing_mut().button_padding = Vec2::new(7.0, 3.0);
                ui.horizontal(|ui| {
                    let reserved_actions = if tiled_root {
                        334.0
                    } else if tab_root {
                        194.0
                    } else {
                        76.0
                    };
                    let title_width = (ui.available_width() - reserved_actions).max(20.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(title_width, 24.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let name_width = if compact {
                                title_width
                            } else {
                                (title_width - 450.0).clamp(80.0, 340.0)
                            };
                            ui.allocate_ui_with_layout(
                                Vec2::new(name_width, 22.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    if tiled_root {
                                        let title = tiled_console_title(ui, run, self.light);
                                        let response = ui
                                            .add(
                                                egui::Label::new(title)
                                                    .truncate()
                                                    .sense(egui::Sense::click()),
                                            )
                                            .on_hover_text(format!(
                                                "{}\n{}",
                                                crate::i18n::run_title(run),
                                                tr("双击在标签视图中打开")
                                            ));
                                        if response.double_clicked() {
                                            self.open_console_as_tab = Some(id);
                                        }
                                    } else {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(crate::i18n::run_title(run).as_ref())
                                                    .strong(),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(crate::i18n::run_title(run).as_ref());
                                    }
                                },
                            );
                            if !compact {
                                ui.add_space(8.0);
                                run_status(ui, run, self.light);
                            }
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if tiled_root || tab_root {
                            let target = if tiled_root {
                                CloseRunTarget::Window
                            } else {
                                CloseRunTarget::Tab
                            };
                            let hint = if run.close_requested {
                                tr("正在停止并关闭")
                            } else if run.state.active() {
                                tr("停止并关闭")
                            } else {
                                tr("关闭 Console")
                            };
                            let color = if run.state.active() {
                                tone(self.light, RED)
                            } else {
                                tone(self.light, MUTED)
                            };
                            if action_button(ui, !run.close_requested, icon::X, hint, Some(color))
                                .clicked()
                            {
                                close_console = Some(target);
                            }
                        }
                        if action_button(
                            ui,
                            matches!(run.state, State::Starting | State::Running),
                            icon::STOP,
                            tr("停止"),
                            Some(tone(self.light, RED)),
                        )
                        .clicked()
                        {
                            stop = true;
                        }
                        ui.add_enabled_ui(s.can_restart(id), |ui| {
                            if run.external.is_some() && run.associated_scripts.len() > 1 {
                                icon_menu(ui, icon::ARROW_CLOCKWISE, |ui| {
                                    for id in &run.associated_scripts {
                                        if let Some(script) = s.script(*id)
                                            && ui
                                                .add_enabled(
                                                    s.can_restart_target(run.id, *id),
                                                    egui::Button::new(
                                                        crate::presentation::launch_label(
                                                            script.display_label(),
                                                        ),
                                                    ),
                                                )
                                                .clicked()
                                        {
                                            restart = Some(*id);
                                            ui.close();
                                        }
                                    }
                                });
                            } else if action_button(
                                ui,
                                true,
                                icon::ARROW_CLOCKWISE,
                                tr("重启"),
                                None,
                            )
                            .clicked()
                            {
                                restart = Some(run.script_id);
                            }
                        });
                        if tiled_root {
                            if small(ui, icon::CORNERS_OUT, tr("在标签视图中打开当前 Console"))
                            {
                                self.open_console_as_tab = Some(id);
                            }
                            if small(ui, icon::MAGNIFYING_GLASS, tr("搜索日志 · ⌘F")) {
                                search.toggle();
                            }
                            clear = small(ui, icon::TRASH, tr("清空当前 Console 日志"));
                            open = small(ui, icon::FILE_TEXT, tr("打开日志文件"));
                            if let Some(favorite) = favorite_state
                                && favorite_button(
                                    ui,
                                    favorite,
                                    if favorite {
                                        tr("取消收藏启动项")
                                    } else {
                                        tr("收藏启动项")
                                    },
                                    self.light,
                                )
                            {
                                toggle_favorite = true;
                            }
                        } else if tab_root {
                            if !terminal_active {
                                if small(ui, icon::MAGNIFYING_GLASS, tr("搜索日志 · ⌘F")) {
                                    search.toggle();
                                }
                                clear = small(ui, icon::TRASH, tr("清空当前 Console 日志"));
                                open = small(ui, icon::FILE_TEXT, tr("打开日志文件"));
                            }
                            if let Some(favorite) = favorite_state
                                && favorite_button(
                                    ui,
                                    favorite,
                                    if favorite {
                                        tr("取消收藏启动项")
                                    } else {
                                        tr("收藏启动项")
                                    },
                                    self.light,
                                )
                            {
                                toggle_favorite = true;
                            }
                        }
                    });
                });
                if compact && !root_console {
                    ui.horizontal_wrapped(|ui| run_status(ui, run, self.light));
                }
                if tiled_root || tab_root {
                    // Use the status row in compact cards and the command row in spacious headers.
                    // Narrow cards move the switch to its own row rather than hide terminal access.
                    let inline = available_width >= 360.0;
                    if compact && !inline {
                        ui.horizontal_wrapped(|ui| run_status(ui, run, self.light));
                    }
                    ui.horizontal(|ui| {
                        let reserve = 170.0;
                        let width = (ui.available_width() - reserve).max(0.0);
                        if !compact || inline {
                            ui.allocate_ui_with_layout(
                                Vec2::new(width, ui.text_style_height(&egui::TextStyle::Body)),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_min_width(width);
                                    ui.set_max_width(width);
                                    if compact {
                                        ui.horizontal_wrapped(|ui| run_status(ui, run, self.light));
                                    } else {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(
                                                    crate::presentation::command_preview(
                                                        &run.display_command,
                                                    ),
                                                )
                                                .monospace()
                                                .color(tone(self.light, MUTED)),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(
                                            crate::presentation::command_preview(&run.command),
                                        );
                                    }
                                },
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if let Some(terminal) = console_mode_switch(ui, terminal_active) {
                                terminal_active = terminal;
                                terminal_selected = terminal;
                                if terminal {
                                    self.console_terminal_tabs.insert(console_id);
                                } else {
                                    self.console_terminal_tabs.remove(&console_id);
                                }
                            }
                        });
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if small(ui, icon::MAGNIFYING_GLASS, tr("搜索日志 · ⌘F")) {
                                search.toggle();
                            }

                            clear = small(ui, icon::TRASH, tr("清空当前 Console 日志"));
                            open = small(ui, icon::FILE_TEXT, tr("打开日志文件"));
                            if let Some(favorite) = favorite_state
                                && favorite_button(
                                    ui,
                                    favorite,
                                    if favorite {
                                        tr("取消收藏启动项")
                                    } else {
                                        tr("收藏启动项")
                                    },
                                    self.light,
                                )
                            {
                                toggle_favorite = true;
                            }
                            if !compact {
                                let preview =
                                    crate::presentation::command_preview(&run.display_command);
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&preview)
                                                    .monospace()
                                                    .color(tone(self.light, MUTED)),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(
                                            crate::presentation::command_preview(&run.command),
                                        );
                                    },
                                );
                            }
                        });
                    });
                }
            });
        if let Some(target) = close_console {
            self.close_run = Some(CloseRunRequest { id, target });
        }
        if terminal_active {
            match s.run_project_context(id) {
                Err(error) => {
                    ui.colored_label(tone(self.light, RED), error);
                }
                Ok((project, path)) => {
                    let view = self.console_terminals.entry(project).or_default();
                    if terminal_selected {
                        view.request_focus();
                    }
                    if view.session.is_none() {
                        match s.terminals.open(project, &path) {
                            Ok(session) => view.attach(session),
                            Err(error) => {
                                ui.colored_label(tone(self.light, RED), error);
                            }
                        }
                    }
                    if view.session.is_some() {
                        let height = (ui.available_height() - 72.0).max(120.0);
                        match view.show(ui, &mut s.terminals, self.light, "", height, true) {
                            Ok(outcome) => {
                                if outcome.reopen {
                                    view.session = None;
                                }
                            }
                            Err(error) => {
                                ui.colored_label(tone(self.light, RED), error);
                                if ui.button(tr("重开终端")).clicked() {
                                    view.session = None;
                                }
                            }
                        }
                    }
                }
            }
        } else {
            ui.add_space(4.0);
            let input = search_ui::controls(ui, search, id);
            let view = match s.runtime.runs[index].lines.query(
                &search.query,
                search.current,
                input.direction,
            ) {
                Ok(view) => view,
                Err(e) => {
                    ui.colored_label(tone(self.light, RED), e);
                    return;
                }
            };
            search.current = view.current;
            search.matched = view.matched_lines;
            let jump = input.direction != 0
                || input.changed && !search.query.keyword.is_empty() && !search.query.only_matches;
            let run = &s.runtime.runs[index];
            if run.lines.dropped > 0 || run.lines.shortened {
                ui.label(
                    RichText::new(tr("仅显示最近输出；日志文件也有保留上限"))
                        .small()
                        .color(tone(self.light, MUTED)),
                );
            }
            ui.add_space(2.0);
            egui::Frame::new()
                .fill(if self.light {
                    Color32::from_gray(249)
                } else {
                    BG
                })
                .inner_margin(10.0)
                .show(ui, |ui| {
                    if search.open && search.query.only_matches && view.rows.is_empty() {
                        ui.label(
                            RichText::new(tr("没有匹配的日志")).color(tone(self.light, MUTED)),
                        );
                    }
                    let cache = self
                        .console_layouts
                        .entry((ui.ctx().viewport_id(), id))
                        .or_default();
                    if let Some(target) = console_render::show(
                        ui,
                        run,
                        &view,
                        search,
                        cache,
                        &s.catalog.preferences,
                        self.light,
                        true,
                        jump,
                    ) && let Err(e) = crate::console_text::open(&target, &run.cwd)
                    {
                        self.error = Some(e);
                    }
                });
        }
        let run = &s.runtime.runs[index];
        let path = run.log_path.clone();
        let mut restarted = None;
        let restart_result = if let Some(script_id) = restart {
            s.restart_run(id, script_id).map(|run| {
                restarted = Some(run);
            })
        } else {
            Ok(())
        };
        if stop {
            s.runtime.stop(id);
        }
        if clear {
            s.runtime.clear(id);
        }
        if let Some(run) = restarted {
            self.selected_run = Some(run);
        }
        let favorite_result = if toggle_favorite {
            s.script_favorite(run_script_id)
        } else {
            Ok(())
        };
        self.report(restart_result);
        self.report(favorite_result);
        if open {
            self.report(service::open_file(&path));
        }
    }
    fn tile_consoles(&mut self, ctx: &egui::Context) {
        let ids: Vec<_> = self
            .service
            .as_ref()
            .map(|s| {
                s.run_tabs()
                    .into_iter()
                    .filter(|tab| {
                        tab.state.active() || self.console_windows.contains_key(&tab.console_id)
                    })
                    .map(|tab| tab.console_id)
                    .collect()
            })
            .unwrap_or_default();
        let area = match crate::windows::work_area(ctx) {
            Ok(area) => area,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let placements = crate::windows::tiles(area, ids.len());
        for (id, rect) in ids.into_iter().zip(placements) {
            self.console_windows.entry(id).or_default().placement = Some(rect);
        }
    }
    fn show_console_windows(&mut self, ctx: &egui::Context) {
        let ids: Vec<_> = self.console_windows.keys().copied().collect();
        for console_id in ids {
            let Some((title, id)) = self
                .service
                .as_ref()
                .and_then(|service| {
                    service
                        .current_console_generation(console_id)
                        .and_then(|id| service.runtime.runs.iter().find(|run| run.id == id))
                })
                .map(|run| (crate::i18n::run_title(run).into_owned(), run.id))
            else {
                self.console_windows.remove(&console_id);
                continue;
            };
            let mut view = self.console_windows.remove(&console_id).unwrap_or_default();
            let placement = view.placement.take();
            let mut builder = egui::ViewportBuilder::default()
                .with_title(format!("Console · {title}"))
                .with_inner_size(view.size);
            if let Some(rect) = placement {
                view.size = rect.size();
                builder = builder.with_position(rect.min).with_inner_size(rect.size());
            }
            let mut closed = false;
            ctx.show_viewport_immediate(
                crate::windows::viewport_id(console_id),
                builder,
                |child, class| {
                    if class == egui::ViewportClass::Embedded {
                        self.error = Some(tr("当前图形后端不支持独立 Console 窗口").into());
                        closed = true;
                        return;
                    }
                    if child.input(|i| i.viewport().close_requested()) {
                        closed = true;
                        return;
                    }
                    if let Some(rect) = placement {
                        child.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        child.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
                        child.send_viewport_cmd(egui::ViewportCommand::OuterPosition(rect.min));
                        let chrome = child.input(|i| {
                            match (i.viewport().outer_rect, i.viewport().inner_rect) {
                                (Some(outer), Some(inner)) => {
                                    (outer.size() - inner.size()).max(Vec2::ZERO)
                                }
                                _ => Vec2::ZERO,
                            }
                        });
                        view.size = (rect.size() - chrome).max(Vec2::splat(1.0));
                        child.send_viewport_cmd(egui::ViewportCommand::InnerSize(view.size));
                    }
                    egui::CentralPanel::default()
                        .frame(
                            egui::Frame::new()
                                .fill(if self.light {
                                    Color32::from_gray(253)
                                } else {
                                    BG
                                })
                                .inner_margin(14.0),
                        )
                        .show(child, |ui| {
                            ui.push_id(("console-window", console_id), |ui| self.console(ui, id));
                        });
                    child.request_repaint_after(Duration::from_millis(100));
                },
            );
            if !closed {
                self.console_windows.insert(console_id, view);
            }
        }
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        self.project_scan_dialog(ctx);
        self.group_dialogs(ctx);
        if let Some(id) = self.delete_script {
            let target = self.service.as_ref().and_then(|service| {
                service.catalog.projects.iter().find_map(|project| {
                    project
                        .scripts
                        .iter()
                        .find(|script| script.id == id)
                        .map(|script| {
                            (
                                project.name.clone(),
                                script.display_label().to_owned(),
                                crate::api_config::idle(service, &[id]).err(),
                            )
                        })
                })
            });
            if let Some((project, name, blocked)) = target {
                let mut open = true;
                dialog_ui::window(ctx, tr("删除启动项"))
                    .open(&mut open)
                    .collapsible(false)
                    .resizable(false)
                    .default_width(360.0)
                    .show(ctx, |ui| {
                        dialog_ui::content(ui);
                        ui.label(format!("{project} / {name}"));
                        ui.label(tr("确认删除此启动项？"));
                        ui.label(
                            RichText::new(tr("保留原脚本文件和日志。"))
                                .color(tone(self.light, MUTED)),
                        );
                        if let Some(reason) = &blocked {
                            ui.label(
                                RichText::new(crate::i18n::diagnostic(reason))
                                    .color(tone(self.light, RED)),
                            );
                        }
                        dialog_ui::actions(ui, |ui| {
                            let response = ui.add_enabled(
                                blocked.is_none(),
                                egui::Button::new(
                                    RichText::new(tr("删除")).color(tone(self.light, RED)),
                                ),
                            );
                            let response = if blocked.is_none() {
                                response.on_hover_cursor(egui::CursorIcon::PointingHand)
                            } else {
                                response
                            };
                            if response.clicked() {
                                // Revalidate on the service owner: state may change while confirmation is open.
                                let result = crate::api::dispatch(
                                    self.service.as_mut().unwrap(),
                                    "scripts.delete",
                                    &serde_json::json!({"id":id}),
                                )
                                .map(|_| ());
                                if result.is_ok() {
                                    self.delete_script = None;
                                }
                                self.report(result);
                            }
                            if ui
                                .button(tr("取消"))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.delete_script = None;
                            }
                        });
                    });
                if !open {
                    self.delete_script = None;
                }
            } else {
                self.delete_script = None;
            }
        }
        if let Some(plan) = self.bulk_confirm.clone() {
            egui::Modal::new(egui::Id::new("bulk-sessions-confirm"))
                .frame(dialog_ui::frame(ctx))
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    let (title, verb, confirm) = match plan.action {
                        crate::bulk::Action::Stop => {
                            (tr("确认全部停止？"), tr("将停止"), tr("确认全部停止"))
                        }
                        crate::bulk::Action::Start => {
                            (tr("确认全部启动？"), tr("将启动"), tr("确认全部启动"))
                        }
                        crate::bulk::Action::Restart => {
                            (tr("确认全部重启？"), tr("将重启"), tr("确认全部重启"))
                        }
                    };
                    ui.heading(title);
                    ui.label(tf_count(
                        "{verb} {0} 个标签启动项",
                        &[
                            ("0", (plan.targets.len()).to_string()),
                            ("verb", (verb).to_string()),
                        ],
                        plan.targets.len(),
                    ));
                    egui::ScrollArea::vertical()
                        .max_height(220.0)
                        .show(ui, |ui| {
                            for target in &plan.targets {
                                ui.label(&target.title);
                            }
                        });
                    dialog_ui::actions(ui, |ui| {
                        if ui.button(confirm).clicked() {
                            let result = self
                                .service
                                .as_mut()
                                .ok_or(tr("工作空间不可用").to_string())
                                .and_then(|s| s.apply_bulk(&plan, true))
                                .and_then(|result| {
                                    if result.errors.is_empty() {
                                        Ok(())
                                    } else {
                                        Err(result.errors.join("；"))
                                    }
                                });
                            self.bulk_confirm = None;
                            self.report(result);
                        }
                        if ui.button(tr("取消")).clicked() {
                            self.bulk_confirm = None;
                        }
                    });
                });
        }

        if let Some(request) = self.close_run {
            let active = self
                .service
                .as_ref()
                .and_then(|s| s.runtime.runs.iter().find(|r| r.id == request.id))
                .is_some_and(|run| run.state.active());
            // Recheck when rendering the request: a service may have exited after X was clicked.
            if !active {
                let result = self
                    .service
                    .as_mut()
                    .ok_or(tr("工作空间不可用").to_string())
                    .and_then(|s| s.runtime.close_tab(request.id, false));
                self.report(result);
                self.close_run = None;
            } else {
                let title = match request.target {
                    CloseRunTarget::Window => tr("确认停止服务并关闭窗口吗？"),
                    CloseRunTarget::Tab => tr("确认停止服务并关闭此标签吗？"),
                };
                egui::Modal::new(egui::Id::new("close-running-tab"))
                    .frame(dialog_ui::frame(ctx))
                    .show(ctx, |ui| {
                        dialog_ui::content(ui);
                        ui.heading(title);
                        if let Some(run) = self
                            .service
                            .as_ref()
                            .and_then(|s| s.runtime.runs.iter().find(|r| r.id == request.id))
                        {
                            ui.label(crate::i18n::run_title(run).as_ref());
                        }
                        dialog_ui::actions(ui, |ui| {
                            if ui.button(tr("停止并关闭")).clicked() {
                                let result = self
                                    .service
                                    .as_mut()
                                    .ok_or(tr("工作空间不可用").to_string())
                                    .and_then(|s| s.runtime.close_tab(request.id, true));
                                self.report(result);
                                self.close_run = None;
                            }
                            if ui.button(tr("取消")).clicked() {
                                self.close_run = None;
                            }
                        });
                    });
            }
        }

        if let Some((source, mut name, mut args, mut port)) = self.preset.clone() {
            dialog_ui::window(ctx, tr("另存参数预设"))
                .collapsible(false)
                .default_width(500.0)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label(tr("启动项名称"));
                    ui.text_edit_singleline(&mut name);
                    ui.label(tr("参数（每行一个，原样传递）"));
                    ui.add(
                        egui::TextEdit::multiline(&mut args)
                            .desired_width(460.0)
                            .desired_rows(5),
                    );
                    if let Some(s) = self.service.as_ref().and_then(|s| s.script(source)) {
                        ui.label(
                            RichText::new(s.invocation())
                                .small()
                                .color(tone(self.light, MUTED)),
                        );
                    }
                    ui.label(tr("端口约束（可选）"));
                    ui.text_edit_singleline(&mut port);
                    self.preset = Some((source, name.clone(), args.clone(), port.clone()));
                    dialog_ui::actions(ui, |ui| {
                        if ui.button(tr("保存启动项")).clicked() {
                            let result = self
                                .service
                                .as_mut()
                                .ok_or(tr("工作空间不可用").to_string())
                                .and_then(|s| s.create_preset(source, &name, &args, &port));
                            match result {
                                Ok(id) => {
                                    if let Some(p) = self.service.as_ref().and_then(|s| {
                                        s.catalog
                                            .projects
                                            .iter()
                                            .find(|p| p.scripts.iter().any(|s| s.id == id))
                                    }) {
                                        self.expanded.insert(p.id);
                                    }
                                    self.preset = None;
                                }
                                Err(e) => self.error = Some(e),
                            }
                        }
                        if ui.button(tr("取消")).clicked() {
                            self.preset = None;
                        }
                    });
                });
        }

        if let Some((group, id, mut name)) = self.rename_entity.clone() {
            dialog_ui::window(
                ctx,
                if group {
                    tr("重命名分组")
                } else {
                    tr("重命名项目")
                },
            )
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                dialog_ui::content(ui);
                ui.add(egui::TextEdit::singleline(&mut name).desired_width(340.0));
                self.rename_entity = Some((group, id, name.clone()));
                dialog_ui::actions(ui, |ui| {
                    if ui.button(tr("保存")).clicked() {
                        let (method, params) = if group {
                            ("groups.update", serde_json::json!({"id":id,"name":name}))
                        } else {
                            (
                                "projects.update",
                                serde_json::json!({"id":id,"patch":{"name":name}}),
                            )
                        };
                        let result =
                            crate::api::dispatch(self.service.as_mut().unwrap(), method, &params)
                                .map(|_| ());
                        if result.is_ok() {
                            self.rename_entity = None;
                        }
                        self.report(result);
                    }
                    if ui.button(tr("取消")).clicked() {
                        self.rename_entity = None;
                    }
                });
            });
        }
        if self.group_dialog {
            dialog_ui::window(ctx, tr("新建分组"))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.group_name).hint_text(tr("分组名称")),
                    );
                    dialog_ui::actions(ui, |ui| {
                        if ui.button(tr("创建")).clicked()
                            && let Some(s) = &mut self.service
                        {
                            let result = s.add_group(&self.group_name);
                            if result.is_ok() {
                                self.group_dialog = false;
                                self.group_name.clear();
                            }
                            self.report(result);
                        }
                        if ui.button(tr("取消")).clicked() {
                            self.group_dialog = false;
                        }
                    });
                });
        }
        if self.import_dialog {
            dialog_ui::window(ctx, tr("添加项目"))
                .collapsible(false)
                .resizable(false)
                .default_width(500.0)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label(tr("项目目录"));
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [380.0, 32.0],
                            egui::TextEdit::singleline(&mut self.import_path)
                                .hint_text(tr("项目绝对路径")),
                        );
                        if ui.button(tr("选择")).clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_folder()
                        {
                            self.import_path = path.display().to_string();
                        }
                    });
                    if let Some(s) = &self.service {
                        egui::ComboBox::from_label(tr("分组"))
                            .selected_text(
                                s.catalog
                                    .groups
                                    .iter()
                                    .find(|g| g.id == self.import_group)
                                    .map(crate::i18n::group_name)
                                    .unwrap_or(tr("默认分组")),
                            )
                            .show_ui(ui, |ui| {
                                for g in &s.catalog.groups {
                                    ui.selectable_value(
                                        &mut self.import_group,
                                        g.id,
                                        crate::i18n::group_name(g),
                                    );
                                }
                            });
                    }
                    dialog_ui::actions(ui, |ui| {
                        if ui.button(tr("读取启动脚本")).clicked()
                            && let Some(s) = &mut self.service
                        {
                            match s
                                .import(&PathBuf::from(self.import_path.trim()), self.import_group)
                            {
                                Ok(id) => {
                                    self.expanded.insert(id);
                                    self.import_dialog = false;
                                    self.import_path.clear();
                                }
                                Err(e) => self.error = Some(e),
                            }
                        }
                        if ui.button(tr("取消")).clicked() {
                            self.import_dialog = false;
                        }
                    });
                });
        }
        if let Some(mut draft) = self.script_info.clone() {
            let mut open = true;
            dialog_ui::window(ctx, tr("修改信息")).open(&mut open).collapsible(false).resizable(false).default_width(440.0).show(ctx,|ui|{
                dialog_ui::content(ui);
                ui.horizontal(|ui|{
                    ui.label(tr("启动项名称"));
                    let response=ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(270.0)).on_hover_text(tr("最多50个字符"));
                    if response.changed(){draft.reset_name=false;}
                    if ui.button(tr("恢复默认")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){draft.name=draft.default_name.clone();draft.reset_name=true;}
                });
                ui.horizontal(|ui| {
                    ui.label(tr("项目名"));
                    let response=ui.add(egui::TextEdit::singleline(&mut draft.project_name).desired_width(270.0));
                    if response.changed(){draft.reset_project_name=false;}
                    if ui.button(tr("恢复默认")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){draft.project_name=draft.default_project_name.clone();draft.reset_project_name=true;}
                });
                ui.label(tr("简介"));
                ui.add(egui::TextEdit::multiline(&mut draft.description).desired_width(f32::INFINITY).desired_rows(4));
                self.script_info=Some(draft.clone());
                dialog_ui::actions(ui,|ui|{
                    if ui.button(tr("保存")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){
                        let name=if draft.reset_name {None}else{Some(draft.name.clone())};
                        let project_name=if draft.reset_project_name {None}else{Some(draft.project_name.clone())};
                        let result=crate::api::dispatch(self.service.as_mut().unwrap(),"scripts.update",&serde_json::json!({"id":draft.id,"patch":{"display_name":name,"project_display_name":project_name,"description":draft.description}})).map(|_|());
                        if result.is_ok(){self.script_info=None;}
                        self.report(result);
                    }
                    if ui.button(tr("取消")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){self.script_info=None;}
                });
            });
            if !open {
                self.script_info = None;
            }
        }
        if let Some((id, mut path, mut interpreter)) = self.add_script.clone() {
            dialog_ui::window(ctx, tr("添加启动脚本"))
                .collapsible(false)
                .default_width(480.0)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label(tr("原始脚本路径"));
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut path);
                        if ui.button(tr("选择")).clicked()
                            && let Some(file) = rfd::FileDialog::new().pick_file()
                        {
                            path = file.display().to_string();
                        }
                    });
                    ui.label(tr("解释器（例如 /bin/bash、python3）"));
                    ui.text_edit_singleline(&mut interpreter);
                    self.add_script = Some((id, path.clone(), interpreter.clone()));
                    dialog_ui::actions(ui, |ui| {
                        if ui.button(tr("添加")).clicked() {
                            let result = self.service.as_mut().unwrap().add_script(
                                id,
                                &PathBuf::from(&path),
                                &interpreter,
                            );
                            if result.is_ok() {
                                self.add_script = None;
                            }
                            self.report(result);
                        }
                        if ui.button(tr("取消")).clicked() {
                            self.add_script = None;
                        }
                    });
                });
        }
        if let Some(error) = self.error.clone() {
            dialog_ui::window(ctx, tr("操作失败"))
                .collapsible(false)
                .default_width(520.0)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label(
                        RichText::new(crate::i18n::diagnostic(&error)).color(tone(self.light, RED)),
                    );
                    dialog_ui::actions(ui, |ui| {
                        if ui.button(tr("复制错误")).clicked() {
                            ctx.copy_text(error);
                        }
                        if ui.button(tr("关闭")).clicked() {
                            self.error = None;
                        }
                    });
                });
        }
    }
}
impl eframe::App for DevStart {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.preferences().language == crate::i18n::Language::System
            && self.locale_checked.elapsed() >= Duration::from_secs(2)
        {
            self.system_locale = crate::system_language::detect();
            self.locale_checked = std::time::Instant::now();
        }
        let _locale = crate::i18n::scope(self.preferences().language.resolve(self.system_locale));
        if !self.menu_initialized {
            self.menu_initialized = true;
            match crate::application_menu::NativeMenu::install(crate::i18n::current()) {
                Ok(menu) => self.native_menu = Some(menu),
                Err(error) => {
                    eprintln!("[dev start] {error}");
                    self.error = Some(error);
                }
            }
        }
        if let Some(menu) = &mut self.native_menu {
            menu.sync_locale(crate::i18n::current());
        }
        if crate::application_menu::take_about_request() {
            self.about_open = true;
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::Minimized(false),
            );
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
        self.editor_quit_guard(ctx);
        if let Some(s) = &mut self.service {
            let selected_console = self.selected_run.and_then(|id| {
                s.runtime
                    .runs
                    .iter()
                    .find(|run| run.id == id)
                    .map(|run| run.console_id)
            });
            s.poll();
            if let Some(api) = &self.api {
                api.poll(s);
            }
            if let Some(console_id) = selected_console {
                self.selected_run = s.current_console_generation(console_id);
            }
            let visible_consoles: HashSet<_> =
                s.run_tabs().iter().map(|tab| tab.console_id).collect();
            self.console_windows
                .retain(|console_id, _| visible_consoles.contains(console_id));
            let retained: HashSet<_> = s.runtime.runs.iter().map(|r| r.id).collect();
            self.console_layouts
                .retain(|(_, id), _| retained.contains(id));
            self.console_queries.retain(|id, _| retained.contains(id));
            if let Some(error) = s.runtime.error.take() {
                self.error = Some(error);
            }
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::B)) {
            self.toggle_sidebar();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.open_settings();
        }
        if self.applied_preferences.as_ref() != Some(&self.preferences())
            || self.applied_system_theme != ctx.input(|i| i.raw.system_theme)
        {
            // API changes refresh an untouched settings draft without discarding
            // edits the user has not applied yet.
            if self.settings_draft.as_ref() == self.applied_preferences.as_ref() {
                self.settings_draft = Some(self.preferences());
            }
            self.theme(ctx);
        }
        self.sidebar(ctx);
        if self.filter == Filter::Running {
            self.selected_run = self
                .service
                .as_ref()
                .and_then(|s| s.selected_run(self.selected_run));
        }
        self.top(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(if self.light {
                        Color32::from_gray(253)
                    } else {
                        BG
                    })
                    .inner_margin(if self.filter == Filter::Running {
                        egui::Margin::symmetric(14, 8)
                    } else {
                        egui::Margin::symmetric(18, 12)
                    }),
            )
            .show(ctx, |ui| {
                if self.filter == Filter::Editor {
                    self.editor_page(ui);
                } else if self.filter == Filter::Settings {
                    self.settings_page(ui);
                } else if self.filter == Filter::Git {
                    self.git_page(ui);
                } else if self.filter == Filter::Running {
                    self.running_content(ui);
                } else {
                    self.projects(ui);
                }
            });
        self.editor_dialogs(ctx);
        self.dialogs(ctx);
        self.show_console_windows(ctx);
        self.about_dialog(ctx);
        self.git_notifications(ctx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
/// Keep icon-only fonts away from names: some icon fonts declare empty Latin glyphs.
fn button_label(ui: &egui::Ui, text: &str, enabled: bool) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    let mut start = 0;
    let mut icon_run = None;
    for (offset, ch) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        let is_icon = ('\u{e000}'..='\u{f8ff}').contains(&ch);
        if icon_run.is_some_and(|v| v != is_icon) || offset == text.len() {
            let family = if icon_run == Some(true) {
                egui::FontFamily::Name(
                    if enabled {
                        "action-fill"
                    } else {
                        "action-outline"
                    }
                    .into(),
                )
            } else {
                font.family.clone()
            };
            job.append(
                &text[start..offset],
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::new(font.size, family),
                    color: Color32::PLACEHOLDER,
                    ..Default::default()
                },
            );
            start = offset;
        }
        icon_run = Some(is_icon);
    }
    job
}
fn run_tab_button_label(
    ui: &egui::Ui,
    tab: &crate::presentation::RunTab,
    light: bool,
    enabled: bool,
) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    let icon_family = egui::FontFamily::Name(
        if enabled {
            "action-fill"
        } else {
            "action-outline"
        }
        .into(),
    );
    job.append(
        icon::TERMINAL_WINDOW,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(font.size, icon_family),
            color: Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job.append(
        " ",
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job.append(
        state_marker(tab.state),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(font.size * 0.75, font.family.clone()),
            color: state_color(tab.state, light),
            ..Default::default()
        },
    );
    job.append(
        " ",
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job.append(
        &tab.label,
        0.0,
        egui::TextFormat {
            font_id: font,
            color: Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job
}
fn run_state_legend(ui: &egui::Ui, light: bool) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    for (text, color) in [
        ("●", state_color(State::Starting, light)),
        (tr(" 启动中    "), tone(light, MUTED)),
        ("●", state_color(State::Running, light)),
        (tr(" 运行中    "), tone(light, MUTED)),
        ("○", tone(light, MUTED)),
        (tr(" 已停止"), tone(light, MUTED)),
    ] {
        job.append(
            text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                ..Default::default()
            },
        );
    }
    job
}
/// Shared log/PTY switch for both root console layouts; state belongs to the logical console.
fn console_mode_switch(ui: &mut egui::Ui, terminal_active: bool) -> Option<bool> {
    let mut changed = None;
    if ui
        .selectable_label(terminal_active, tr("终端"))
        .on_hover_text(tr("在当前项目目录执行命令"))
        .clicked()
    {
        changed = Some(true);
    }
    if ui
        .selectable_label(!terminal_active, "Console")
        .on_hover_text(tr("查看启动日志"))
        .clicked()
    {
        changed = Some(false);
    }
    changed
}

fn tiled_console_title(
    ui: &egui::Ui,
    run: &crate::runtime::Run,
    light: bool,
) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut job = egui::text::LayoutJob::default();
    job.append(
        state_marker(run.state),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::new(font.size * 0.72, font.family.clone()),
            color: state_color(run.state, light),
            ..Default::default()
        },
    );
    job.append(
        " ",
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color: Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job.append(
        crate::i18n::run_title(run).as_ref(),
        0.0,
        egui::TextFormat {
            font_id: font,
            color: ui.visuals().strong_text_color(),
            ..Default::default()
        },
    );
    job
}
fn state_marker(state: State) -> &'static str {
    if state.active() { "●" } else { "○" }
}
/// Filled icons identify actions; disabled controls retain the outline variant.
fn button_icon(symbol: &str, enabled: bool) -> RichText {
    RichText::new(symbol).family(egui::FontFamily::Name(
        if enabled {
            "action-fill"
        } else {
            "action-outline"
        }
        .into(),
    ))
}
fn icon_menu<R>(
    ui: &mut egui::Ui,
    symbol: &str,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let enabled = ui.is_enabled();
    let result = ui.menu_button(button_icon(symbol, enabled), content);
    if enabled {
        result
            .response
            .clone()
            .on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    result
}
fn action_button(
    ui: &mut egui::Ui,
    enabled: bool,
    symbol: &str,
    label: &str,
    color: Option<Color32>,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let mut text = button_icon(symbol, enabled).size(16.0);
    if let Some(color) = color {
        text = text.color(color);
    }
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new(text)
                .frame(false)
                .min_size(Vec2::new(26.0, 24.0)),
        )
        .on_hover_text(label)
        .on_disabled_hover_text(label);
    let response = if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}
fn small(ui: &mut egui::Ui, symbol: &str, tooltip: &str) -> bool {
    action_button(ui, true, symbol, tooltip, None).clicked()
}
/// Share state shape and alignment between sidebar sessions and top run tabs.
fn paint_run_marker(ui: &egui::Ui, response: &egui::Response, state: State, light: bool) {
    let center = egui::pos2(
        response.rect.right() - ui.spacing().button_padding.x - 3.5,
        response.rect.center().y,
    );
    let color = state_color(state, light);
    if state.active() {
        ui.painter().circle_filled(center, 2.6, color);
    } else {
        ui.painter()
            .circle_stroke(center, 2.6, egui::Stroke::new(1.0_f32, color));
    }
}

fn state_color(state: State, light: bool) -> Color32 {
    match state {
        State::Starting => tone(light, STARTING_BLUE),
        State::Running | State::Stopping => tone(light, RUNNING_GREEN),
        State::Succeeded => tone(light, GREEN),
        State::Failed => tone(light, RED),
        _ => tone(light, MUTED),
    }
}

fn favorite_button(ui: &mut egui::Ui, selected: bool, tooltip: &str, light: bool) -> bool {
    let family = if selected {
        "action-fill"
    } else {
        "action-outline"
    };
    let text = RichText::new(icon::STAR)
        .family(egui::FontFamily::Name(family.into()))
        .size(16.0);
    let text = if selected {
        text.color(Color32::from_rgb(231, 190, 106))
    } else {
        text.color(tone(light, MUTED))
    };
    ui.add(egui::Button::new(text).frame(false))
        .on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

fn tone(light: bool, dark: Color32) -> Color32 {
    if !light {
        return dark;
    }
    match dark {
        MUTED => Color32::from_rgb(93, 98, 116),
        GREEN => Color32::from_rgb(29, 116, 75),
        RUNNING_GREEN => Color32::from_rgb(24, 125, 62),
        STARTING_BLUE => Color32::from_rgb(37, 99, 210),
        RED => Color32::from_rgb(182, 51, 51),
        ACCENT => Color32::from_rgb(46, 94, 160),
        other => other,
    }
}

fn run_status(ui: &mut egui::Ui, run: &crate::runtime::Run, light: bool) {
    ui.label(
        RichText::new(crate::presentation::run_state_display_for_locale(
            run,
            crate::i18n::current(),
        ))
        .color(state_color(run.state, light)),
    );
    ui.label(
        RichText::new(if run.pid == 0 {
            tr("PID 待分配").to_string()
        } else {
            format!("PID {}", run.pid)
        })
        .color(tone(light, MUTED)),
    );
    let timing = run.timing_for_locale(crate::i18n::current());
    ui.add(egui::Label::new(RichText::new(&timing.display).color(tone(light, MUTED))).wrap())
        .on_hover_text(&timing.display);
    if let Some(code) = run.exit {
        ui.label(tf("退出码 {code}", &[("code", (code).to_string())]));
    }
}

fn sort_tiled_tabs_newest_first(tabs: &mut [crate::presentation::RunTab]) {
    tabs.sort_by_key(|tab| std::cmp::Reverse(tab.id));
}

fn launch_widths(width: f32, item_spacing: f32) -> (f32, f32, f32, f32) {
    let actions = LAUNCH_ACTION_SLOT_WIDTH * LAUNCH_ACTION_SLOT_COUNT as f32;
    let gaps = (LAUNCH_DATA_COLUMN_COUNT + LAUNCH_ACTION_SLOT_COUNT - 1) as f32 * item_spacing;
    let name = (width * 0.24).clamp(100.0, 210.0);
    let port = 92.0;
    let status = 64.0;
    (
        name,
        (width - name - status - port - actions - gaps).max(60.0),
        status,
        port,
    )
}
fn launch_table_column_widths(
    width: f32,
    item_spacing: f32,
    grouped: bool,
    combined: bool,
) -> [f32; 7] {
    let operation_slots = LAUNCH_ACTION_SLOT_COUNT + usize::from(!grouped);
    let operation_width = LAUNCH_ACTION_SLOT_WIDTH * operation_slots as f32
        + item_spacing * (operation_slots - 1) as f32;
    let gaps = if grouped { 5.0 } else { 6.0 };
    let data_width = (width - operation_width - item_spacing * gaps).max(650.0);
    if combined {
        return [
            data_width * 0.20,
            data_width * 0.34,
            0.0,
            data_width * 0.24,
            data_width * 0.10,
            data_width * 0.12,
            operation_width,
        ];
    }
    if grouped {
        return [
            data_width * 0.22,
            data_width * 0.28,
            0.0,
            data_width * 0.27,
            data_width * 0.10,
            data_width * 0.13,
            operation_width,
        ];
    }
    [
        data_width * 0.14,
        data_width * 0.27,
        data_width * 0.12,
        data_width * 0.24,
        data_width * 0.10,
        data_width * 0.13,
        operation_width,
    ]
}
const LAUNCH_ACTION_SLOT_WIDTH: f32 = 28.0;
const LAUNCH_ACTION_SLOT_COUNT: usize = 6;
const LAUNCH_DATA_COLUMN_COUNT: usize = 4;
fn launch_action_slot<R>(
    ui: &mut egui::Ui,
    height: f32,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.allocate_ui_with_layout(
        Vec2::new(LAUNCH_ACTION_SLOT_WIDTH, height),
        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        content,
    )
    .inner
}
fn launch_header(ui: &mut egui::Ui, (name, command, status, port): (f32, f32, f32, f32)) {
    ui.horizontal(|ui| {
        for (label, width) in [
            (tr("启动项"), name),
            (tr("启动命令"), command),
            (tr("状态"), status),
            (tr("端口"), port),
        ] {
            ui.allocate_ui_with_layout(
                Vec2::new(width, 18.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(width);
                    ui.label(RichText::new(label).small().weak());
                },
            );
        }
    });
}

#[cfg(test)]
mod action_cursor_tests {
    use super::*;

    #[test]
    fn launch_columns_reserve_a_fixed_action_region() {
        let width = 1200.0;
        let item_spacing = 8.0;
        let (name, command, status, port) = launch_widths(width, item_spacing);
        assert!(
            (name
                + command
                + status
                + port
                + LAUNCH_ACTION_SLOT_WIDTH * LAUNCH_ACTION_SLOT_COUNT as f32
                + item_spacing * (LAUNCH_DATA_COLUMN_COUNT + LAUNCH_ACTION_SLOT_COUNT - 1) as f32
                - width)
                .abs()
                < 0.01
        );
    }

    #[test]
    fn tiled_consoles_show_newest_start_first() {
        let mut tabs = [8, 12, 10].map(|id| crate::presentation::RunTab {
            console_id: id,
            id,
            label: id.to_string(),
            full_name: id.to_string(),
            tooltip: String::new(),
            state: State::Running,
        });
        sort_tiled_tabs_newest_first(&mut tabs);
        assert_eq!(tabs.map(|tab| tab.id), [12, 10, 8]);
    }

    #[test]
    fn enabled_actions_show_hand_but_disabled_and_disabled_parent_keep_arrow() {
        for (enabled, parent_enabled, expected) in [
            (true, true, egui::CursorIcon::PointingHand),
            (false, true, egui::CursorIcon::Default),
            (true, false, egui::CursorIcon::Default),
        ] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::appearance::fonts(&Preferences::default()));
            let mut target = egui::Pos2::ZERO;
            let draw = |ctx: &egui::Context, target: &mut egui::Pos2| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if !parent_enabled {
                        ui.disable();
                    }
                    *target = action_button(ui, enabled, icon::PLAY, "启动", None)
                        .rect
                        .center();
                });
            };
            let _ = ctx.run(Default::default(), |ctx| draw(ctx, &mut target));
            let input = egui::RawInput {
                events: vec![egui::Event::PointerMoved(target)],
                ..Default::default()
            };
            let frame = ctx.run(input, |ctx| draw(ctx, &mut target));
            assert_eq!(frame.platform_output.cursor_icon, expected);
        }
    }
}
