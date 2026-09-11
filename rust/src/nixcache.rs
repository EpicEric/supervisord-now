use std::time::Duration;

use tokio::process::Command;
use tokio::sync::Mutex;

use crate::state::AppState;

pub const CACHE_DIR: &str = "nix-cache";
const PUSH_TIMEOUT: Duration = Duration::from_secs(600);

static PUSH_LOCK: Mutex<()> = Mutex::const_new(());

fn cache_url(state: &AppState) -> String {
    format!("file://{}", state.run_dir.join(CACHE_DIR).display())
}

async fn push(state: &AppState) {
    let result = tokio::time::timeout(PUSH_TIMEOUT, async {
        Command::new("nix")
            .args(["copy", "--all", "--to"])
            .arg(cache_url(state))
            .output()
            .await
    })
    .await;
    match result {
        Ok(Ok(output)) if output.status.success() => {}
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!("nix cache push failed: {}", stderr.trim());
        }
        Ok(Err(err)) => tracing::debug!("cannot run nix for cache push: {err}"),
        Err(_) => tracing::debug!("nix cache push timed out"),
    }
}

pub async fn sync(state: AppState) {
    let Ok(_guard) = PUSH_LOCK.try_lock() else {
        return;
    };
    push(&state).await;
}
