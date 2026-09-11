use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use crate::state::AppState;

pub async fn bridge(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle(socket, state))
}

async fn handle(socket: WebSocket, state: AppState) {
    let mut child = match Command::new("nil")
        .current_dir(&state.workspace)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            tracing::error!("failed to start nil: {err}");
            let mut socket = socket;
            let _ = socket
                .send(Message::Text(format!("nil failed to start: {err}").into()))
                .await;
            return;
        }
    };
    tracing::debug!("nil spawned (pid {:?})", child.id());

    let mut stdin = child.stdin.take().expect("nil stdin");
    let mut stdout = child.stdout.take().expect("nil stdout");
    let mut stderr = child.stderr.take().expect("nil stderr");

    let (mut sender, mut receiver) = socket.split();

    let mut to_server = tokio::spawn(async move {
        while let Some(Ok(message)) = receiver.next().await {
            tracing::debug!(
                "ws -> nil: {} bytes",
                match &message {
                    Message::Text(t) => t.len(),
                    Message::Binary(b) => b.len(),
                    _ => 0,
                }
            );
            if let Message::Text(text) = message {
                let frame = format!("Content-Length: {}\r\n\r\n{}", text.len(), text);
                if stdin.write_all(frame.as_bytes()).await.is_err() {
                    tracing::debug!("stdin write failed");
                    break;
                }
                let _ = stdin.flush().await;
            }
        }
        tracing::debug!("to_server task ended");
    });

    let mut from_server = tokio::spawn(async move {
        let mut buffer = Vec::with_capacity(16 * 1024);
        let mut chunk = [0u8; 16 * 1024];
        loop {
            let read = match stdout.read(&mut chunk).await {
                Ok(0) | Err(_) => {
                    tracing::debug!("nil stdout closed");
                    break;
                }
                Ok(n) => n,
            };
            buffer.extend_from_slice(&chunk[..read]);
            while let Some(frame) = extract_lsp_frame(&mut buffer) {
                let json = String::from_utf8_lossy(&frame);
                tracing::debug!("nil -> ws: {} bytes", json.len());
                if sender
                    .send(Message::Text(json.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    });

    let stderr_task = tokio::spawn(async move {
        let mut buffer = [0u8; 4096];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    tracing::debug!(target: "supervisord_now::lsp", "nil stderr: {}", String::from_utf8_lossy(&buffer[..n]));
                }
            }
        }
    });

    tokio::select! {
        res = &mut to_server => {
            tracing::debug!("select: to_server ended ({res:?})");
        }
        res = &mut from_server => {
            tracing::debug!("select: from_server ended ({res:?})");
        }
    }
    child.kill().await.ok();
    child.wait().await.ok();
    to_server.abort();
    from_server.abort();
    stderr_task.abort();
}

fn extract_lsp_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let header_end = buffer.windows(4).position(|w| w == b"\r\n\r\n")?;
    let header = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = parse_content_length(&header)?;
    let total = header_end + 4 + content_length;
    if buffer.len() < total {
        return None;
    }
    let body = buffer[header_end + 4..total].to_vec();
    buffer.drain(..total);
    Some(body)
}

fn parse_content_length(header: &str) -> Option<usize> {
    for line in header.split("\r\n") {
        if let Some(value) = line.strip_prefix("Content-Length:") {
            return value.trim().parse().ok();
        }
    }
    None
}
