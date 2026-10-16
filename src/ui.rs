use crate::appearance::{ACCENT, BG, BORDER, GREEN, MUTED, RED, SIDE, SURFACE};
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
#[path = "ui_console_render.rs"]
mod console_render;
#[path = "ui_dialog.rs"]
mod dialog_ui;
#[path = "ui_editor.rs"]
mod editor_ui;
#[path = "ui_console_search.rs"]
mod search_ui;
#[path = "ui_settings.rs"]
mod settings_ui;

#[derive(Clone)]
struct ScriptInfoDraft {
    id: u64,
    name: String,
    description: String,
    default_name: String,
    reset_name: bool,
}
pub struct DevStart {
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
    expanded: HashSet<u64>,
    selected_run: Option<u64>,
    open_console_as_tab: Option<u64>,
    close_run: Option<u64>,
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
}
impl DevStart {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let light = false;
        let (service, mut error) = match Service::load() {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e)),
        };
        let api = service.as_ref().and_then(|s| {
            match crate::api_transport::Server::bind(&s.store.root) {
                Ok(api) => Some(api),
                Err(e) => {
                    eprintln!("[dev start api] {e}");
                    error = Some(format!("本地API不可用: {e}"));
                    None
                }
            }
        });
        let expanded = service
            .as_ref()
            .map(|s| s.catalog.projects.iter().map(|p| p.id).collect())
            .unwrap_or_default();
        let mut app = Self {
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
                            .ok_or("工作空间不可用".to_string())?
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
            .ok_or("工作空间不可用".to_string())
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
                            "展开侧栏 · ⌘B"
                        } else {
                            "收起侧栏 · ⌘B"
                        },
                    ) {
                        self.toggle_sidebar();
                    }
                    if !collapsed {
                        ui.label(RichText::new("DevStart").strong());
                    }
                });
                ui.add_space(12.0);
                for (filter, symbol, name) in [
                    (Filter::All, icon::STACK, "项目"),
                    (Filter::Favorites, icon::STAR, "收藏"),
                    (Filter::Running, icon::TERMINAL_WINDOW, "运行中"),
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
                            format!("运行中 · {count}")
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
                        .map(|s| s.sidebar_sessions())
                        .unwrap_or_default();
                    if !sessions.is_empty() {
                        ui.add_space(16.0);
                        ui.separator();
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("启动会话")
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
                                            format!("{}\n{}", tab.tooltip, tab.state.label()),
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
                        format!("{}   设置", icon::GEAR_SIX)
                    };
                    if ui
                        .add_sized(
                            [ui.available_width(), row_height],
                            egui::Button::new(button_label(ui, &settings, ui.is_enabled()))
                                .right_text(if collapsed { "" } else { "⌘," })
                                .selected(self.filter == Filter::Settings)
                                .frame_when_inactive(self.filter == Filter::Settings),
                        )
                        .on_hover_text("设置 · ⌘,")
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.open_settings();
                    }
                    let data = if collapsed {
                        icon::FOLDER_OPEN.into()
                    } else {
                        format!("{}   数据目录", icon::FOLDER_OPEN)
                    };
                    if ui
                        .add_sized(
                            [ui.available_width(), row_height],
                            egui::Button::new(button_label(ui, &data, ui.is_enabled()))
                                .right_text("")
                                .frame(false),
                        )
                        .on_hover_text("打开数据目录")
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                        && let Some(s) = &self.service
                    {
                        self.report(service::open_file(&s.store.root));
                    }
                    if !collapsed {
                        ui.label(
                            RichText::new("本地工作空间")
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
                                Filter::Favorites => "收藏",
                                Filter::Settings => "设置",
                                _ => "项目",
                            })
                            .strong(),
                        );
                    }
                    if !running && !matches!(self.filter, Filter::Settings | Filter::Editor) {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .button(button_label(
                                    ui,
                                    &format!("{} 添加项目", icon::PLUS),
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
                                    &format!("{} 新建分组", icon::FOLDER_PLUS),
                                    ui.is_enabled(),
                                ))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.group_dialog = true;
                            }
                            ui.add(
                                egui::TextEdit::singleline(&mut self.query)
                                    .hint_text("搜索项目或启动项")
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
                                    &format!("{} 平铺视图", icon::SQUARES_FOUR),
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
                                    &format!("{} 标签视图", icon::TERMINAL_WINDOW),
                                    ui.is_enabled(),
                                ),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.tiled = false;
                        }
                        for (action, symbol, label) in [
                            (crate::bulk::Action::Stop, icon::STOP, "全部停止"),
                            (crate::bulk::Action::Start, icon::PLAY, "全部启动"),
                            (
                                crate::bulk::Action::Restart,
                                icon::ARROW_CLOCKWISE,
                                "全部重启",
                            ),
                        ] {
                            if let Some(plan) = self.service.as_ref().map(|s| s.bulk_plan(action)) {
                                let enabled = plan.enabled() && ui.is_enabled();
                                let hint = if !plan.blocked.is_empty() {
                                    format!("{label}\n{}", plan.blocked.join("；"))
                                } else if plan.targets.is_empty() {
                                    "没有符合条件的标签启动项".into()
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
    fn launch(&mut self, id: u64) {
        if let Some(service) = &mut self.service {
            match service.launch(id) {
                Ok(run) => {
                    self.selected_run = Some(run);
                    self.tiled = true;
                    self.filter = Filter::Running;
                }
                Err(e) => {
                    eprintln!("[dev start] {e}");
                    self.error = Some(e);
                }
            }
        }
    }
    fn run_tabs(&mut self, ui: &mut egui::Ui) {
        let tabs = self
            .service
            .as_ref()
            .map(|s| s.run_tabs())
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
                                if ui.button("独立 Console 窗口").clicked() {
                                    self.console_windows.entry(tab.id).or_default();
                                    ui.close();
                                }
                                if ui.button("平铺独立窗口").clicked() {
                                    self.tile_consoles(ui.ctx());
                                    ui.close();
                                }
                                ui.separator();
                                if ui
                                    .button(button_label(
                                        ui,
                                        &format!("{} 关闭标签", icon::X),
                                        ui.is_enabled(),
                                    ))
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    if tab.state.active() {
                                        self.close_run = Some(tab.id);
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
                ui.label(RichText::new("暂无启动记录").color(tone(self.light, MUTED)));
            });
            if ui.button("选择项目启动").clicked() {
                self.filter = Filter::All;
            }
        }
    }
    fn tiled_consoles(&mut self, ui: &mut egui::Ui) {
        let mut tabs = self
            .service
            .as_ref()
            .map(|s| s.run_tabs())
            .unwrap_or_default();
        if tabs.is_empty() {
            ui.label("暂无启动记录");
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
                    ui.horizontal(|ui| {
                        for tab in row {
                            ui.allocate_ui_with_layout(
                                Vec2::new(width, height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_max_height(height);
                                    egui::Frame::new()
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
                                },
                            );
                        }
                    });
                    ui.add_space(12.0);
                }
            });
    }
    fn projects(&mut self, ui: &mut egui::Ui) {
        let Some(service) = &self.service else {
            ui.label("工作空间不可用");
            return;
        };
        let grouped = service.grouped_projects(self.filter, &self.query);
        egui::ScrollArea::vertical()
            .id_salt("grouped-projects")
            .show(ui, |ui| {
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
                                    egui::Button::new(RichText::new(&group.name).strong())
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
                                    "收起分组"
                                } else {
                                    "展开分组"
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
                                    if ui.button("重命名分组").clicked() {
                                        self.rename_entity =
                                            Some((true, group.id, group.name.clone()));
                                        ui.close();
                                    }
                                })
                                .response
                                .on_hover_text("分组操作");
                            });
                        });
                    });
                    if self.collapsed_groups.contains(&group.id) {
                        continue;
                    }
                    if group_projects.is_empty() {
                        ui.indent(("empty-group", group.id), |ui| {
                            ui.label(
                                RichText::new("暂无项目")
                                    .small()
                                    .color(tone(self.light, MUTED)),
                            );
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
                                            "收起启动项"
                                        } else {
                                            "展开启动项"
                                        },
                                    );
                                    let name_clicked = ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(&project.name).strong(),
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
                                            RichText::new("外部运行 · 参数未识别")
                                                .small()
                                                .color(tone(self.light, ACCENT)),
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            icon_menu(ui, icon::DOTS_THREE, |ui| {
                                                if ui.button("重命名项目").clicked() {
                                                    self.rename_entity = Some((
                                                        false,
                                                        project.id,
                                                        project.name.clone(),
                                                    ));
                                                    ui.close();
                                                }
                                                if ui.button("重新读取启动脚本").clicked() {
                                                    let result = self
                                                        .service
                                                        .as_mut()
                                                        .unwrap()
                                                        .rescan(project.id);
                                                    self.report(result);
                                                    ui.close();
                                                }
                                                if ui.button("添加启动脚本").clicked() {
                                                    self.add_script = Some((
                                                        project.id,
                                                        String::new(),
                                                        "/bin/bash".into(),
                                                    ));
                                                    ui.close();
                                                }
                                                if ui.button("打开项目目录").clicked() {
                                                    self.report(service::open_file(&project.path));
                                                    ui.close();
                                                }
                                                ui.separator();
                                                ui.add_space(2.0);
                                                ui.add(egui::Label::new(
                                                    RichText::new("移动到分组")
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
                                                    if ui.button(&g.name).clicked() {
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
                                            if favorite_button(ui, project.favorite, "收藏项目")
                                            {
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
                                if self.expanded.contains(&project.id) {
                                    ui.indent("launch-options", |ui| {
                                        if !project.scripts.is_empty() {
                                            launch_header(ui);
                                        }
                                        for script in &project.scripts {
                                            self.launch_row(ui, script);
                                        }
                                        if project.scripts.is_empty() {
                                            ui.label(
                                                RichText::new("暂无启动项")
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
    fn launch_row(&mut self, ui: &mut egui::Ui, script: &crate::model::Script) {
        ui.push_id(script.id, |ui| {
            let width = ui.available_width();
            let (name_width, command_width, status_width, port_width) = launch_widths(width);
            let port = self
                .service
                .as_ref()
                .map(|s| s.running_port_cell(script.id))
                .unwrap_or(crate::presentation::PortCell {
                    label: "--".into(),
                    tooltip: "工作空间不可用".into(),
                });
            let active = self.service.as_ref().and_then(|s| s.active_run(script.id));
            let failed = self
                .service
                .as_ref()
                .and_then(|s| s.latest_state(script.id))
                == Some(State::Failed);
            let title = if failed {
                format!(
                    "{} · 失败",
                    crate::presentation::launch_label(script.display_label())
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
                        ui.label(RichText::new(status).color(tone(
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
                let launch_hint = if let Some(reason) = script.launch_block_reason() {
                    format!("未启用：{reason}")
                } else if active.is_some() {
                    "已运行，可停止或重启".into()
                } else {
                    "启动".into()
                };
                if action_button(
                    ui,
                    active.is_none() && script.launch_block_reason().is_none(),
                    icon::PLAY,
                    &launch_hint,
                    Some(tone(self.light, GREEN)),
                )
                .clicked()
                {
                    self.launch(script.id);
                }
                if action_button(ui, active.is_some(), icon::STOP, "停止", None).clicked()
                    && let Some(run) = active
                {
                    self.service.as_mut().unwrap().runtime.stop(run);
                }
                if action_button(
                    ui,
                    self.service
                        .as_ref()
                        .is_some_and(|s| s.can_restart_script(script.id)),
                    icon::ARROW_CLOCKWISE,
                    "重启",
                    None,
                )
                .clicked()
                {
                    match self.service.as_mut().unwrap().restart(script.id) {
                        Ok(run) => {
                            self.selected_run = Some(run);
                            self.filter = Filter::Running;
                        }
                        Err(e) => self.report(Err(e)),
                    }
                }
                if small(ui, icon::PENCIL_SIMPLE, "编辑原脚本文件") {
                    self.open_editor(script.id);
                }
                icon_menu(ui, icon::DOTS_THREE, |ui| {
                    if let Some(run) = active
                        && ui.button("查看 Console").clicked()
                    {
                        self.selected_run = Some(run);
                        self.filter = Filter::Running;
                        ui.close();
                    }
                    if ui.button("另存参数预设").clicked() {
                        self.preset = Some((
                            script.id,
                            format!("{} · 新参数", script.name),
                            script.args.join("\n"),
                            script
                                .expected_port
                                .map(|p| p.to_string())
                                .unwrap_or_default(),
                        ));
                        ui.close();
                    }
                    if ui.button("修改信息").clicked() {
                        self.script_info = Some(ScriptInfoDraft {
                            id: script.id,
                            name: script.display_label().to_owned(),
                            description: script.description.clone(),
                            default_name: script.name.clone(),
                            reset_name: script.display_name.is_none(),
                        });
                        ui.close();
                    }
                    if ui.button("根据执行情况生成简介").clicked() {
                        let result = self
                            .service
                            .as_mut()
                            .unwrap()
                            .generate_description(script.id);
                        self.report(result);
                        ui.close();
                    }
                    if ui
                        .button(if script.favorite {
                            "取消收藏"
                        } else {
                            "收藏启动项"
                        })
                        .clicked()
                    {
                        let result = self.service.as_mut().unwrap().script_favorite(script.id);
                        self.report(result);
                        ui.close();
                    }
                    if ui
                        .button("删除启动项")
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        self.delete_script = Some(script.id);
                        ui.close();
                    }
                });
            });
        });
    }
    fn console(&mut self, ui: &mut egui::Ui, id: u64) {
        let can_find =
            self.selected_run == Some(id) || ui.ctx().viewport_id() != egui::ViewportId::ROOT;
        let find =
            can_find && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F));
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
        let mut stop = false;
        let mut restart = None;
        let mut clear = false;
        let mut open = false;
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
                        300.0
                    } else if tab_root {
                        160.0
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
                                        ui.add(egui::Label::new(title).truncate())
                                            .on_hover_text(&run.title);
                                    } else {
                                        ui.add(
                                            egui::Label::new(RichText::new(&run.title).strong())
                                                .truncate(),
                                        )
                                        .on_hover_text(&run.title);
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
                        if action_button(
                            ui,
                            matches!(run.state, State::Starting | State::Running),
                            icon::STOP,
                            "停止",
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
                            } else if action_button(ui, true, icon::ARROW_CLOCKWISE, "重启", None)
                                .clicked()
                            {
                                restart = Some(run.script_id);
                            }
                        });
                        if tiled_root {
                            if small(ui, icon::CORNERS_OUT, "在标签视图中打开当前 Console")
                            {
                                self.open_console_as_tab = Some(id);
                            }
                            if small(ui, icon::MAGNIFYING_GLASS, "搜索日志 · ⌘F") {
                                search.toggle();
                            }
                            clear = small(ui, icon::ERASER, "清空视图");
                            open = small(ui, icon::FILE_TEXT, "打开日志文件");
                        } else if tab_root {
                            if small(ui, icon::MAGNIFYING_GLASS, "搜索日志 · ⌘F") {
                                search.toggle();
                            }
                            clear = small(ui, icon::ERASER, "清空视图");
                            open = small(ui, icon::FILE_TEXT, "打开日志文件");
                        }
                    });
                });
                if compact {
                    ui.horizontal_wrapped(|ui| run_status(ui, run, self.light));
                }
                if tiled_root || tab_root {
                    if !compact {
                        ui.horizontal(|ui| {
                            let preview =
                                crate::presentation::command_preview(&run.display_command);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&preview)
                                        .monospace()
                                        .color(tone(self.light, MUTED)),
                                )
                                .truncate(),
                            )
                            .on_hover_text(crate::presentation::command_preview(&run.command));
                        });
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if small(ui, icon::MAGNIFYING_GLASS, "搜索日志 · ⌘F") {
                                search.toggle();
                            }

                            clear = small(ui, icon::ERASER, "清空视图");
                            open = small(ui, icon::FILE_TEXT, "打开日志文件");
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
        ui.add_space(4.0);
        let input = search_ui::controls(ui, search, id);
        let view =
            match s.runtime.runs[index]
                .lines
                .query(&search.query, search.current, input.direction)
            {
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
                RichText::new("仅显示最近输出；日志文件也有保留上限")
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
                    ui.label(RichText::new("没有匹配的日志").color(tone(self.light, MUTED)));
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
        self.report(restart_result);
        if open {
            self.report(service::open_file(&path));
        }
    }
    fn tile_consoles(&mut self, ctx: &egui::Context) {
        let ids: Vec<_> = self
            .service
            .as_ref()
            .map(|s| {
                s.runtime
                    .runs
                    .iter()
                    .filter(|run| run.state.active() || self.console_windows.contains_key(&run.id))
                    .map(|run| run.id)
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
        for id in ids {
            let Some((title, console_id)) = self
                .service
                .as_ref()
                .and_then(|s| s.runtime.runs.iter().find(|r| r.id == id))
                .map(|r| (r.title.clone(), r.console_id))
            else {
                self.console_windows.remove(&id);
                continue;
            };
            let mut view = self.console_windows.remove(&id).unwrap_or_default();
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
                        self.error = Some("当前图形后端不支持独立 Console 窗口".into());
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
                            ui.push_id(("console-window", id), |ui| self.console(ui, id));
                        });
                    child.request_repaint_after(Duration::from_millis(100));
                },
            );
            if !closed {
                let current = self
                    .service
                    .as_ref()
                    .and_then(|s| s.current_console_run(id))
                    .unwrap_or(id);
                self.console_windows.insert(current, view);
            }
        }
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
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
                dialog_ui::window(ctx, "删除启动项")
                    .open(&mut open)
                    .collapsible(false)
                    .resizable(false)
                    .default_width(360.0)
                    .show(ctx, |ui| {
                        dialog_ui::content(ui);
                        ui.label(format!("{project} / {name}"));
                        ui.label("确认删除此启动项？");
                        ui.label(
                            RichText::new("保留原脚本文件和日志。").color(tone(self.light, MUTED)),
                        );
                        if let Some(reason) = &blocked {
                            ui.label(RichText::new(reason).color(tone(self.light, RED)));
                        }
                        dialog_ui::actions(ui, |ui| {
                            let response = ui.add_enabled(
                                blocked.is_none(),
                                egui::Button::new(
                                    RichText::new("删除").color(tone(self.light, RED)),
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
                                .button("取消")
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
                        crate::bulk::Action::Stop => ("确认全部停止？", "将停止", "确认全部停止"),
                        crate::bulk::Action::Start => ("确认全部启动？", "将启动", "确认全部启动"),
                        crate::bulk::Action::Restart => {
                            ("确认全部重启？", "将重启", "确认全部重启")
                        }
                    };
                    ui.heading(title);
                    ui.label(format!("{verb} {} 个标签启动项", plan.targets.len()));
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
                                .ok_or("工作空间不可用".to_string())
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
                        if ui.button("取消").clicked() {
                            self.bulk_confirm = None;
                        }
                    });
                });
        }

        if let Some(id) = self.close_run {
            egui::Modal::new(egui::Id::new("close-running-tab"))
                .frame(dialog_ui::frame(ctx))
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.heading("是否停止并关闭？");
                    if let Some(run) = self
                        .service
                        .as_ref()
                        .and_then(|s| s.runtime.runs.iter().find(|r| r.id == id))
                    {
                        ui.label(&run.title);
                    }
                    dialog_ui::actions(ui, |ui| {
                        if ui.button("是，停止并关闭").clicked() {
                            if let Some(s) = &mut self.service {
                                let result = s.runtime.close_tab(id, true);
                                self.report(result);
                            }
                            self.close_run = None;
                        }
                        if ui.button("取消").clicked() {
                            self.close_run = None;
                        }
                    });
                });
        }

        if let Some((source, mut name, mut args, mut port)) = self.preset.clone() {
            dialog_ui::window(ctx, "另存参数预设")
                .collapsible(false)
                .default_width(500.0)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label("启动项名称");
                    ui.text_edit_singleline(&mut name);
                    ui.label("参数（每行一个，原样传递）");
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
                    ui.label("端口约束（可选）");
                    ui.text_edit_singleline(&mut port);
                    self.preset = Some((source, name.clone(), args.clone(), port.clone()));
                    dialog_ui::actions(ui, |ui| {
                        if ui.button("保存启动项").clicked() {
                            let result = self
                                .service
                                .as_mut()
                                .ok_or("工作空间不可用".to_string())
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
                        if ui.button("取消").clicked() {
                            self.preset = None;
                        }
                    });
                });
        }

        if let Some((group, id, mut name)) = self.rename_entity.clone() {
            dialog_ui::window(
                ctx,
                if group {
                    "重命名分组"
                } else {
                    "重命名项目"
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
                    if ui.button("保存").clicked() {
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
                    if ui.button("取消").clicked() {
                        self.rename_entity = None;
                    }
                });
            });
        }
        if self.group_dialog {
            dialog_ui::window(ctx, "新建分组")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.add(egui::TextEdit::singleline(&mut self.group_name).hint_text("分组名称"));
                    dialog_ui::actions(ui, |ui| {
                        if ui.button("创建").clicked()
                            && let Some(s) = &mut self.service
                        {
                            let result = s.add_group(&self.group_name);
                            if result.is_ok() {
                                self.group_dialog = false;
                                self.group_name.clear();
                            }
                            self.report(result);
                        }
                        if ui.button("取消").clicked() {
                            self.group_dialog = false;
                        }
                    });
                });
        }
        if self.import_dialog {
            dialog_ui::window(ctx, "添加项目")
                .collapsible(false)
                .resizable(false)
                .default_width(500.0)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label("项目目录");
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [380.0, 32.0],
                            egui::TextEdit::singleline(&mut self.import_path)
                                .hint_text("项目绝对路径"),
                        );
                        if ui.button("选择").clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_folder()
                        {
                            self.import_path = path.display().to_string();
                        }
                    });
                    if let Some(s) = &self.service {
                        egui::ComboBox::from_label("分组")
                            .selected_text(
                                s.catalog
                                    .groups
                                    .iter()
                                    .find(|g| g.id == self.import_group)
                                    .map(|g| g.name.as_str())
                                    .unwrap_or("默认分组"),
                            )
                            .show_ui(ui, |ui| {
                                for g in &s.catalog.groups {
                                    ui.selectable_value(&mut self.import_group, g.id, &g.name);
                                }
                            });
                    }
                    dialog_ui::actions(ui, |ui| {
                        if ui.button("读取启动脚本").clicked()
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
                        if ui.button("取消").clicked() {
                            self.import_dialog = false;
                        }
                    });
                });
        }
        if let Some(mut draft) = self.script_info.clone() {
            let mut open = true;
            dialog_ui::window(ctx, "修改信息").open(&mut open).collapsible(false).resizable(false).default_width(440.0).show(ctx,|ui|{
                dialog_ui::content(ui);
                ui.horizontal(|ui|{
                    ui.label("显示名");
                    let response=ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(270.0)).on_hover_text("最多50个字符");
                    if response.changed(){draft.reset_name=false;}
                    if ui.button("恢复默认").on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){draft.name=draft.default_name.clone();draft.reset_name=true;}
                });
                ui.label("简介");
                ui.add(egui::TextEdit::multiline(&mut draft.description).desired_width(f32::INFINITY).desired_rows(4));
                self.script_info=Some(draft.clone());
                dialog_ui::actions(ui,|ui|{
                    if ui.button("保存").on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){
                        let name=if draft.reset_name {None}else{Some(draft.name.clone())};
                        let result=crate::api::dispatch(self.service.as_mut().unwrap(),"scripts.update",&serde_json::json!({"id":draft.id,"patch":{"display_name":name,"description":draft.description}})).map(|_|());
                        if result.is_ok(){self.script_info=None;}
                        self.report(result);
                    }
                    if ui.button("取消").on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){self.script_info=None;}
                });
            });
            if !open {
                self.script_info = None;
            }
        }
        if let Some((id, mut path, mut interpreter)) = self.add_script.clone() {
            dialog_ui::window(ctx, "添加启动脚本")
                .collapsible(false)
                .default_width(480.0)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label("原始脚本路径");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut path);
                        if ui.button("选择").clicked()
                            && let Some(file) = rfd::FileDialog::new().pick_file()
                        {
                            path = file.display().to_string();
                        }
                    });
                    ui.label("解释器（例如 /bin/bash、python3）");
                    ui.text_edit_singleline(&mut interpreter);
                    self.add_script = Some((id, path.clone(), interpreter.clone()));
                    dialog_ui::actions(ui, |ui| {
                        if ui.button("添加").clicked() {
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
                        if ui.button("取消").clicked() {
                            self.add_script = None;
                        }
                    });
                });
        }
        if let Some(error) = self.error.clone() {
            dialog_ui::window(ctx, "操作失败")
                .collapsible(false)
                .default_width(520.0)
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.label(RichText::new(&error).color(tone(self.light, RED)));
                    dialog_ui::actions(ui, |ui| {
                        if ui.button("复制错误").clicked() {
                            ctx.copy_text(error);
                        }
                        if ui.button("关闭").clicked() {
                            self.error = None;
                        }
                    });
                });
        }
    }
}
impl eframe::App for DevStart {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.editor_quit_guard(ctx);
        if let Some(s) = &mut self.service {
            let count = s.runtime.runs.len();
            s.poll();
            if let Some(api) = &self.api {
                api.poll(s);
            }
            if s.runtime.runs.len() > count && self.filter == Filter::Running {
                self.selected_run = None;
            }
            let moved: Vec<_> = self
                .console_windows
                .keys()
                .filter_map(|old| {
                    s.current_console_run(*old)
                        .filter(|new| new != old)
                        .map(|new| (*old, new))
                })
                .collect();
            for (old, new) in moved {
                if let Some(view) = self.console_windows.remove(&old) {
                    self.console_windows.insert(new, view);
                }
            }
            let retained: HashSet<_> = s.runtime.runs.iter().map(|r| r.id).collect();
            self.console_layouts
                .retain(|(_, id), _| retained.contains(id));
            self.console_queries.retain(|id, _| retained.contains(id));
            self.console_windows.retain(|id, _| retained.contains(id));
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
                } else if self.filter == Filter::Running {
                    self.running_content(ui);
                } else {
                    self.projects(ui);
                }
            });
        self.editor_dialogs(ctx);
        self.dialogs(ctx);
        self.show_console_windows(ctx);
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
        ("●", tone(light, GREEN)),
        (" 运行中    ", tone(light, MUTED)),
        ("○", tone(light, MUTED)),
        (" 已停止", tone(light, MUTED)),
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
        &run.title,
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
        State::Starting => tone(light, ACCENT),
        State::Running | State::Succeeded => tone(light, GREEN),
        State::Failed => tone(light, RED),
        _ => tone(light, MUTED),
    }
}

