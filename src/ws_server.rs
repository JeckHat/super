// src/ws_server.rs
use axum::{
    extract::{State, ws::{WebSocket, WebSocketUpgrade, Message}},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::{StreamExt, SinkExt};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

#[derive(Clone)]
pub struct WsServerHandle {
    pub tx: broadcast::Sender<String>,
}

pub fn build_router(handle: WsServerHandle) -> Router {
    Router::new()
        .route("/ws", get(ws_upgrade))
        .with_state(Arc::new(handle))
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(handle): State<Arc<WsServerHandle>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, handle))
}

async fn handle_socket(socket: WebSocket, handle: Arc<WsServerHandle>) {
    info!("WS client connected");
    // split sink & stream
    let (mut sender, mut receiver) = socket.split();

    // subscribe to broadcast channel
    let mut rx = handle.tx.subscribe();

    // task: forward broadcast -> client
    let send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(json) => {
                    // convert String -> Utf8Bytes via into()
                    if sender.send(Message::Text(json.into())).await.is_err() {
                        // client disconnected
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("client lagged {} messages", n);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });

    // task: read incoming messages from client (optional)
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            match msg {
                Message::Text(text) => {
                    info!("from client: {}", text);
                    // optionally parse commands
                }
                Message::Close(_) => {
                    info!("client closed");
                    break;
                }
                _ => {}
            }
        }
    });

    // wait until one finishes
    let _ = tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    };

    info!("WS client disconnected");
}
