//! Bounded background API jobs. Mutating Git requests never block the GUI service loop.
use serde::Serialize;
use std::sync::{Mutex, OnceLock};
#[derive(Clone, Serialize)]
pub struct Job {
    pub id: u64,
    pub state: String,
    pub output: Option<String>,
    pub error: Option<String>,
}
fn registry() -> &'static Mutex<Vec<Job>> {
    static JOBS: OnceLock<Mutex<Vec<Job>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(vec![]))
}
pub fn start(
    action: impl FnOnce() -> Result<String, String> + Send + 'static,
) -> Result<u64, String> {
    let id = crate::model::unique_id();
    {
        let mut jobs = registry().lock().map_err(|e| e.to_string())?;
        if jobs.iter().filter(|j| j.state == "running").count() >= 4 {
            return Err("Four Git jobs are already running".into());
        }
        while jobs.len() >= 16 {
            let i = jobs
                .iter()
                .position(|j| j.state != "running")
                .ok_or("Git job history is full")?;
            jobs.remove(i);
        }
        jobs.push(Job {
            id,
            state: "running".into(),
            output: None,
            error: None,
        });
    }
    std::thread::spawn(move || {
        let result = action();
        if let Ok(mut jobs) = registry().lock()
            && let Some(job) = jobs.iter_mut().find(|j| j.id == id)
        {
            match result {
                Ok(out) => {
                    job.state = "succeeded".into();
                    job.output = Some(out);
                }
                Err(e) => {
                    eprintln!("[dev start git job {id}] {e}");
                    job.state = "failed".into();
                    job.error = Some(e);
                }
            }
        }
    });
    Ok(id)
}
pub fn get(id: u64) -> Result<Job, String> {
    registry()
        .lock()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|j| j.id == id)
        .cloned()
        .ok_or("Git job not found (last 16 retained)".into())
}
