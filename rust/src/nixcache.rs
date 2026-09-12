use std::path::PathBuf;
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::Mutex;

use crate::state::AppState;

pub const CACHE_DIR: &str = "nix-cache";
pub const GCROOTS_DIR: &str = "gcroots";
const COPY_TIMEOUT: Duration = Duration::from_secs(600);

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
    push(&state).await;
}
