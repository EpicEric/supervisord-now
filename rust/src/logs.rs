use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use serde_json::json;

use crate::jobs;
use crate::state::AppState;

const POLL_INTERVAL: Duration = Duration::from_millis(400);
const CHUNK_SIZE: i64 = 16 * 1024;

pub async fn stream(
    ws: WebSocketUpgrade,
    Path(name): Path<String>,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle(socket, state, name))
}

async fn handle(mut socket: WebSocket, state: AppState, job: String) {
    let group = match jobs::program_name(&job) {
        Ok(group) => group,
        Err(err) => {
            let _ = socket
                .send(Message::Text(
                    json!({ "error": err.message }).to_string().into(),
                ))
                .await;
            return;
        }
    };

    // ochinchina/supervisord rejects negative offsets, so bootstrap by
    // reading the existing log from the start with a generous window.
    let mut offset: i64 = 0;
    let mut first_read = true;

    loop {
        let length = if first_read {
            first_read = false;
            512 * 1024
        } else {
            CHUNK_SIZE
        };
        let result = state
            .supervisor
            .tail_process_stdout_log(&group, offset, length)
            .await;
        tracing::debug!(
            "log tail {group} offset={offset} -> {:?}",
            result
                .as_ref()
                .map(|t| (t.offset, t.data.len(), t.overflow))
        );

        match result {
            Ok(tail) => {
                if tail.overflow {
                    tracing::debug!(
                        "log overflow for {group}: skipped to offset {}",
                        tail.offset
                    );
                }
                offset = tail.offset;
                if !tail.data.is_empty() {
                    let message = json!({ "data": tail.data }).to_string();
                    if socket.send(Message::Text(message.into())).await.is_err() {
                        break;
                    }
                }
            }
            Err(err) => {
                let message = json!({ "error": err.message, "fatal": true }).to_string();
                let _ = socket.send(Message::Text(message.into())).await;
                break;
            }
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
