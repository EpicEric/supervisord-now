use std::path::PathBuf;
use std::time::Duration;

use tokio::time::sleep;

use crate::jobs::{self, PROGRAM_PREFIX};
use crate::state::AppState;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

fn resume_file(state: &AppState) -> PathBuf {
    state.run_dir.join("resume.json")
}

fn read_resume(state: &AppState) -> Vec<String> {
    std::fs::read(resume_file(state))
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

fn write_resume(state: &AppState, jobs: &[String]) {
    let target = resume_file(state);
    let tmp = target.with_extension("json.tmp");
    let Ok(mut file) = std::fs::File::create(&tmp) else {
        tracing::warn!("cannot create resume file {}", tmp.display());
        return;
    };
    if let Err(err) = serde_json::to_writer(&mut file, jobs) {
        tracing::warn!("cannot write resume file {}: {err}", tmp.display());
        return;
    }
    if let Err(err) = file.sync_all() {
        tracing::warn!("cannot flush resume file {}: {err}", tmp.display());
        return;
    }
    if let Err(err) = std::fs::rename(&tmp, &target) {
        tracing::warn!("cannot replace resume file {}: {err}", target.display());
    }
}

async fn observe(state: &AppState) -> Option<Vec<String>> {
    let infos = state.supervisor.get_all_process_info().await.ok()?;
    let mut running: Vec<String> = infos
        .iter()
        .filter(|info| info.statename == "Running")
        .filter_map(|info| info.group.strip_prefix(PROGRAM_PREFIX))
        .map(str::to_string)
        .collect();
    running.sort();
    running.dedup();
    Some(running)
}

async fn resume_running(state: &AppState) {
    let jobs = read_resume(state);
    if jobs.is_empty() {
        return;
    }
    tracing::info!("resuming {} previously running job(s)", jobs.len());
    for job in &jobs {
        let Ok(group) = jobs::program_name(job) else {
            tracing::warn!("skipping invalid job in resume file: {job}");
            continue;
        };
        if let Err(err) = state.supervisor.start_process_group(&group, false).await {
            tracing::warn!("could not resume job {job}: {}", err.message);
        }
    }
}

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        resume_running(&state).await;
        let mut running = read_resume(&state);
        loop {
            sleep(POLL_INTERVAL).await;
            let Some(observed) = observe(&state).await else {
                continue;
            };
            if observed != running {
                write_resume(&state, &observed);
                running = observed;
            }
        }
    });
}
