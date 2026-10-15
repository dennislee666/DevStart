//! Reviewable batch intent. Planning is read-only; applying requires an unchanged confirmed plan.
use crate::{runtime::Run, service::Service};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
mod plan_id {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(id: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&id.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Stop,
    Start,
    Restart,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    #[serde(with = "plan_id")]
    pub run_id: u64,
    pub script_id: Option<u64>,
    pub title: String,
}
impl Target {
    pub fn from_run(run: &Run) -> Self {
        Self {
            run_id: run.id,
            script_id: (!(run.external.is_some() && run.associated_scripts.len() != 1))
                .then_some(run.script_id),
            title: run.title.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub revision: String,
    pub action: Action,
    pub targets: Vec<Target>,
    pub blocked: Vec<String>,
}
impl Plan {
    pub fn enabled(&self) -> bool {
        !self.targets.is_empty() && self.blocked.is_empty()
    }
}
#[derive(Serialize)]
pub struct ResultSet {
    pub action: Action,
    pub run_ids: Vec<u64>,
    pub errors: Vec<String>,
}
impl Service {
    pub fn bulk_plan(&self, action: Action) -> Plan {
        use crate::runtime::State;
        let mut candidates: Vec<_> = self
            .runtime
            .runs
            .iter()
            .filter(|r| r.tab_open && !r.superseded)
            .filter(|r| match action {
                Action::Stop => r.state.active(),
                Action::Start => !r.state.active(),
                Action::Restart => r.state == State::Running,
            })
            .collect();
        candidates.sort_by_key(|r| std::cmp::Reverse(r.id));
        let target_consoles: HashSet<_> = candidates.iter().map(|r| r.console_id).collect();
        let mut seen = HashSet::new();
        let mut ports = HashSet::new();
        let mut resources = HashSet::new();
        let mut targets = Vec::new();
        let mut blocked = Vec::new();
        for run in &candidates {
            let target = Target::from_run(run);
            if action == Action::Stop {
                targets.push(target);
                continue;
            }
            let Some(sid) = target.script_id else {
                blocked.push(format!(
                    "{}：外部运行参数未识别，请单独选择启动项",
                    target.title
                ));
                targets.push(target);
                continue;
            };
            if !seen.insert(sid) {
                continue;
            }
            let Some(script) = self.script(sid) else {
                blocked.push(format!("{}：启动项已移除", target.title));
                targets.push(target);
                continue;
            };
            if let Some(reason) = script.launch_block_reason() {
                blocked.push(format!("{}：{reason}", target.title));
            }
            let port = match self.configured_port_hint(sid) {
                Ok(h) => h.map(|v| v.port),
                Err(e) => {
                    blocked.push(format!("{}：{e}", target.title));
                    None
                }
            };
            if let Some(port) = port
                && !ports.insert(port)
            {
                blocked.push(format!("端口 {port} 被多个目标共享，请单独选择"));
            }
            if let Some(key) = &script.resource_key
                && !resources.insert(key.clone())
            {
                blocked.push("多个目标共享调试资源，请单独选择".into());
            }
            // Starting stopped tabs must never stop an already-running different tab.
            // Restart may replace only its approved logical consoles and their retiring generations.
            for other in self.runtime.runs.iter().filter(|r| r.state.active()) {
                if action == Action::Restart && target_consoles.contains(&other.console_id) {
                    continue;
                }
                let occupied = self
                    .configured_port_hint(other.script_id)
                    .ok()
                    .flatten()
                    .map(|h| h.port);
                if other.associated_scripts.contains(&sid)
                    || script
                        .resource_key
                        .as_ref()
                        .is_some_and(|key| other.resource_key.as_ref() == Some(key))
                    || port.is_some_and(|p| {
                        other.listening_ports.contains(&p)
                            || other.expected_port == Some(p)
                            || occupied == Some(p)
                    })
                {
                    blocked.push(format!(
                        "{} 与{}的 {} 冲突，请单独选择",
                        target.title,
                        other.state.label(),
                        other.title
                    ));
                }
            }
            targets.push(target);
        }
        if action == Action::Stop
            && !candidates.is_empty()
            && candidates.iter().all(|r| r.state == State::Stopping)
        {
            blocked.push("正在停止，请稍候".into());
        }
        targets.sort_by_key(|r| r.run_id);
        blocked.sort();
        blocked.dedup();
        self.seal_bulk(Plan {
            revision: String::new(),
            action,
            targets,
            blocked,
        })
    }
    fn seal_bulk(&self, mut plan: Plan) -> Plan {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&plan.action).expect("serializable batch action"));
        hash.update(serde_json::to_vec(&plan.targets).expect("serializable batch targets"));
        for target in &plan.targets {
            if let Some(script) = target.script_id.and_then(|id| self.script(id)) {
                hash.update(serde_json::to_vec(script).expect("serializable launch configuration"));
                // Detect source replacement while the confirmation dialog is open.
                let stamp = std::fs::metadata(&script.file).map(|m| (m.len(), m.modified()));
                hash.update(format!("{stamp:?}"));
            }
        }
        plan.revision = format!("{:x}", hash.finalize());
        plan
    }
    pub fn apply_bulk(&mut self, plan: &Plan, confirmed: bool) -> Result<ResultSet, String> {
        if !confirmed {
            return Err("批量操作需要二次确认".into());
        }
        if self.bulk_plan(plan.action) != *plan {
            return Err("会话或配置已变化，请重新确认批量操作".into());
        }
        if !plan.enabled() {
            return Err(if plan.blocked.is_empty() {
                "暂无可操作的启动会话".into()
            } else {
                plan.blocked.join("；")
            });
        }
        if plan.action == Action::Stop {
            for target in &plan.targets {
                self.runtime.stop_console(target.run_id)?;
            }
            return Ok(ResultSet {
                action: Action::Stop,
                run_ids: plan.targets.iter().map(|r| r.run_id).collect(),
                errors: vec![],
            });
        }
        let mut result = ResultSet {
            action: plan.action,
            run_ids: vec![],
            errors: vec![],
        };
        for target in &plan.targets {
            let sid = target.script_id.ok_or("启动项未识别")?;
            let start = self.restart_run(target.run_id, sid);
            match start {
                Ok(id) => result.run_ids.push(id),
                Err(e) => result.errors.push(format!("{}：{e}", target.title)),
            }
        }
        Ok(result)
    }
}
