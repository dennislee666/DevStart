//! Three Git panes: project selection, working tree/actions, and paged history.
//! All repository inspection and mutations run on a worker through shared backend functions.
use super::*;
#[path = "ui_git_commit.rs"]
mod commit_ui;
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
    checked: HashSet<String>,
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
}
type Details = git::Snapshot;
enum Reply {
    List(Vec<Repository>),
    Selected(u64, Result<Box<Details>, String>, Option<String>),
    Diff(Result<crate::git_diff::Document, String>),
    Commit(Result<Box<git::commits::Inspection>, String>),
}
#[derive(Clone)]
enum Action {
    Switch(String),
    Restore(String),
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
            self.git_page.checked.clear();
            self.git_page.message.clear();
            self.git_page.output.clear();

            self.git_page.details = self
                .git_page
                .detail_cache
                .get(&id)
                .filter(|d| d.repository.path == project.path && d.offset == offset)
                .cloned();
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
        let files: Vec<_> = self.git_page.checked.iter().cloned().collect();
        self.git_page.request(move || {
            let (label, result) = match action {
                Action::Switch(branch) => (
                    format!("git switch {branch}"),
                    git::switch(&project.path, &branch).map(|_| format!("Switched to {branch}")),
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
            Reply::Selected(id, details(&project, 0), Some(output))
        });
    }
    fn git_poll(&mut self) {
        let reply = match self.git_page.pending.as_ref().map(|r| r.try_recv()) {
            Some(Ok(reply)) => Some(reply),
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.git_page.pending = None;
                self.git_page.error = Some(tr("Git 后台任务中断").into());
                None
            }
            _ => None,
        };
        if let Some(reply) = reply {
            self.git_page.pending = None;
            match reply {
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
                            self.git_page
                                .checked
                                .retain(|f| d.files.iter().any(|c| &c.path == f));
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
        self.git_poll();
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
                                    ui.strong(tr("修改文件"));
                                    if d.files.is_empty() {
                                        ui.label(tr("工作区干净"));
                                    }
                                    egui::ScrollArea::vertical()
                                        .id_salt("git-changed-files")
                                        .max_height(200.0)
                                        .show(ui, |ui| {
                                            for file in &d.files {
                                                ui.horizontal(|ui| {
                                                    let mut checked =
                                                        self.git_page.checked.contains(&file.path);
                                                    if ui
                                                        .add_enabled(
                                                            !busy,
                                                            egui::Checkbox::without_text(
                                                                &mut checked,
                                                            ),
                                                        )
                                                        .changed()
                                                    {
                                                        if checked {
                                                            self.git_page
                                                                .checked
                                                                .insert(file.path.clone());
                                                        } else {
                                                            self.git_page
                                                                .checked
                                                                .remove(&file.path);
                                                        }
                                                    }
                                                    ui.monospace(&file.status);
                                                    ui.add(egui::Label::new(&file.path).truncate())
                                                        .on_hover_text(&file.path);
                                                });
                                                ui.horizontal(|ui| {
                                                    if ui
                                                        .add_enabled(
                                                            !busy,
                                                            egui::Button::new("Diff"),
                                                        )
                                                        .on_hover_cursor(
                                                            egui::CursorIcon::PointingHand,
                                                        )
                                                        .clicked()
                                                    {
                                                        preview = Some(file.path.clone());
                                                    }
                                                    if ui
                                                        .add_enabled(
                                                            !busy && file.status != "??",
                                                            egui::Button::new(tr("回滚文件")),
                                                        )
                                                        .on_hover_cursor(
                                                            egui::CursorIcon::PointingHand,
                                                        )
                                                        .clicked()
                                                    {
                                                        action = Some(Action::Restore(
                                                            file.path.clone(),
                                                        ));
                                                    }
                                                });
                                            }
                                        });
                                    ui.add_space(8.0);
                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.git_page.message)
                                            .hint_text(tr("提交备注"))
                                            .desired_rows(2)
                                            .desired_width(f32::INFINITY),
                                    );
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add_enabled(
                                                !busy
                                                    && !self.git_page.checked.is_empty()
                                                    && !self.git_page.message.trim().is_empty(),
                                                egui::Button::new(button_label(
                                                    ui,
                                                    &format!(
                                                        "{} {}",
                                                        icon::GIT_COMMIT,
                                                        tr("提交本地")
                                                    ),
                                                    !busy
                                                        && !self.git_page.checked.is_empty()
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
            let path = d.repository.path.clone();
            self.git_page.request(move || {
                Reply::Diff(actions::diff(&path, &file).map(|text| {
                    crate::git_diff::Document::new(
                        format!("{}:working:{file}", path.display()),
                        path.join(&file).display().to_string(),
                        text,
                    )
                }))
            });
        }
        if let Some(action) = action {
            self.git_page.confirm = Some(action);
        }
        self.git_dialogs(ui.ctx());
    }
    fn git_dialogs(&mut self, ctx: &egui::Context) {
        self.git_remote_dialog(ctx);
        if let Some(action) = self.git_page.confirm.clone() {
            let response=egui::Modal::new(egui::Id::new("git-confirm-action")).frame(dialog_ui::frame(ctx)).show(ctx,|ui|{
                dialog_ui::content(ui);ui.set_width(420.0);
                let title=match &action{Action::Switch(_)=>tr("切换分支"),Action::Restore(_)=>tr("回滚文件"),Action::Commit=>tr("提交本地"),Action::Push(_,_)=>tr("推送远程"),Action::Update(_,_)=>"Update"};
                ui.heading(title);
                if let Some(d)=&self.git_page.details{ui.label(RichText::new(format!("{} · {}",d.repository.name,d.repository.branch)).small().color(tone(self.light,MUTED)));}
                match &action{
                    Action::Switch(branch)=>{ui.label(branch);ui.label(tr("切换会更新整个仓库的文件，关联项目也会受到影响。请先停止依赖这些文件的服务；有未提交改动时将拒绝切换。"));},
                    Action::Restore(file)=>{ui.label(file);ui.label(tr("恢复到 HEAD，覆盖此文件的工作区和暂存区改动。DevStart 会先保存备份。"));},
                    Action::Commit=>{ui.label(&self.git_page.message);ui.label(tf("提交 {count} 个选中文件",&[("count",self.git_page.checked.len().to_string())]));},
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
