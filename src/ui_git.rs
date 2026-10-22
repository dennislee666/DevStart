//! Three Git panes: project selection, working tree/actions, and paged history.
//! All repository inspection and mutations run on a worker through shared backend functions.
use super::*;
#[path = "ui_git_commit.rs"]
mod commit_ui;
#[path = "ui_git_merge.rs"]
mod merge_ui;
#[path = "ui_git_remote.rs"]
mod remote_ui;
use crate::git_workspace::{self as git, Repository, actions};
#[derive(Default)]
pub(super) struct GitPage {
    pub(super) terminal: terminal_ui::TerminalView,
    remote_choices: HashMap<u64, String>,
    rows: Vec<Repository>,
    pub(super) selected: Option<u64>,
    details: Option<Box<Details>>,
    detail_cache: HashMap<u64, Box<Details>>,
    pending: Option<std::sync::mpsc::Receiver<Reply>>,
    loaded: bool,
    repaint: Option<egui::Context>,
    error: Option<String>,
    pub(super) output: String,
    message: String,
    generating_message: bool,
    selection: git::selection::FileSelection,
    diffs: Vec<crate::git_diff::Document>,
    diff_active: usize,
    diff_open: bool,
    diff_minimized: bool,
    diff_maximized: bool,
    browsing_commit: Option<String>,
    commit_files: Option<Box<git::commits::Inspection>>,
    selected_commit_file: Option<String>,
    confirm: Option<Action>,
    remote_picker: Option<remote_ui::Picker>,
    preview_error: Option<String>,
    notices: Vec<(bool, String)>,
    merge_picker: Option<merge_ui::Picker>,
}
type Details = git::Snapshot;
enum Reply {
    List(Vec<Repository>),
    Generated(
        u64,
        crate::preferences::CommitAgent,
        String,
        Result<git::ai_message::Generated, String>,
    ),
    Submitted(u64, Result<Box<Details>, String>, bool, String),
    Selected(u64, Result<Box<Details>, String>, Option<String>),
    Diff(Result<crate::git_diff::Document, String>),
    WorkingDiff(Result<crate::git_diff::Document, String>),
    Commit(Result<Box<git::commits::Inspection>, String>),
}
#[derive(Clone)]
enum Action {
    Switch(String),
    Merge(String, String),
    Restore(String),
    RestoreVersion(String, String, String),
    Commit,
    Push(String, String),
    Update(String, String),
}
fn details(project: &crate::model::Project, offset: usize) -> Result<Box<Details>, String> {
    git::snapshot(project, offset)
}
impl GitPage {
    fn request(&mut self, work: impl FnOnce() -> Reply + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending = Some(rx);
        self.error = None;
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            // Wake the view when data arrives instead of waiting for the 100 ms UI tick.
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        });
    }
    fn append_output(&mut self, text: &str) {
        self.output.push_str(text);
        self.output.push(char::from(10));
        // Keep command feedback bounded independently of process Console retention.
        if self.output.len() > 262144 {
            let mut start = self.output.len() - 262144;
            while !self.output.is_char_boundary(start) {
                start += 1;
            }
            self.output.drain(..start);
        }
    }
}
impl GitPage {
    pub(super) fn push_notice(&mut self, success: bool, message: String) {
        self.notices.push((success, message));
    }
}
impl DevStart {
    pub(super) fn git_request_pending(&self) -> bool {
        self.git_page.pending.is_some()
    }
    fn git_refresh(&mut self) {
        let projects = self
            .service
            .as_ref()
            .map(|s| s.catalog.projects.clone())
            .unwrap_or_default();
        self.git_page
            .request(move || Reply::List(git::list(&projects)));
    }
    pub(super) fn git_select(&mut self, id: u64, offset: usize) {
        let Some(project) = self
            .service
            .as_ref()
            .and_then(|s| s.catalog.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            return;
        };
        if self.git_page.selected != Some(id) {
            self.git_page.browsing_commit = None;
            self.git_page.commit_files = None;
            self.git_page.selected_commit_file = None;
            self.git_page.selection.clear();
            self.git_page.message.clear();
            self.git_page.output.clear();

            self.git_page.details = self
                .git_page
                .detail_cache
                .get(&id)
                .filter(|d| d.repository.path == project.path && d.offset == offset)
                .cloned();
            if let Some(details) = &self.git_page.details {
                self.git_page.selection.update(&details.files);
            }
        }
        self.git_page.selected = Some(id);
        let directory = project.path.clone();
        self.git_page
            .request(move || Reply::Selected(id, details(&project, offset), None));
        if let Some(service) = &mut self.service {
            match service.terminals.open(id, &directory) {
                Ok(session) => {
                    self.git_page.terminal.attach(session);
                }
                Err(error) => self.git_page.error = Some(error),
            }
        }
    }
    fn git_generate_message(&mut self) {
        let Some(id) = self.git_page.selected else {
            return;
        };
        let Some(service) = &self.service else {
            return;
        };
        let Some(project) = service.catalog.projects.iter().find(|p| p.id == id) else {
            return;
        };
        let path = project.path.clone();
        let root = service.store.root.clone();
        let preference = service.catalog.preferences.commit_agent;
        let english = crate::i18n::current() == crate::i18n::Locale::English;
        let files: Vec<_> = self.git_page.selection.selected.iter().cloned().collect();
        let previous = self.git_page.message.clone();
        self.git_page.generating_message = true;
        self.git_page.request(move || {
            Reply::Generated(
                id,
                preference,
                previous,
                git::ai_message::generate(&path, &files, preference, &root, english),
            )
        });
    }
    fn git_action(&mut self, action: Action) {
        let Some(id) = self.git_page.selected else {
            return;
        };
        let Some(project) = self
            .service
            .as_ref()
            .and_then(|s| s.catalog.projects.iter().find(|p| p.id == id))
            .cloned()
        else {
            return;
        };
        let backup = self
            .service
            .as_ref()
            .unwrap()
            .store
            .root
            .join("git-backups");
        let message = self.git_page.message.clone();
        let files: Vec<_> = self.git_page.selection.selected.iter().cloned().collect();
        let destination = self.git_page.remote_choices.get(&id).cloned().zip(
            self.git_page
                .details
                .as_ref()
                .map(|d| d.repository.branch.clone()),
        );
        self.git_page.request(move || {
            if matches!(action, Action::Commit | Action::Push(_, _)) {
                let push = matches!(action, Action::Push(_, _));
                let result = match &action {
                    Action::Push(key, branch) => {
                        git::remotes::submit(&project.path, key, branch, &message, &files, true)
                    }
                    _ => match destination {
                        Some((key, branch)) => git::remotes::submit(
                            &project.path,
                            &key,
                            &branch,
                            &message,
                            &files,
                            false,
                        ),
                        None => Err("No remote selected; select a remote before submitting".into()),
                    },
                };
                let success = result.is_ok();
                let operation = if push {
                    "Update → Commit → Push"
                } else {
                    "Update → Commit"
                };
                let output = format!(
                    "{} · {operation}\n{}",
                    project.name,
                    result.unwrap_or_else(|error| error)
                );
                return Reply::Submitted(id, details(&project, 0), success, output);
            }
            let notification = match &action {
                Action::Update(_, _) => Some("Update Project"),
                Action::Merge(_, _) => Some("git merge"),
                Action::RestoreVersion(_, _, _) => Some("Restore version"),
                Action::Switch(_) => Some("git switch"),
                _ => None,
            };
            let (label, result) = match action {
                Action::Merge(source, target) => (
                    format!("git merge {source} → {target}"),
                    git::merging::merge(&project.path, &source, &target),
                ),
                Action::Switch(branch) => (
                    format!("git switch {branch}"),
                    git::switch(&project.path, &branch).map(|_| format!("Switched to {branch}")),
                ),
                Action::RestoreVersion(commit, branch, head) => (
                    format!("restore version {commit}"),
                    git::version_restore::restore(&project.path, &commit, &branch, &head),
                ),
                Action::Restore(file) => (
                    format!("restore {file}"),
                    actions::restore(&project.path, &file, &backup),
                ),
                Action::Commit => (
                    "git commit".into(),
                    actions::commit(&project.path, &message, &files),
                ),
                Action::Push(key, branch) => (
                    format!("git push {branch}"),
                    git::remotes::push(&project.path, &key, &branch),
                ),
                Action::Update(key, branch) => (
                    format!("git pull (merge) {branch}"),
                    git::remotes::update(&project.path, &key, &branch),
                ),
            };
            let success = result.is_ok();
            let output = match result {
                Ok(out) => format!(
                    "$ {label}
{out}"
                ),
                Err(e) => {
                    eprintln!("[dev start git] {e}");
                    format!(
                        "$ {label}
[error] {e}"
                    )
                }
            };
            if let Some(operation) = notification {
                Reply::Submitted(
                    id,
                    details(&project, 0),
                    success,
                    format!("{} · {operation}\n{output}", project.name),
                )
            } else {
                Reply::Selected(id, details(&project, 0), Some(output))
            }
        });
    }
    fn git_poll(&mut self) {
        let reply = match self.git_page.pending.as_ref().map(|r| r.try_recv()) {
            Some(Ok(reply)) => Some(reply),
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.git_page.pending = None;
                self.git_page.error = Some(tr("Git 后台任务中断").into());
                self.git_page.generating_message = false;
                None
            }
            _ => None,
        };
        if let Some(reply) = reply {
            self.git_page.pending = None;
            let reply = match reply {
                Reply::Submitted(id, snapshot, success, output) => {
                    self.git_page.notices.push((success, output.clone()));
                    Reply::Selected(id, snapshot, Some(output))
                }
                other => other,
            };
            match reply {
                Reply::Generated(id, preference, previous, result) => {
                    self.git_page.generating_message = false;
                    match result {
                        Ok(generated) => {
                            if let Some(service) = &mut self.service {
                                match service.remember_commit_agent(preference, generated.agent) {
                                    Ok(true) => {
                                        if let Some(draft) = &mut self.settings_draft
                                            && draft.commit_agent == preference
                                        {
                                            draft.commit_agent = generated.agent;
                                        }
                                    }
                                    Ok(false) => {}
                                    Err(error) => self.git_page.notices.push((
                                        false,
                                        format!("{}: {error}", tr("生成工具设置保存失败")),
                                    )),
                                }
                            }
                            if self.git_page.selected == Some(id)
                                && self.git_page.message == previous
                            {
                                self.git_page.message = generated.message;
                            } else {
                                self.git_page.notices.push((
                                    false,
                                    format!(
                                        "{}\n{}",
                                        tr("备注已被修改，生成结果未覆盖当前内容"),
                                        generated.message
                                    ),
                                ));
                            }
                        }
                        Err(error) => self
                            .git_page
                            .notices
                            .push((false, format!("{}\n{error}", tr("生成提交备注失败")))),
                    }
                }
                Reply::Submitted(..) => unreachable!("submission normalized above"),
                Reply::List(rows) => {
                    self.git_page.rows = rows;
                    self.git_page.loaded = true;
                    if let Some(id) = self.git_page.selected {
                        if self.git_page.rows.iter().any(|r| r.project_id == id) {
                            self.git_select(id, 0);
                        } else {
                            self.git_page.selected = None;
                            self.git_page.details = None;
                        }
                    }
                }
                Reply::Commit(result) => match result {
                    Ok(files) => self.git_page.commit_files = Some(files),
                    Err(error) => self.git_page.error = Some(error),
                },
                Reply::WorkingDiff(result) => match result {
                    Ok(doc) => {
                        self.git_page.diffs = vec![doc];
                        self.git_page.diff_active = 0;
                        self.git_page.diff_open = true;
                        self.git_page.diff_minimized = false;
                    }
                    Err(error) => self.git_page.preview_error = Some(error),
                },
                Reply::Diff(result) => match result {
                    Ok(v) => {
                        if let Some(i) = self.git_page.diffs.iter().position(|d| d.key == v.key) {
                            self.git_page.diffs[i] = v;
                            self.git_page.diff_active = i;
                        } else {
                            if self.git_page.diffs.len() == 10 {
                                self.git_page.diffs.remove(0);
                            }
                            self.git_page.diffs.push(v);
                            self.git_page.diff_active = self.git_page.diffs.len() - 1;
                        }
                        self.git_page.diff_open = true;
                        self.git_page.diff_minimized = false;
                    }
                    Err(e) => self.git_page.error = Some(e),
                },
                Reply::Selected(id, result, output) => {
                    if let Some(text) = output {
                        self.git_page.append_output(&text);
                    }
                    match result {
                        Ok(d) => {
                            let selected = self.git_page.remote_choices.get(&id);
                            if !d.remotes.iter().any(|r| Some(&r.key) == selected) {
                                if let Some(remote) = d.remotes.iter().find(|r| r.preferred) {
                                    self.git_page.remote_choices.insert(id, remote.key.clone());
                                } else {
                                    self.git_page.remote_choices.remove(&id);
                                }
                            }

                            if let Some(row) =
                                self.git_page.rows.iter_mut().find(|r| r.project_id == id)
                            {
                                *row = d.repository.clone();
                            }
                            self.git_page.selection.update(&d.files);
                            // Cache at most eight visited workspaces; refresh on every selection.
                            if !self.git_page.detail_cache.contains_key(&id)
                                && self.git_page.detail_cache.len() >= 8
                                && let Some(old) = self
                                    .git_page
                                    .detail_cache
                                    .keys()
                                    .copied()
                                    .find(|key| *key != id)
                            {
                                self.git_page.detail_cache.remove(&old);
                            }
                            self.git_page.detail_cache.insert(id, d.clone());
                            self.git_page.details = Some(d);
                        }
                        Err(e) => {
                            self.git_page.error = Some(e);
                            self.git_page.details = None;
                        }
                    }
                }
            }
        }
    }
    pub(super) fn git_page(&mut self, ui: &mut egui::Ui) {
        self.git_page.repaint = Some(ui.ctx().clone());

        if !self.git_page.loaded && self.git_page.pending.is_none() {
            self.git_refresh();
        }
        let busy = self.git_page.pending.is_some();
        let mut selection = None;
        let mut action = None;
        let mut preview = None;
        let mut page = None;
        let mut inspect = None;
        let mut historical_diff = None;
        let mut generate_message = false;
        let height = ui.available_height();
        // Resizable native panes retain usable widths; horizontal scrolling protects narrow windows.
        egui::SidePanel::left("git-project-pane")
            .default_width(190.0)
            .min_width(150.0)
            .max_width(320.0)
            .resizable(true)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(tr("项目"));
                    if ui
                        .add_enabled(!busy, egui::Button::new(tr("刷新")))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        selection = Some(None);
                    }
                    if busy {
                        ui.spinner();
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("git-project-list")
                    .show(ui, |ui| {
                        if self.git_page.loaded && self.git_page.rows.is_empty() {
                            ui.label(tr("暂无项目"));
                        }
                        for row in &self.git_page.rows {
                            ui.add_space(4.0);
                            let selected = self.git_page.selected == Some(row.project_id);
                            let frame = egui::Frame::new()
                                .fill(if selected {
                                    tone(self.light, SURFACE)
                                } else {
                                    Color32::TRANSPARENT
                                })
                                .corner_radius(4.0)
                                .inner_margin(egui::Margin::symmetric(6, 3))
                                .show(ui, |ui| {
                                    // The whole two-line project item is a single selection target.
                                    ui.set_width(ui.available_width());
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.add(
                                        egui::Label::new(RichText::new(&row.name).strong())
                                            .truncate()
                                            .selectable(false),
                                    );
                                    let branch = row
                                        .error
                                        .as_ref()
                                        .map_or(row.branch.as_str(), |_| tr("Git 不可用"));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(branch)
                                                .small()
                                                .color(tone(self.light, MUTED)),
                                        )
                                        .truncate()
                                        .selectable(false),
                                    )
                                    .on_hover_text(row.error.as_deref().unwrap_or(branch));
                                });
                            let response = frame
                                .response
                                .interact(if busy {
                                    egui::Sense::hover()
                                } else {
                                    egui::Sense::click()
                                })
                                .on_hover_cursor(if busy {
                                    egui::CursorIcon::Default
                                } else {
                                    egui::CursorIcon::PointingHand
                                })
                                .on_hover_text(service::path_label(&row.path));
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::SelectableLabel,
                                    !busy,
                                    selected,
                                    &row.name,
                                )
                            });
                            if !busy && response.clicked() {
                                selection = Some(Some(row.project_id));
                            }
                        }
                    });
            });
        if self.git_page.selected.is_some() {
            let browsing = self.git_page.browsing_commit.is_some();
            if browsing {
                egui::SidePanel::left("git-commit-history-pane")
                    .default_width(350.0)
                    .min_width(260.0)
                    .max_width(560.0)
                    .resizable(true)
                    .show_inside(ui, |ui| {
                        let event = self.git_history_contents(ui, height, busy);
                        inspect = event.commit;
                        page = event.offset;
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().inner_margin(10.0))
                    .show_inside(ui, |ui| {
                        historical_diff = self.git_commit_files_view(ui, busy);
                    });
            } else {
                egui::SidePanel::right("git-history-pane")
                    .default_width(350.0)
                    .min_width(260.0)
                    .max_width(560.0)
                    .resizable(true)
                    .show_inside(ui, |ui| {
                        let event = self.git_history_contents(ui, height, busy);
                        inspect = event.commit;
                        page = event.offset;
                    });
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().inner_margin(10.0))
                    .show_inside(ui, |ui| {
                        if let Some(error) = &self.git_page.error {
                            ui.colored_label(tone(self.light, RED), error);
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("git-working-pane")
                            .show(ui, |ui| {
                                if let Some(d) = &self.git_page.details {
                                    let r = &d.repository;
                                    ui.strong(&r.name);
                                    ui.label(
                                        RichText::new(
                                            r.root
                                                .as_ref()
                                                .map(|p| service::path_label(p))
                                                .unwrap_or_default(),
                                        )
                                        .small()
                                        .color(tone(self.light, MUTED)),
                                    );
                                    let remote_key =
                                        self.git_page.remote_choices.get(&r.project_id).cloned();
                                    let remote = d
                                        .remotes
                                        .iter()
                                        .find(|v| Some(&v.key) == remote_key.as_ref());
                                    ui.horizontal_wrapped(|ui| {
                                        ui.add_enabled_ui(!busy, |ui| {
                                            let label = format!(
                                                "{} {} {}",
                                                icon::GIT_BRANCH,
                                                r.branch,
                                                icon::CARET_DOWN
                                            );
                                            let label = button_label(ui, &label, !busy);
                                            ui.menu_button(label, |ui| {
                                                for branch in &d.branches {
                                                    if ui
                                                        .selectable_label(
                                                            branch == &r.branch,
                                                            branch,
                                                        )
                                                        .on_hover_cursor(
                                                            egui::CursorIcon::PointingHand,
                                                        )
                                                        .clicked()
                                                    {
                                                        if branch != &r.branch {
                                                            action = Some(Action::Switch(
                                                                branch.clone(),
                                                            ));
                                                        }
                                                        ui.close();
                                                    }
                                                }
                                            })
                                            .response
                                            .on_hover_text(tr("切换分支"))
                                            .on_hover_cursor(egui::CursorIcon::PointingHand);
                                        });
                                        let enabled = !busy
                                            && remote.is_some()
                                            && !r.branch.starts_with("detached ");
                                        if ui
                                            .add_enabled(
                                                enabled,
                                                egui::Button::new(button_label(
                                                    ui,
                                                    &format!("{} Update Project", icon::ARROW_DOWN),
                                                    enabled,
                                                )),
                                            )
                                            .on_hover_cursor(if enabled {
                                                egui::CursorIcon::PointingHand
                                            } else {
                                                egui::CursorIcon::Default
                                            })
                                            .on_hover_text(tr(
                                                "从当前远程仓库当前分支获取最新版本与本地合并代码",
                                            ))
                                            .clicked()
                                            && let Some(remote) = remote
                                        {
                                            action = Some(Action::Update(
                                                remote.key.clone(),
                                                r.branch.clone(),
                                            ));
                                        }
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            let enabled = !busy && d.branches.len() > 1;
                                            if ui.add_enabled(enabled, egui::Button::new(button_label(ui, &format!("{} {}", icon::GIT_MERGE, tr("合并分支")), enabled)))
                                                .on_hover_text(tr("选择来源与目标分支，在本地仓库执行合并"))
                                                .on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default }).clicked() {
                                                self.git_page.merge_picker = Some(merge_ui::Picker::new(r.project_id, &r.branch, &d.branches));
                                            }
                                        });
                                        let label = remote
                                            .map(|v| format!("{} · {}", v.name, v.url))
                                            .unwrap_or_else(|| tr("未配置远程仓库").into());
                                        if ui
                                            .add_enabled(
                                                !busy && !d.remotes.is_empty(),
                                                egui::Button::new(
                                                    RichText::new(&label)
                                                        .small()
                                                        .color(tone(self.light, MUTED)),
                                                )
                                                .frame(false)
                                                .wrap_mode(egui::TextWrapMode::Truncate),
                                            )
                                            .on_hover_text(&label)
                                            .on_hover_cursor(if !busy && !d.remotes.is_empty() {
                                                egui::CursorIcon::PointingHand
                                            } else {
                                                egui::CursorIcon::Default
                                            })
                                            .clicked()
                                        {
                                            self.git_page.remote_picker = Some(
                                                remote_ui::Picker::new(r.project_id, remote_key),
                                            );
                                        }
                                    });
                                    ui.separator();
                                    ui.horizontal(|ui| {
                                        ui.strong(tr("文件变动"));
                                        let state = self.git_page.selection.state(&d.files);
                                        let mut all = state.all;
                                        if ui.add_enabled(!busy && !state.empty, egui::Checkbox::new(&mut all, tr("全选")).indeterminate(state.partial))
                                            .on_hover_text(tr("选中或取消所有分组中的全部文件，包括已收起的分组"))
                                            .changed() { self.git_page.selection.set_all(&d.files, all); }
                                    });
                                    if d.files.is_empty() {
                                        ui.label(tr("工作区干净"));
                                    }
                                    ui.scope(|ui| {
                                        // Reserve a separate scrollbar lane so row actions never share its hit area.
                                        ui.style_mut().spacing.scroll.floating = false;
                                        ui.style_mut().spacing.scroll.bar_inner_margin = 10.0;
                                    egui::ScrollArea::vertical()
                                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                                        .id_salt("git-changed-files")
                                        .max_height(200.0)
                                        .show(ui, |ui| {
                                            for (group, files) in d.change_groups() {
                                                ui.add_space(4.0);
                                                let group_state=self.git_page.selection.state(files.iter().copied());
                                                let mut toggle=false;
                                                let id=ui.make_persistent_id(("git-file-group",r.project_id,group));
                                                let mut header=egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(),id,true).show_header(ui,|ui| {
                                                    let mut all=group_state.all;
                                                    if ui.add_enabled(!busy,egui::Checkbox::without_text(&mut all).indeterminate(group_state.partial))
                                                        .on_hover_text(tr("选中或取消本分组的全部文件")).changed() {
                                                        self.git_page.selection.set_all(files.iter().copied(),all);
                                                    }
                                                    toggle=ui.add(egui::Label::new(RichText::new(format!("{} ({})",tr(group),files.len())).small().color(tone(self.light,MUTED))).sense(egui::Sense::click()))
                                                        .on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
                                                });
                                                if toggle { header.toggle(); }
                                                header.body(|ui| {
                                                for file in files {
                                                    ui.horizontal(|ui| {
                                                        let mut checked = self.git_page.selection.selected.contains(&file.path);
                                                        if ui.add_enabled(!busy, egui::Checkbox::without_text(&mut checked)).on_hover_text(tr("勾选的文件将包含在本次提交中")).changed() {
                                                            self.git_page.selection.set(&file.path, checked);
                                                        }
                                                        ui.label(RichText::new(icon::FILE).color(tone(self.light, MUTED)));
                                                        let width = (ui.available_width() - 44.0).max(20.0);
                                                        let file_response = file_name_response(ui, &file.path, width)
                                                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                                                            .on_hover_ui(|ui| file_preview_tooltip(ui, &file.preview_path(r.root.as_deref().unwrap_or(&r.path))));
                                                        if !busy && file_response.double_clicked() { preview = Some(file.path.clone()); }
                                                        let enabled = !busy && file.status != "??";
                                                        let tooltip = if file.status == "??" {
                                                            tr("未跟踪文件没有 HEAD 版本，无法撤销恢复。")
                                                        } else {
                                                            tr("撤销此文件的工作区和暂存区修改，恢复到 HEAD 版本；操作前保存备份，点击后需再次确认。")
                                                        };
                                                        let tooltip = if busy { tr("Git 操作进行中，请稍候再回滚文件。") } else { tooltip };
                                                        if restore_file_button(ui, enabled, tooltip).clicked() {
                                                            action = Some(Action::Restore(file.path.clone()));
                                                        }
                                                    });
                                                }
                                                    });
                                            }
                                        });
                                    });
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        let button_width = [tr("生成"), tr("生成中…")].iter().map(|text| ui.painter().layout_no_wrap((*text).to_owned(), egui::TextStyle::Body.resolve(ui.style()), ui.visuals().text_color()).size().x).fold(0.0_f32, f32::max) + 48.0;
                                        let button_width = button_width.max(88.0);
                                        let width = (ui.available_width() - button_width - ui.spacing().item_spacing.x).max(80.0);
                                        ui.add(egui::TextEdit::multiline(&mut self.git_page.message).hint_text(tr("提交备注")).desired_rows(2).desired_width(width));
                                        ui.allocate_ui_with_layout(egui::vec2(button_width, 32.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                            ui.set_min_width(button_width);
                                            if self.git_page.generating_message {
                                                ui.spinner().on_hover_text(tr("正在使用本地 AI 工具生成提交备注…"));
                                                ui.label(tr("生成中…"));
                                            } else {
                                                let enabled = !busy && !self.git_page.selection.selected.is_empty();
                                                let label = format!("{} {}", icon::MAGIC_WAND, tr("生成"));
                                                let response = ui.add_enabled(enabled, egui::Button::new(button_label(ui, &label, enabled)).min_size(egui::vec2(button_width, 32.0)));
                                                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tr("生成提交备注")));
                                                generate_message = response.on_hover_text(tr("使用本地 AI 工具根据勾选文件生成提交备注，不会提交代码"))
                                                    .on_disabled_hover_text(tr("请先勾选需要提交的文件，并等待当前任务完成"))
                                                    .on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default }).clicked();
                                            }
                                        });
                                    });
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add_enabled(
                                                !busy
                                                    && !self.git_page.selection.selected.is_empty()
                                                    && !self.git_page.message.trim().is_empty(),
                                                egui::Button::new(button_label(
                                                    ui,
                                                    &format!(
                                                        "{} {}",
                                                        icon::GIT_COMMIT,
                                                        tr("提交本地")
                                                    ),
                                                    !busy
                                                        && !self.git_page.selection.selected.is_empty()
                                                        && !self.git_page.message.trim().is_empty(),
                                                )),
                                            )
                                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                                            .clicked()
                                        {
                                            action = Some(Action::Commit);
                                        }
                                        if ui
                                            .add_enabled(
                                                !busy
                                                    && self
                                                        .git_page
                                                        .remote_choices
                                                        .contains_key(&r.project_id),
                                                egui::Button::new(button_label(
                                                    ui,
                                                    &format!(
                                                        "{} {}",
                                                        icon::UPLOAD_SIMPLE,
                                                        tr("推送远程")
                                                    ),
                                                    !busy
                                                        && self
                                                            .git_page
                                                            .remote_choices
                                                            .contains_key(&r.project_id),
                                                )),
                                            )
                                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                                            .clicked()
                                            && let Some(key) =
                                                self.git_page.remote_choices.get(&r.project_id)
                                        {
                                            action =
                                                Some(Action::Push(key.clone(), r.branch.clone()));
                                        }
                                    });
                                } else if busy {
                                    if let Some(row) = self
                                        .git_page
                                        .rows
                                        .iter()
                                        .find(|r| Some(r.project_id) == self.git_page.selected)
                                    {
                                        ui.strong(&row.name);
                                        ui.label(
                                            RichText::new(service::path_label(&row.path))
                                                .small()
                                                .color(tone(self.light, MUTED)),
                                        );
                                        ui.label(
                                            RichText::new(&row.branch)
                                                .small()
                                                .color(tone(self.light, MUTED)),
                                        );
                                    }
                                    ui.spinner();
                                }
                                ui.separator();
                                self.project_terminal(ui);
                            });
                    });
            }
        }
        if let Some(commit) = inspect {
            self.git_inspect_commit(commit);
        } else if let Some((commit, file)) = historical_diff {
            self.git_historical_diff(commit, file);
        }
        if let Some(id) = selection {
            if let Some(id) = id {
                self.git_select(id, 0);
            } else {
                self.git_refresh();
            }
        }
        if let Some(offset) = page
            && let Some(id) = self.git_page.selected
        {
            self.git_select(id, offset);
        }
        if let Some(file) = preview
            && let Some(d) = &self.git_page.details
        {
            if self.git_page.diff_open && !self.git_page.diffs.is_empty() {
                self.git_page.preview_error =
                    Some(tr("请先关闭当前 Diff 窗口，再查看其他文件。").into());
                self.git_page.diff_minimized = false;
            } else {
                let path = d.repository.path.clone();
                self.git_page.request(move || {
                    Reply::WorkingDiff(actions::diff(&path, &file).map(|text| {
                        crate::git_diff::Document::new(
                            format!("{}:working:{file}", path.display()),
                            path.join(&file).display().to_string(),
                            text,
                        )
                    }))
                });
            }
        }
        if generate_message {
            self.git_generate_message();
        }
        if let Some(action) = action {
            if matches!(
                action,
                Action::Commit | Action::Push(_, _) | Action::Update(_, _) | Action::Switch(_)
            ) {
                self.git_action(action);
            } else {
                self.git_page.confirm = Some(action);
            }
        }
        self.git_dialogs(ui.ctx());
    }
    pub(super) fn git_notifications(&mut self, ctx: &egui::Context) {
        self.git_poll();
        if self.git_page.notices.is_empty() {
            return;
        }
        let mut closed = None;
        egui::Area::new(egui::Id::new("git-result-notices"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -18.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for (index, (success, message)) in self.git_page.notices.iter().enumerate() {
                    let mut frame = egui::Frame::popup(ui.style());
                    if !success {
                        frame = frame
                            .fill(if self.light {
                                Color32::from_rgb(255, 237, 238)
                            } else {
                                Color32::from_rgb(64, 24, 29)
                            })
                            .stroke(egui::Stroke::new(1.0_f32, tone(self.light, RED)));
                    }
                    frame.show(ui, |ui| {
                        ui.set_width(370.0);
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                tone(self.light, if *success { ACCENT } else { RED }),
                                if *success {
                                    tr("操作成功")
                                } else {
                                    tr("操作失败")
                                },
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if small(ui, icon::X, tr("关闭窗口")) {
                                        closed = Some(index);
                                    }
                                },
                            );
                        });
                        egui::ScrollArea::vertical()
                            .id_salt(("git-notice", index))
                            .max_height(170.0)
                            .show(ui, |ui| {
                                ui.label(message);
                            });
                    });
                }
            });
        if let Some(index) = closed {
            self.git_page.notices.remove(index);
        }
    }
    fn git_dialogs(&mut self, ctx: &egui::Context) {
        self.git_remote_dialog(ctx);
        self.git_merge_dialog(ctx);
        if let Some(error) = self.git_page.preview_error.clone() {
            let mut close = false;
            let response = egui::Modal::new(egui::Id::new("git-preview-message"))
                .frame(dialog_ui::frame(ctx))
                .show(ctx, |ui| {
                    dialog_ui::content(ui);
                    ui.set_width(420.0);
                    ui.heading(tr("文件预览"));
                    ui.label(tr(&error));
                    dialog_ui::actions(ui, |ui| {
                        close = ui.button(tr("关闭窗口")).clicked();
                    });
                });
            if close || response.should_close() {
                self.git_page.preview_error = None;
            }
        }
        if let Some(action) = self.git_page.confirm.clone() {
            let response=egui::Modal::new(egui::Id::new("git-confirm-action")).frame(dialog_ui::frame(ctx)).show(ctx,|ui|{
                dialog_ui::content(ui);ui.set_width(420.0);
                let title=match &action{Action::Switch(_)=>tr("切换分支"),Action::Merge(_,_)=>tr("合并分支"),Action::Restore(_)=>tr("回滚文件"),Action::RestoreVersion(_,_,_)=>tr("恢复到当前版本"),Action::Commit=>tr("提交本地"),Action::Push(_,_)=>tr("推送远程"),Action::Update(_,_)=>"Update"};
                ui.heading(title);
                if let Some(d)=&self.git_page.details{ui.label(RichText::new(format!("{} · {}",d.repository.name,d.repository.branch)).small().color(tone(self.light,MUTED)));}
                match &action{
                    Action::Merge(source,target)=>{ui.label(format!("{source} → {target}"));},
                    Action::Switch(branch)=>{ui.label(branch);ui.label(tr("切换会更新实际项目文件，关联项目也会受到影响。无冲突的本地修改会保留；冲突时将取消切换并提示原因。"));},
                    Action::RestoreVersion(commit,branch,_)=>{ui.label(tr("确认要把这个版本恢复为本地最新版本吗？"));ui.label(format!("{branch} · {commit}"));ui.label(tr("将生成新的本地恢复提交，保留历史，不修改远程。之后需自行推送以同步远程。"));},
                    Action::Restore(file)=>{ui.label(file);ui.label(tr("恢复到 HEAD，覆盖此文件的工作区和暂存区改动。DevStart 会先保存备份。"));},
                    Action::Commit=>{ui.label(&self.git_page.message);ui.label(tf("提交 {count} 个选中文件",&[("count",self.git_page.selection.selected.len().to_string())]));},
                    Action::Push(key,branch)|Action::Update(key,branch)=>{
                        if let Some(d)=&self.git_page.details && let Some(remote)=d.remotes.iter().find(|r|&r.key==key){ui.label(format!("{} · {}",remote.name,remote.url));}
                        ui.label(branch);
                        ui.label(if matches!(&action,Action::Update(_,_)){tr("从当前远程仓库当前分支获取最新版本与本地合并代码")}else{tr("将本地提交推送到所选远程仓库的当前分支，不强制覆盖。")});
                    },
                }
                dialog_ui::actions(ui,|ui|{
                    if ui.button(tr("取消")).clicked(){self.git_page.confirm=None;}
                    if ui.button(tr("确认")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked(){self.git_page.confirm=None;self.git_action(action);}
                });
            });
            if response.should_close() {
                self.git_page.confirm = None;
            }
        }
        self.git_diff_window(ctx);
    }
}

