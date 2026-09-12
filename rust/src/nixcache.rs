use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::state::AppState;

pub const CACHE_DIR: &str = "nix-cache";
pub const GCROOTS_DIR: &str = "gcroots";
const COPY_TIMEOUT: Duration = Duration::from_secs(600);
const EVAL_TIMEOUT: Duration = Duration::from_secs(120);

static COPY_LOCK: Mutex<()> = Mutex::const_new(());

fn cache_url(state: &AppState) -> String {
    format!("file://{}", state.run_dir.join(CACHE_DIR).display())
}

fn gcroot_dir(state: &AppState) -> PathBuf {
    state.run_dir.join(GCROOTS_DIR)
}

// Roots survive on the volume as dangling symlinks after the store is wiped,
// so targets are read without resolving them.
async fn root_targets(state: &AppState) -> Vec<String> {
    let mut targets = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(gcroot_dir(state)).await else {
        return targets;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Ok(target) = tokio::fs::read_link(entry.path()).await {
            targets.push(target.display().to_string());
        }
    }
    targets
}

#[derive(Deserialize)]
struct EvalOutput {
    #[serde(default)]
    jobs: serde_json::Map<String, serde_json::Value>,
}

async fn eval_drv_paths(state: &AppState) -> Option<Vec<String>> {
    let flake = state.workspace.join("flake.nix").exists();
    let mut command = Command::new("now");
    command.arg("eval");
    if flake {
        command.arg("--flake").arg(&state.workspace);
    } else {
        command
            .arg("--workflow")
            .arg(state.workspace.join("now.nix"));
    }
    command.current_dir(&state.workspace);

    let result = tokio::time::timeout(EVAL_TIMEOUT, command.output()).await;
    let output = match result {
        Ok(Ok(output)) if output.status.success() => output,
        Ok(Ok(output)) => {
            tracing::debug!(
                "skipping root prune, workflow eval failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return None;
        }
        Ok(Err(err)) => {
            tracing::debug!("skipping root prune, cannot run now eval: {err}");
            return None;
        }
        Err(_) => {
            tracing::debug!("skipping root prune, workflow eval timed out");
            return None;
        }
    };

    let parsed: EvalOutput = serde_json::from_slice(&output.stdout).ok()?;
    let mut drv_paths = Vec::new();
    for job in parsed.jobs.values() {
        let job_list: &[serde_json::Value] = match job {
            serde_json::Value::Array(array) => array,
            job @ serde_json::Value::Object(_) => std::slice::from_ref(job),
            _ => continue,
        };
        for job in job_list {
            let Some(steps) = job.get("steps").and_then(|steps| steps.as_array()) else {
                continue;
            };
            for step in steps {
                for key in ["runDrv", "teardownDrv"] {
                    if let Some(path) = step.get(key).and_then(|path| path.as_str()) {
                        drv_paths.push(path.to_string());
                    }
                }
            }
        }
    }
    Some(drv_paths)
}

async fn live_closure(_state: &AppState, drv_paths: &[String]) -> Option<HashSet<String>> {
    let result = Command::new("nix-store")
        .args(["--query", "--requisites", "--include-outputs"])
        .args(drv_paths)
        .output()
        .await;
    match result {
        Ok(output) if output.status.success() => Some(
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_string)
                .collect(),
        ),
        Ok(output) => {
            tracing::debug!(
                "skipping root prune, closure query failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            None
        }
        Err(err) => {
            tracing::debug!("skipping root prune, cannot run nix-store: {err}");
            None
        }
    }
}

/// Removes roots that no longer match the current workflow. The live set is
/// the requisites closure of the evaluated step derivations, which also covers
/// the roots that runner.steps.build/upload create for their user derivations.
/// Anything that cannot be determined (eval failure, missing store paths)
/// skips the prune instead of deleting unverified roots.
async fn prune(state: &AppState) -> usize {
    let Some(drv_paths) = eval_drv_paths(state).await else {
        return 0;
    };
    if drv_paths.is_empty() {
        return 0;
    }
    let Some(live) = live_closure(state, &drv_paths).await else {
        return 0;
    };

    let mut removed = 0;
    let Ok(mut entries) = tokio::fs::read_dir(gcroot_dir(state)).await else {
        return 0;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(target) = tokio::fs::read_link(entry.path()).await else {
            continue;
        };
        if !live.contains(&target.display().to_string()) {
            if tokio::fs::remove_file(entry.path()).await.is_ok() {
                removed += 1;
            }
        }
    }
    if removed > 0 {
        tracing::info!("pruned {removed} stale Nix GC root(s)");
    }
    removed
}

async fn clear_cache(state: &AppState) {
    let Ok(mut entries) = tokio::fs::read_dir(state.run_dir.join(CACHE_DIR)).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            let _ = tokio::fs::remove_dir_all(path).await;
        } else {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

async fn copy(state: &AppState, direction: &str, args: &[&str]) -> bool {
    let result = tokio::time::timeout(COPY_TIMEOUT, async {
        Command::new("nix")
            .arg("copy")
            .args([direction, &cache_url(state)])
            .args(args)
            .output()
            .await
    })
    .await;
    match result {
        Ok(Ok(output)) if output.status.success() => true,
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("nix copy {direction} cache failed: {}", stderr.trim());
            false
        }
        Ok(Err(err)) => {
            tracing::warn!("cannot run nix for cache copy {direction}: {err}");
            false
        }
        Err(_) => {
            tracing::warn!("nix copy {direction} cache timed out");
            false
        }
    }
}

async fn push(state: &AppState) {
    let targets = root_targets(state).await;
    if targets.is_empty() {
        return;
    }
    let target_args: Vec<&str> = targets.iter().map(String::as_str).collect();
    copy(state, "--to", &target_args).await;
    // Root closures only cover outputs; also cache the derivation closures,
    // which the store loses on restart but `nix-store --realise` needs.
    let drv_args: Vec<&str> = ["--derivation"]
        .into_iter()
        .chain(target_args.iter().copied())
        .collect();
    copy(state, "--to", &drv_args).await;
}

/// Fetches the cached store so resumed jobs can realize their step
/// derivations without rebuilding.
pub async fn restore(state: &AppState) {
    let targets = root_targets(state).await;
    if targets.is_empty() {
        return;
    }
    // The cache only ever holds what was pushed from root closures, so
    // restoring everything is bounded; a target-wise pull would need the
    // derivation paths, which the wiped store no longer knows.
    if copy(state, "--from", &["--all"]).await {
        return;
    }
    tracing::warn!("restoring Nix store from cache failed; jobs will substitute on demand");
}

pub async fn sync(state: AppState) {
    let Ok(_guard) = COPY_LOCK.try_lock() else {
        return;
    };
    let removed = prune(&state).await;
    if removed > 0 {
        // The cache keeps everything ever pushed, so rebuild it from the
        // remaining roots once stale roots are gone.
        clear_cache(&state).await;
    }
    push(&state).await;
}
