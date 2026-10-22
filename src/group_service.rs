//! Group operations change only the app-owned catalog. Discovery never executes project code.
use crate::{
    model::{Catalog, Project},
    service::Service,
};

pub struct RefreshPlan {
    before: String,
    group: u64,
    projects: Vec<Project>,
}
/// Build the entire replacement before publication: any unreadable project aborts the operation.
pub fn prepare(catalog: &Catalog, group: u64) -> Result<RefreshPlan, String> {
    if !catalog.groups.iter().any(|g| g.id == group) {
        return Err("分组不存在".into());
    }
    let mut projects = Vec::new();
    for old in catalog.projects.iter().filter(|p| p.group == group) {
        let (name, mut scripts) = crate::project_scan::rediscover(&old.path)
            .map_err(|e| format!("{}: {e}", old.path.display()))?;
        let mut identities = std::collections::HashSet::new();
        scripts.retain(|s| identities.insert(s.identity()));
        for script in &mut scripts {
            if let Some(previous) = old
                .scripts
                .iter()
                .find(|s| s.source_identity() == script.source_identity())
            {
                script.id = previous.id;
                script.favorite = previous.favorite;
                script.display_name = previous.display_name.clone();
                script.description_override = previous.description_override.clone();
            } else {
                script.id = crate::model::unique_id();
            }
            script.custom = true;
            script.expected_port = crate::configured_ports::resolve(script)?
                .map(|h| h.port)
                .or(script.expected_port);
        }
        let mut project = old.clone();
        project.name = name;
        project.scripts = scripts;
        project.explicit_scripts = true;
        projects.push(project);
    }
    Ok(RefreshPlan {
        before: serde_json::to_string(catalog).map_err(|e| e.to_string())?,
        group,
        projects,
    })
}
/// Registration scope shared by project and group removal; never a filesystem deletion target.
#[derive(Clone, Copy)]
pub enum DeletionTarget {
    Group(u64),
    Project(u64),
}
impl DeletionTarget {
    fn contains(self, project: &Project) -> bool {
        match self {
            Self::Group(id) => project.group == id,
            Self::Project(id) => project.id == id,
        }
    }
    pub fn phrase(self) -> &'static str {
        match self {
            Self::Group(_) => "删除分组",
            Self::Project(_) => "删除项目",
        }
    }
    pub fn success(self) -> &'static str {
        match self {
            Self::Group(_) => "分组及项目登记已删除",
            Self::Project(_) => "项目及启动项登记已删除",
        }
    }
}
pub struct Deletion {
    target: DeletionTarget,
    before: String,
    runs: Vec<(u64, String)>,
    deadline: std::time::Instant,
}
impl Service {
    /// Initiate scoped shutdown without blocking the UI. Runtime owns TERM/KILL escalation.
    pub fn begin_group_deletion(&mut self, group: u64) -> Result<Deletion, String> {
        self.begin_deletion(DeletionTarget::Group(group))
    }
    pub fn begin_deletion(&mut self, target: DeletionTarget) -> Result<Deletion, String> {
        match target {
            DeletionTarget::Group(1) => return Err("默认分组不可删除".into()),
            DeletionTarget::Group(id) if !self.catalog.groups.iter().any(|g| g.id == id) => {
                return Err("分组不存在".into());
            }
            DeletionTarget::Project(id) if !self.catalog.projects.iter().any(|p| p.id == id) => {
                return Err("项目不存在".into());
            }
            _ => {}
        }
        let ids: Vec<_> = self
            .catalog
            .projects
            .iter()
            .filter(|p| target.contains(p))
            .flat_map(|p| p.scripts.iter().map(|s| s.id))
            .collect();
        let runs: Vec<_> = self
            .runtime
            .runs
            .iter()
            .filter(|r| {
                r.state.active()
                    && (ids.contains(&r.script_id)
                        || r.associated_scripts.iter().any(|id| ids.contains(id)))
            })
            .map(|r| (r.id, format!("{} (PID {})", r.title, r.pid)))
            .collect();
        for (id, title) in &runs {
            eprintln!(
                "[dev start registrations] {} stopping run={id} {title}",
                target.phrase()
            );
            self.runtime.stop(*id);
        }
        Ok(Deletion {
            target,
            before: serde_json::to_string(&self.catalog).map_err(|e| e.to_string())?,
            runs,
            deadline: std::time::Instant::now()
                + crate::lifecycle::GRACE
                + crate::lifecycle::FORCE_WAIT
                + std::time::Duration::from_secs(5),
        })
    }
    /// Publish removal only after every scoped lifecycle worker verifies that its process exited.
    pub fn finish_deletion(&mut self, deletion: &Deletion) -> Result<Option<usize>, String> {
        self.runtime.poll();
        for (id, title) in &deletion.runs {
            match self.runtime.termination_result(*id) {
                Some(Ok(())) => {}
                Some(Err(error)) => return Err(format!("{title}: {error}")),
                None if std::time::Instant::now() >= deletion.deadline => {
                    return Err(format!("关闭实例超时，登记已保留: {title}"));
                }
                None => return Ok(None),
            }
        }
        if serde_json::to_string(&self.catalog).map_err(|e| e.to_string())? != deletion.before {
            return Err("关闭实例期间配置已变化，请重新执行删除".into());
        }
        let target = deletion.target;
        let ids: Vec<_> = self
            .catalog
            .projects
            .iter()
            .filter(|p| target.contains(p))
            .flat_map(|p| p.scripts.iter().map(|s| s.id))
            .collect();
        if let Some(run) = self.runtime.runs.iter().find(|r| {
            r.state.active()
                && (ids.contains(&r.script_id)
                    || r.associated_scripts.iter().any(|id| ids.contains(id)))
        }) {
            return Err(format!(
                "删除期间发现新的关联实例，登记已保留，请重新删除: {} (PID {})",
                run.title, run.pid
            ));
        }
        let mut next = self.catalog.clone();
        let count = next.projects.iter().filter(|p| target.contains(p)).count();
        next.projects.retain(|p| !target.contains(p));
        if let DeletionTarget::Group(group) = target {
            next.groups.retain(|g| g.id != group);
        }
        self.commit(next)?;
        Ok(Some(count))
    }
    pub fn delete_group(&mut self, group: u64) -> Result<usize, String> {
        let deletion = self.begin_group_deletion(group)?;
        self.wait_deletion(deletion)
    }
    pub fn delete_project(&mut self, project: u64) -> Result<usize, String> {
        let deletion = self.begin_deletion(DeletionTarget::Project(project))?;
        self.wait_deletion(deletion)
    }
    fn wait_deletion(&mut self, deletion: Deletion) -> Result<usize, String> {
        loop {
            if let Some(count) = self.finish_deletion(&deletion)? {
                return Ok(count);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
    pub fn apply_group_refresh(&mut self, plan: RefreshPlan) -> Result<usize, String> {
        // Reject stale plans rather than overwrite concurrent API or UI edits.
        if serde_json::to_string(&self.catalog).map_err(|e| e.to_string())? != plan.before {
            return Err("扫描期间配置已变化，请重新扫描".into());
        }
        let ids: Vec<_> = self
            .catalog
            .projects
            .iter()
            .filter(|p| p.group == plan.group)
            .flat_map(|p| p.scripts.iter().map(|s| s.id))
            .collect();
        crate::api_config::idle(self, &ids)?;
        let count = plan.projects.len();
        let mut next = self.catalog.clone();
        for project in plan.projects {
            let id = project.id;
            *next
                .projects
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or("项目不存在")? = project;
        }
        self.commit(next)?;
        Ok(count)
    }
}
