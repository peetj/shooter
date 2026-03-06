use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::{ws::Message, ws::WebSocket, ws::WebSocketUpgrade, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use shooter_shared::protocol::{decode_c2s, encode_s2c, C2s, ClientId, S2c, Snapshot};
use tokio::sync::mpsc;
use tracing::{info, warn};

#[derive(Clone, Default)]
struct AppState {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    next_id: ClientId,
    clients: HashMap<ClientId, mpsc::UnboundedSender<Message>>,
    tick: u32,
    // MVP state: positions only.
    players: HashMap<ClientId, Player>,
}

#[derive(Clone, Copy)]
struct Player {
    x_mm: i32,
    y_mm: i32,
    hp: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let state = AppState::default();

    // Game loop: snapshot broadcast.
    {
        let state2 = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(50)); // 20hz snapshots
            loop {
                interval.tick().await;
                let (snapshot, conns) = {
                    let mut inner = state2.inner.lock().unwrap();
                    inner.tick = inner.tick.wrapping_add(1);
                    let players = inner
                        .players
                        .iter()
                        .map(|(&id, p)| shooter_shared::protocol::PlayerState {
                            id,
                            x_mm: p.x_mm,
                            y_mm: p.y_mm,
                            hp: p.hp,
                        })
                        .collect::<Vec<_>>();

                    (
                        Snapshot {
                            tick: inner.tick,
                            players,
                        },
                        inner.clients.values().cloned().collect::<Vec<_>>(),
                    )
                };

                let msg = Message::Binary(encode_s2c(&S2c::Snapshot(snapshot)));
                for tx in conns {
                    let _ = tx.send(msg.clone());
                }
            }
        });
    }

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr: SocketAddr = "0.0.0.0:3000".parse()?;
    info!(%addr, "server listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(state, socket))
}

async fn handle_socket(state: AppState, mut socket: WebSocket) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    let client_id = {
        let mut inner = state.inner.lock().unwrap();
        inner.next_id = inner.next_id.wrapping_add(1);
        let id = inner.next_id;
        inner.clients.insert(id, tx);
        inner.players.insert(
            id,
            Player {
                x_mm: 0,
                y_mm: 0,
                hp: 100,
            },
        );
        id
    };

    info!(client_id, "client connected");

    // Send welcome
    let _ = socket
        .send(Message::Binary(encode_s2c(&S2c::Welcome { client_id })))
        .await;

    // Drive both reading (client->server) and writing (server->client) without cloning the socket.
    loop {
        tokio::select! {
            maybe_out = rx.recv() => {
                match maybe_out {
                    Some(msg) => {
                        if socket.send(msg).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            maybe_in = socket.recv() => {
                match maybe_in {
                    Some(Ok(msg)) => {
                        match msg {
                            Message::Binary(bytes) => match decode_c2s(&bytes) {
                                Ok(C2s::Input(input)) => {
                                    // MVP movement: apply directly server-side.
                                    let speed_mm_per_input = 80; // very rough
                                    let mut inner = state.inner.lock().unwrap();
                                    if let Some(p) = inner.players.get_mut(&client_id) {
                                        let mut dx = 0;
                                        let mut dy = 0;
                                        if input.left { dx -= 1 }
                                        if input.right { dx += 1 }
                                        if input.up { dy -= 1 }
                                        if input.down { dy += 1 }
                                        p.x_mm += dx * speed_mm_per_input;
                                        p.y_mm += dy * speed_mm_per_input;

                                        // Clamp to arena bounds (matches client draw: 1000x600 px, 10mm per px).
                                        let half_w_mm = 5000;
                                        let half_h_mm = 3000;
                                        p.x_mm = p.x_mm.clamp(-half_w_mm, half_w_mm);
                                        p.y_mm = p.y_mm.clamp(-half_h_mm, half_h_mm);
                                    }
                                }
                                Err(e) => warn!(client_id, error=%e, "bad C2S message"),
                            },
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                    _ => break,
                }
            }
        }
    }

    // Cleanup
    {
        let mut inner = state.inner.lock().unwrap();
        inner.clients.remove(&client_id);
        inner.players.remove(&client_id);
    }

    info!(client_id, "client disconnected");
}
