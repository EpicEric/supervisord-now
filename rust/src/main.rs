mod error;
mod files;
mod jobs;
mod logs;
mod lsp;
mod state;
mod supervisor;

use std::env;
use std::path::PathBuf;

use axum::extract::DefaultBodyLimit;
use axum::http::{StatusCode, Uri, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_embed::Embed;
use serde_json::json;

use crate::state::AppState;

#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../web/dist"]
struct Assets;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "supervisord_now=info,tower_http=warn".into()),
        )
        .init();

    let workspace: PathBuf = env::var_os("NOW_WORKSPACE")
        .map(Into::into)
        .unwrap_or_else(|| "/workspace".into());
    let run_dir: PathBuf = env::var_os("NOW_RUN_DIR")
        .map(Into::into)
        .unwrap_or_else(|| "/var/lib/supervisord-now".into());
    let conf_dir: PathBuf = env::var_os("SUPERVISOR_CONF_DIR")
        .map(Into::into)
        .unwrap_or_else(|| "/etc/supervisord-now".into());
    let socket_path = env::var("SUPERVISOR_SOCK").unwrap_or_else(|_| "/supervisor.sock".into());
    let port: u16 = env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9991);

    std::fs::create_dir_all(&run_dir).expect("create run dir");
    std::fs::create_dir_all(&conf_dir).expect("create conf dir");
    ensure_workspace(&workspace);

    let state = AppState {
        workspace: workspace.clone(),
        run_dir,
        conf_dir,
        supervisor: supervisor::SupervisorClient::new(socket_path),
    };

    let app = Router::new()
        .route("/api/health", get(health))
        .route("/api/files", get(files::tree))
        .route(
            "/api/file",
            get(files::read).put(files::write).delete(files::delete),
        )
        .route("/api/dir", post(files::mkdir))
        .route("/api/rename", post(files::rename))
        .route("/api/eval", get(jobs::eval))
        .route("/api/jobs", get(jobs::list))
        .route("/api/jobs/{name}/invoke", post(jobs::invoke))
        .route("/api/jobs/{name}/stop", post(jobs::stop))
        .route("/api/jobs/{name}/logs", get(logs::stream))
        .route("/api/lsp", get(lsp::bridge))
        .fallback(get(serve_static))
        .layer(DefaultBodyLimit::max(16 * 1024 * 1024))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("bind web port");
    tracing::info!("listening on 0.0.0.0:{port}");
    axum::serve(listener, app).await.expect("server error");
}

fn ensure_workspace(workspace: &std::path::Path) {
    if let Err(err) = std::fs::create_dir_all(workspace) {
        tracing::error!("cannot create workspace {workspace:?}: {err}");
    }
    let workspace_file = workspace.join(".supervisord-now.code-workspace");
    if !workspace_file.exists() {
        let content = json!({
            "folders": [{ "uri": "file:///workspace" }],
            "settings": {}
        });
        if let Err(err) = std::fs::write(
            &workspace_file,
            serde_json::to_string_pretty(&content).unwrap(),
        ) {
            tracing::error!("cannot write workspace file: {err}");
        }
    }
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

async fn serve_static(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => match Assets::get("index.html") {
            Some(index) => (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/html")],
                index.data,
            )
                .into_response(),
            None => (StatusCode::NOT_FOUND, "frontend not built").into_response(),
        },
    }
}