/// File previews share a two-line tooltip; paths stay intact rather than wrapping or truncating.
fn file_preview_tooltip(ui: &mut egui::Ui, path: &str) {
    ui.label(tr("双击查看"));
    ui.add(egui::Label::new(path).wrap_mode(egui::TextWrapMode::Extend));
}

/// Reserve the row's filename column without add_sized's centered-and-justified child layout.
fn file_name_response(ui: &mut egui::Ui, path: &str, width: f32) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 24.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.add(
                egui::Label::new(path)
                    .truncate()
                    .sense(egui::Sense::click()),
            )
        },
    )
    .inner
}
/// Disabled actions keep an enabled hover-only target, explaining why restore is unavailable.
fn restore_file_button(ui: &mut egui::Ui, enabled: bool, tooltip: &str) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(button_label(ui, icon::ARROW_U_UP_LEFT, enabled))
            .min_size(egui::vec2(32.0, 28.0))
            .frame(false),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, tr("回滚文件"))
    });
    if enabled {
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tooltip)
    } else {
        // A disabled button does not participate in normal hover hit testing.
        // Keep this separate target non-clickable and avoid a misleading hand cursor.
        ui.interact(
            response.rect,
            response.id.with("disabled-restore-reason"),
            egui::Sense::hover(),
        )
        .on_hover_cursor(egui::CursorIcon::Default)
        .on_hover_text(tooltip);
        response
    }
}
#[cfg(test)]
mod file_row_tests {
    use super::egui;
    #[test]
    fn filenames_start_after_icon_and_keep_action_column_fixed() {
        let context = egui::Context::default();
        let _ = context.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut action_positions = vec![];
                for name in ["a.txt", "directory/longer-filename.txt"] {
                    ui.horizontal(|ui| {
                        let icon = ui.label("file");
                        let label = super::file_name_response(ui, name, 400.0);
                        assert!(
                            (label.rect.left() - icon.rect.right() - ui.spacing().item_spacing.x)
                                .abs()
                                < 1.0
                        );
                        action_positions.push(ui.button("undo").rect.left());
                    });
                }
                assert!((action_positions[0] - action_positions[1]).abs() < 1.0);
            });
        });
    }
}
