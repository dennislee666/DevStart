//! Background launch preflight. No signals are sent until every candidate is verified.
use crate::{
    lifecycle::{Control, Journal},
    model::{Project, Script},
};
use std::{
    collections::HashSet,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub struct Tracked {
    pub pid: u32,
    pub control: Control,
}
pub fn prepare(
    project: &Project,
    script: &Script,
    tracked: &[Tracked],
    cancel: &Control,
    journal: &Journal,
) -> Result<(), String> {
    let check_cancel = || {
        if cancel.stop.load(Ordering::Acquire) {
            Err("已取消启动".to_string())
        } else {
            Ok(())
        }
    };
    check_cancel()?;
    let compile = crate::java_edit::prepare(script, journal.workspace_root(), cancel, journal)?;
    check_cancel()?;
    journal.note("检查项目原实例与端口占用…");
    let excluded: HashSet<_> = tracked.iter().map(|p| p.pid).filter(|p| *p > 0).collect();
    if script.expected_port.is_none() {
        journal.note("未配置端口，按启动命令、参数和工作目录核实原实例");
    }
    let external = crate::external::find_existing(project, script, &excluded)?;
    check_cancel()?;
    journal.note(format!(
        "检查完成：本工具实例 {} 个，已核实外部实例 {} 个",
        tracked.len(),
        external.len()
    ));
    if tracked.is_empty() && external.is_empty() {
        journal.note("未发现可核实的原实例，无需关闭");
    }
    // All ownership checks precede termination, so an unrelated listener leaves old runs intact.
    for process in tracked {
        process.control.observe(journal.clone());
        journal.note(format!("请求关闭已登记实例 PID {}", process.pid));
        process.control.request();
    }
    for process in &external {
        check_cancel()?;
        crate::lifecycle::terminate_external(process, &|s| journal.note(s))?;
    }
    let deadline = Instant::now()
        + crate::lifecycle::GRACE
        + crate::lifecycle::FORCE_WAIT
        + Duration::from_secs(2);
    for process in tracked {
        loop {
            check_cancel()?;
            if let Some(result) = process.control.completion() {
                result?;
                journal.note(format!("确认原实例 PID {} 已退出", process.pid));
                break;
            }
            if Instant::now() >= deadline {
                return Err(format!("等待 PID {} 退出超时，取消启动", process.pid));
            }
            std::thread::sleep(Duration::from_millis(60));
        }
    }
    check_cancel()?;
    journal.note("复核是否仍有相同项目实例或守护程序重新拉起的服务…");
    let remaining = crate::external::find_existing(project, script, &excluded)?;
    if !remaining.is_empty() {
        return Err(
            "关闭后仍检测到匹配实例，可能由其它守护程序重新拉起；取消启动，请先停止对应守护程序"
                .into(),
        );
    }
    check_cancel()?;
    if let Some(compiled) = compile {
        compiled.install(journal)?;
    }
    journal.note("原实例关闭完成，准备再次校验并启动新实例");
    Ok(())
}
