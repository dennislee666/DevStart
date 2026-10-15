//! Read-only, asynchronous TCP listening-port snapshots for launch process trees.
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    process::Command,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct Target {
    pub id: u64,
    pub pid: u32,
    pub external: Option<crate::external::Process>,
}
struct Snapshot {
    id: u64,
    pid: u32,
    ports: Vec<u16>,
}
#[derive(Default)]
pub struct Monitor {
    pending: Option<Receiver<Result<Vec<Snapshot>, String>>>,
    last: Option<Instant>,
}
impl Monitor {
    pub fn poll(&mut self, runs: &mut [crate::runtime::Run]) {
        for run in runs.iter_mut().filter(|r| !r.state.active() || r.pid == 0) {
            run.listening_ports.clear();
        }
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.pending = None;
                    match result {
                        Ok(values) => {
                            for snapshot in values {
                                if let Some(run) = runs.iter_mut().find(|r| {
                                    r.id == snapshot.id && r.pid == snapshot.pid && r.state.active()
                                }) {
                                    run.listening_ports = snapshot.ports;
                                }
                            }
                        }
                        Err(e) => {
                            for run in runs.iter_mut() {
                                run.listening_ports.clear();
                            }
                            eprintln!("[dev start] 运行端口检测失败: {e}");
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.pending = None;
                    for run in runs.iter_mut() {
                        run.listening_ports.clear();
                    }
                    eprintln!("[dev start] 运行端口检测线程异常退出");
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.pending.is_none()
            && self
                .last
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
        {
            let targets: Vec<_> = runs
                .iter()
                .filter(|r| r.state.active() && r.pid > 0)
                .map(|r| Target {
                    id: r.id,
                    pid: r.pid,
                    external: r.external.clone(),
                })
                .collect();
            if targets.is_empty() {
                return;
            }
            let (tx, rx) = mpsc::channel();
            self.pending = Some(rx);
            self.last = Some(Instant::now());
            thread::spawn(move || {
                let _ = tx.send(scan(&targets));
            });
        }
    }
}
fn scan(targets: &[Target]) -> Result<Vec<Snapshot>, String> {
    let out = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid="])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "进程关系快照失败: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let processes = parse_processes(&String::from_utf8_lossy(&out.stdout));
    let listeners = crate::external::listeners()?;
    let mut result = vec![];
    for target in targets {
        let valid = if let Some(identity) = &target.external {
            crate::external::same_process(identity)?
        } else {
            true
        };
        let ports = if valid {
            ports_for(
                target.pid,
                target.external.is_none(),
                &processes,
                &listeners,
            )
        } else {
            vec![]
        };
        result.push(Snapshot {
            id: target.id,
            pid: target.pid,
            ports,
        });
    }
    Ok(result)
}
fn parse_processes(text: &str) -> Vec<(u32, u32, u32)> {
    text.lines()
        .filter_map(|line| {
            let mut p = line.split_whitespace();
            Some((
                p.next()?.parse().ok()?,
                p.next()?.parse().ok()?,
                p.next()?.parse().ok()?,
            ))
        })
        .collect()
}
/// Owned launches have a dedicated group; external launches can share a terminal group.
fn ports_for(
    root: u32,
    owned: bool,
    processes: &[(u32, u32, u32)],
    listeners: &BTreeMap<u16, Vec<u32>>,
) -> Vec<u16> {
    let mut ids: HashSet<_> = processes
        .iter()
        .filter(|(pid, _, group)| if owned { *group == root } else { *pid == root })
        .map(|(pid, _, _)| *pid)
        .collect();
    loop {
        let children: Vec<_> = processes
            .iter()
            .filter(|(pid, parent, _)| !ids.contains(pid) && ids.contains(parent))
            .map(|(pid, _, _)| *pid)
            .collect();
        if children.is_empty() {
            break;
        }
        ids.extend(children);
    }
    listeners
        .iter()
        .filter(|(_, pids)| pids.iter().any(|pid| ids.contains(pid)))
        .map(|(port, _)| *port)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correlates_owned_groups_and_descendants_without_external_siblings() {
        let processes =
            parse_processes("10 1 10\n11 10 10\n12 11 12\n13 1 10\n20 1 20\nmalformed\n");
        let listeners = BTreeMap::from([
            (8080, vec![11, 11]),
            (9090, vec![12]),
            (5000, vec![13]),
            (3000, vec![20]),
        ]);
        assert_eq!(
            ports_for(10, true, &processes, &listeners),
            vec![5000, 8080, 9090]
        );
        assert_eq!(
            ports_for(10, false, &processes, &listeners),
            vec![8080, 9090]
        );
        assert!(ports_for(99, false, &processes, &listeners).is_empty());
    }
}
