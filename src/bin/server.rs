use axum::{
    Router,
    extract::{
        State,
        ws::{WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use tracing::{Level, error, info};
use tracing_subscriber::FmtSubscriber;
use wires::signaling::{SignalingService, signaling_conn};

#[tokio::main]
async fn main() {
    // Initialize the tracing subscriber for logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

    info!("Starting signaling server");
    let signaling = SignalingService::new();

    let app = Router::new()
        .route("/signaling", get(ws_handler))
        .with_state(signaling);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await.unwrap();
    info!("Listening on {}", listener.local_addr().unwrap());

    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(svc): State<SignalingService>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| peer(socket, svc))
}

async fn peer(ws: WebSocket, svc: SignalingService) {
    info!("New incoming signaling connection");
    match signaling_conn(ws, svc).await {
        Ok(_) => info!("Signaling connection stopped"),
        Err(e) => error!("Signaling connection failed: {}", e),
    }
}