fn favorite_button(ui: &mut egui::Ui, selected: bool, tooltip: &str) -> bool {
    let text = button_icon(icon::STAR, ui.is_enabled()).size(16.0);
    let text = if selected {
        text.color(Color32::from_rgb(231, 190, 106))
    } else {
        text
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
        RED => Color32::from_rgb(182, 51, 51),
        ACCENT => Color32::from_rgb(46, 94, 160),
        other => other,
    }
}

fn run_status(ui: &mut egui::Ui, run: &crate::runtime::Run, light: bool) {
    ui.label(
        RichText::new(crate::presentation::run_state_display(run))
            .color(state_color(run.state, light)),
    );
    ui.label(
        RichText::new(if run.pid == 0 {
            "PID 待分配".to_string()
        } else {
            format!("PID {}", run.pid)
        })
        .color(tone(light, MUTED)),
    );
    let timing = run.timing();
    ui.add(egui::Label::new(RichText::new(&timing.display).color(tone(light, MUTED))).wrap())
        .on_hover_text(&timing.display);
    if let Some(code) = run.exit {
        ui.label(format!("退出码 {code}"));
    }
}

fn sort_tiled_tabs_newest_first(tabs: &mut [crate::presentation::RunTab]) {
    tabs.sort_by_key(|tab| std::cmp::Reverse(tab.id));
}

fn launch_widths(width: f32) -> (f32, f32, f32, f32) {
    let name = (width * 0.24).clamp(100.0, 210.0);
    let port = 92.0;
    let status = 64.0;
    (
        name,
        (width - name - status - port - 196.0).max(60.0),
        status,
        port,
    )
}
fn launch_header(ui: &mut egui::Ui) {
    let (name, command, status, port) = launch_widths(ui.available_width());
    ui.horizontal(|ui| {
        for (label, width) in [
            ("启动项", name),
            ("启动命令", command),
            ("状态", status),
            ("端口", port),
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
