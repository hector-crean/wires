use axum::extract::ws;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Signaling error: {0}")]
    Signaling(#[from] SignalingError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error(transparent)]
    Axum(#[from] axum::Error),

    #[error("Unexpected error: {0}")]
    Other(String),
}

#[derive(Error, Debug)]
pub enum SignalingError {
    #[error("Failed to send message: {0}")]
    MessageSend(String),

    #[error("Topic not found: {0}")]
    TopicNotFound(String),

    #[error("Invalid message format")]
    InvalidMessageFormat,

    #[error("Connection timeout")]
    ConnectionTimeout,
}
