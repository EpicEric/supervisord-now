use std::path::Path;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::process::Command;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::supervisor::ProcessInfo;

const PROGRAM_PREFIX: &str = "now-job-";

pub fn program_name(job: &str) -> Result<String, ApiError> {
    if job.is_empty() || job.len() > 128 {
        return Err(ApiError::bad_request("invalid job name"));
    }
    if !job
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(ApiError::bad_request(
            "job names may only contain [A-Za-z0-9._-] characters",
        ));
    }
    Ok(format!("{PROGRAM_PREFIX}{job}"))
}

fn workspace_mode(workspace: &std::path::Path) -> Mode {
    if workspace.join("flake.nix").exists() {
        Mode::Flake
    } else {
        Mode::Workflow
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Workflow,
    Flake,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::Workflow => "workflow",
            Mode::Flake => "flake",
        }
    }

    fn eval_args(self, workspace: &Path) -> Vec<String> {
        match self {
            Mode::Workflow => {
                vec![
                    "--workflow".into(),
                    workspace.join("now.nix").to_string_lossy().to_string(),
                ]
            }
            Mode::Flake => vec!["--flake".into(), workspace.to_string_lossy().to_string()],
        }
    }

    fn run_args(self, workspace: &Path) -> Vec<String> {
        let mut args = match self {
            Mode::Workflow => {
                vec![
                    "--workflow".into(),
                    workspace.join("now.nix").to_string_lossy().to_string(),
                ]
            }
            Mode::Flake => vec!["--flake".into(), workspace.to_string_lossy().to_string()],
        };
        args.push("--env-file".into());
        args
    }
}

#[derive(Serialize)]
pub struct JobListing {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct EvalOutput {
    #[serde(default)]
    jobs: serde_json::Map<String, serde_json::Value>,
}

pub async fn eval(State(state): State<AppState>) -> ApiResult<Json<serde_json::Value>> {
    if !state.workspace.join("flake.nix").exists() && !state.workspace.join("now.nix").exists() {
        return Ok(Json(json!({ "mode": "empty", "jobs": [] })));
    }
    let mode = workspace_mode(&state.workspace);
    let mut cmd = Command::new("now");
    cmd.arg("eval")
        .args(mode.eval_args(&state.workspace))
        .current_dir(&state.workspace);
    let output = cmd
        .output()
        .await
        .map_err(|e| ApiError::internal(format!("failed to run 'now eval': {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ApiError::unprocessable(format!(
            "now eval failed:\n{}",
            stderr.trim()
        )));
    }

    let parsed: EvalOutput = serde_json::from_slice(&output.stdout)
        .map_err(|e| ApiError::unprocessable(format!("could not parse 'now eval' output: {e}")))?;

    let jobs: Vec<JobListing> = parsed
        .jobs
        .into_iter()
        .map(|(id, job)| JobListing {
            name: job
                .get("name")
                .and_then(|name| name.as_str())
                .unwrap_or(&id)
                .to_string(),
            id,
            needs: job.get("needs").and_then(|n| n.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            }),
        })
        .collect();

    Ok(Json(json!({ "mode": mode.as_str(), "jobs": jobs })))
}

#[derive(Serialize)]
pub struct JobStatus {
    pub name: String,
    pub group: String,
    pub state: i64,
    pub statename: String,
    pub description: String,
    pub exitstatus: i64,
    pub spawnerr: String,
}

impl From<(&str, &ProcessInfo)> for JobStatus {
    fn from((job, info): (&str, &ProcessInfo)) -> Self {
        // ochinchina/supervisord races its exit-state transition and reports
        // normally-exiting programs as Fatal when startretries=0. A Fatal
        // state without a spawn error and with exit code 0 is a completed job.
        let statename =
            if info.statename == "Fatal" && info.spawnerr.is_empty() && info.exitstatus == 0 {
                "Exited".to_string()
            } else {
                info.statename.clone()
            };
        Self {
            name: job.to_string(),
            group: info.group.clone(),
            state: info.state,
            statename,
            description: info.description.clone(),
            exitstatus: info.exitstatus,
            spawnerr: info.spawnerr.clone(),
        }
    }
}

pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Vec<JobStatus>>> {
    let infos = state.supervisor.get_all_process_info().await?;
    let mut jobs: Vec<JobStatus> = infos
        .iter()
        .filter_map(|info| {
            info.group
                .strip_prefix(PROGRAM_PREFIX)
                .map(|job| (job, info).into())
        })
        .collect();
    jobs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(jobs))
}

#[derive(Deserialize)]
pub struct InvokeBody {
    #[serde(default)]
    pub vars: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: std::collections::BTreeMap<String, String>,
}

pub async fn invoke(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(body): Json<InvokeBody>,
) -> ApiResult<Json<serde_json::Value>> {
    let group = program_name(&name)?;
    let mode = workspace_mode(&state.workspace);
    if mode == Mode::Workflow && !state.workspace.join("now.nix").exists() {
        return Err(ApiError::bad_request("no now.nix in workspace"));
    }

    let env_file = state.run_dir.join(format!("{group}.env"));
    tokio::fs::create_dir_all(&state.run_dir).await?;
    write_env_file(&env_file, &body.vars, &body.secrets).await?;

    let conf_file = state.conf_dir.join(format!("{group}.conf"));
    tokio::fs::create_dir_all(&state.conf_dir).await?;
    let conf = render_program_conf(&group, &name, &mode, &state, &env_file);
    tokio::fs::write(&conf_file, conf).await?;

    // Reset the group so the new env file is picked up on the next start.
    if let Err(err) = state.supervisor.stop_process_group(&group, true).await {
        tracing::debug!("stop during invoke (expected if not running): {err}");
    }
    if let Err(err) = state.supervisor.remove_process_group(&group).await {
        tracing::debug!("remove during invoke (expected if never added): {err}");
    }
    state.supervisor.reload_config().await?;
    state.supervisor.add_process_group(&group).await?;

    // The group may already be running if the program section did not change.
    if let Err(err) = state.supervisor.start_process_group(&group, false).await {
        tracing::debug!("start during invoke (may already be running): {err}");
    }

    Ok(Json(json!({ "group": group, "status": "requested" })))
}

pub async fn stop(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> ApiResult<StatusCode> {
    let group = program_name(&name)?;
    state.supervisor.stop_process_group(&group, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn render_program_conf(
    group: &str,
    job: &str,
    mode: &Mode,
    state: &AppState,
    env_file: &Path,
) -> String {
    let mut args = mode.run_args(&state.workspace);
    args.push(env_file.to_string_lossy().to_string());
    args.push(job.to_string());
    let command = format!("now run {}", args.join(" "));
    format!(
        "[program:{group}]\n\
         command={command}\n\
         directory={}\n\
         autorestart=false\n\
         startsecs=0\n\
         startretries=0\n\
         stopsignal=TERM\n\
         stopasgroup=true\n\
         killasgroup=true\n\
         redirect_stderr=true\n\
         stdout_logfile={}\n\
         stdout_logfile_maxbytes=20971520\n\
         stdout_logfile_backups=0\n",
        state.workspace.to_string_lossy(),
        state.run_dir.join(format!("{group}.log")).to_string_lossy(),
    )
}

async fn write_env_file(
    path: &Path,
    vars: &std::collections::BTreeMap<String, String>,
    secrets: &std::collections::BTreeMap<String, String>,
) -> ApiResult<()> {
    for (key, _) in vars.iter().chain(secrets.iter()) {
        if !key
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(ApiError::bad_request(format!(
                "invalid environment variable name: {key}"
            )));
        }
    }
    let mut content = String::from("# managed by supervisord-now; regenerated on each invoke\n");
    for (key, value) in vars.iter().chain(secrets.iter()) {
        if value.contains('\n') || value.contains('\r') {
            return Err(ApiError::bad_request(format!(
                "value for '{key}' must not contain newlines"
            )));
        }
        content.push_str(&format!("{key}={value}\n"));
    }
    tokio::fs::write(path, content).await?;
    Ok(())
}
