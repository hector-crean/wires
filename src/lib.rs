use std::sync::Arc;
use tokio::sync::RwLock;

pub mod client;
pub mod broadcast;
pub mod conn;
pub mod error;
pub mod signaling;
pub mod ws;

pub type AwarenessRef = Arc<RwLock<yrs::sync::Awareness>>;
