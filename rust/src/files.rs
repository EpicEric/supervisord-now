use std::fs;
use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct FilePathParams {
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct FileNode {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileNode>>,
}

#[derive(Deserialize)]
pub struct FileContent {
    pub content: String,
}

#[derive(Deserialize)]
pub struct RenameRequest {
    pub from: String,
    pub to: String,
}

const IGNORED_DIRS: &[&str] = &[".git", ".jj", "node_modules", ".direnv", "result"];

pub fn safe_path(workspace: &Path, rel: &str) -> Result<PathBuf, ApiError> {
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() {
        return Err(ApiError::bad_request("path must not be empty"));
    }
    let mut resolved = workspace.to_path_buf();
    for component in Path::new(rel).components() {
        match component {
            std::path::Component::Normal(part) => resolved.push(part),
            std::path::Component::CurDir => {}
            _ => return Err(ApiError::bad_request("invalid path")),
        }
    }
    Ok(resolved)
}

fn is_ignored(name: &str) -> bool {
    IGNORED_DIRS.contains(&name)
}

fn read_tree(dir: &Path, rel_prefix: &str) -> ApiResult<Vec<FileNode>> {
    let mut nodes = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| ApiError::internal(format!("read dir: {e}")))? {
        let entry = entry.map_err(|e| ApiError::internal(e.to_string()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_ignored(&name) {
            continue;
        }
        let rel = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        let metadata = entry
            .metadata()
            .map_err(|e| ApiError::internal(e.to_string()))?;
        if metadata.is_dir() {
            nodes.push(FileNode {
                name,
                path: rel.clone(),
                kind: "dir".into(),
                size: None,
                children: Some(read_tree(&entry.path(), &rel)?),
            });
        } else if metadata.is_file() {
            nodes.push(FileNode {
                name,
                path: rel,
                kind: "file".into(),
                size: Some(metadata.len()),
                children: None,
            });
        }
    }
    nodes.sort_by(|a, b| {
        let kind = b.kind.cmp(&a.kind);
        kind.then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(nodes)
}

pub async fn tree(State(state): State<AppState>) -> ApiResult<Json<FileNode>> {
    let children = read_tree(&state.workspace, "")?;
    Ok(Json(FileNode {
        name: "workspace".into(),
        path: "".into(),
        kind: "dir".into(),
        size: None,
        children: Some(children),
    }))
}

pub async fn read(
    State(state): State<AppState>,
    Query(params): Query<FilePathParams>,
) -> ApiResult<Json<serde_json::Value>> {
    let rel = params.path.as_deref().unwrap_or_default();
    let path = safe_path(&state.workspace, rel)?;
    let bytes = fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ApiError::not_found(format!("file not found: {rel}"))
        } else {
            ApiError::internal(format!("read file: {e}"))
        }
    })?;
    match String::from_utf8(bytes) {
        Ok(content) => Ok(Json(json!({ "content": content }))),
        Err(_) => Err(ApiError::unprocessable(format!(
            "binary file not supported: {rel}"
        ))),
    }
}

pub async fn write(
    State(state): State<AppState>,
    Query(params): Query<FilePathParams>,
    Json(body): Json<FileContent>,
) -> ApiResult<StatusCode> {
    let rel = params.path.as_deref().unwrap_or_default();
    let path = safe_path(&state.workspace, rel)?;
    if path.extension().map(|e| e == "nix").is_none() && looks_binary(&body.content) {
        return Err(ApiError::unprocessable("refusing to write binary content"));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, body.content.as_bytes())
        .map_err(|e| ApiError::internal(format!("write file: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

fn looks_binary(content: &str) -> bool {
    content
        .chars()
        .any(|c| c == '\0' || (c.is_control() && c != '\n' && c != '\t' && c != '\r'))
}

pub async fn mkdir(
    State(state): State<AppState>,
    Query(params): Query<FilePathParams>,
) -> ApiResult<StatusCode> {
    let rel = params.path.as_deref().unwrap_or_default();
    let path = safe_path(&state.workspace, rel)?;
    fs::create_dir_all(&path)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn rename(
    State(state): State<AppState>,
    Json(body): Json<RenameRequest>,
) -> ApiResult<StatusCode> {
    let from = safe_path(&state.workspace, &body.from)?;
    let to = safe_path(&state.workspace, &body.to)?;
    if !from.exists() {
        return Err(ApiError::not_found(format!(
            "path not found: {}",
            body.from
        )));
    }
    if to.exists() {
        return Err(ApiError::bad_request("target already exists"));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&from, &to).map_err(|e| ApiError::internal(format!("rename: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(state): State<AppState>,
    Query(params): Query<FilePathParams>,
) -> ApiResult<StatusCode> {
    let rel = params.path.as_deref().unwrap_or_default();
    let path = safe_path(&state.workspace, rel)?;
    let metadata = fs::symlink_metadata(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ApiError::not_found(format!("path not found: {rel}"))
        } else {
            ApiError::internal(e.to_string())
        }
    })?;
    let result = if metadata.is_dir() {
        fs::remove_dir_all(&path)
    } else {
        fs::remove_file(&path)
    };
    result.map_err(|e| ApiError::internal(format!("delete: {e}")))?;
    Ok(StatusCode::NO_CONTENT)
}

impl IntoResponse for FileNode {
    fn into_response(self) -> axum::response::Response {
        Json(self).into_response()
    }
}
