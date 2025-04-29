use axum::extract::ws::{Message, WebSocket};
use bytes::Bytes;
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::select;
use tokio::sync::{Mutex, RwLock};
use tokio::time::interval;
use tracing::{info, error};
use thiserror::Error;

use crate::error::{ServerError, SignalingError};

const PING_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_ROOMS_PER_CLIENT: usize = 10;
const MAX_CLIENTS_PER_ROOM: usize = 100;
const MAX_TOPIC_LENGTH: usize = 100;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Signal<'a> {
    #[serde(rename = "publish")]
    Publish { topic: &'a str },
    #[serde(rename = "subscribe")]
    Subscribe { topics: Vec<&'a str> },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { topics: Vec<&'a str> },
}

impl<'a> Signal<'a> {
    pub fn from_json(json: &'a str) -> Result<Self, ServerError> {
        let signal = serde_json::from_str::<Signal>(json)?;
        Ok(signal)
    }
    pub fn to_json(&self) -> Result<String, ServerError> {
        let json = serde_json::to_string(self)?;
        Ok(json)
    }
}

#[derive(Error, Debug)]
pub enum RoomError {
    #[error("room '{0}' not found")]
    RoomNotFound(String),
    #[error("room '{0}' is full (max {MAX_CLIENTS_PER_ROOM} clients)")]
    RoomFull(String),
    #[error("client has reached maximum number of rooms ({MAX_ROOMS_PER_CLIENT})")]
    TooManyRooms,
    #[error("invalid topic name: {0}")]
    InvalidTopic(String),
    #[error("failed to send message: {0}")]
    SendError(#[from] ServerError),
}

impl From<RoomError> for ServerError {
    fn from(err: RoomError) -> Self {
        match err {
            RoomError::RoomNotFound(topic) => ServerError::Signaling(SignalingError::TopicNotFound(topic)),
            RoomError::SendError(e) => e,
            RoomError::InvalidTopic(msg) => ServerError::Signaling(SignalingError::InvalidMessageFormat),
            RoomError::RoomFull(_) | RoomError::TooManyRooms => ServerError::Signaling(SignalingError::MessageSend(err.to_string())),
        }
    }
}

#[derive(Debug, Clone)]
struct RoomState {
    client_count: usize,
    created_at: std::time::Instant,
    last_activity: std::time::Instant,
}

impl Default for RoomState {
    fn default() -> Self {
        Self {
            client_count: 0,
            created_at: std::time::Instant::now(),
            last_activity: std::time::Instant::now(),
        }
    }
}

type Topics = Arc<RwLock<HashMap<Arc<str>, (HashSet<WsSink>, RoomState)>>>;

#[derive(Debug)]
struct RoomManager {
    topics: Topics,
}

impl Clone for RoomManager {
    fn clone(&self) -> Self {
        Self {
            topics: self.topics.clone(),
        }
    }
}

impl RoomManager {
    fn new() -> Self {
        Self {
            topics: Arc::new(RwLock::new(Default::default())),
        }
    }

    fn validate_topic(topic: &str) -> Result<(), RoomError> {
        if topic.is_empty() {
            return Err(RoomError::InvalidTopic("topic cannot be empty".to_string()));
        }
        if topic.len() > MAX_TOPIC_LENGTH {
            return Err(RoomError::InvalidTopic(format!(
                "topic length exceeds maximum of {MAX_TOPIC_LENGTH} characters"
            )));
        }
        if !topic.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(RoomError::InvalidTopic(
                "topic can only contain alphanumeric characters, hyphens, and underscores".to_string(),
            ));
        }
        Ok(())
    }

    async fn get_client_room_count(&self, ws: &WsSink) -> usize {
        let topics = self.topics.read().await;
        topics
            .values()
            .filter(|(subs, _)| subs.contains(ws))
            .count()
    }

    async fn subscribe(&self, topic: &str, ws: &WsSink) -> Result<(), RoomError> {
        Self::validate_topic(topic)?;

        let room_count = self.get_client_room_count(ws).await;
        if room_count >= MAX_ROOMS_PER_CLIENT {
            return Err(RoomError::TooManyRooms);
        }

        let mut topics = self.topics.write().await;
        let topic: Arc<str> = topic.into();
        
        let (subs, state) = topics.entry(topic.clone()).or_insert_with(|| {
            (HashSet::new(), RoomState::default())
        });

        if subs.len() >= MAX_CLIENTS_PER_ROOM {
            return Err(RoomError::RoomFull(topic.to_string()));
        }

        tracing::info!("Client subscribing to room '{topic}'");
        subs.insert(ws.clone());
        state.client_count = subs.len();
        state.last_activity = std::time::Instant::now();
        tracing::info!("Room '{topic}' now has {} clients", state.client_count);
        
        Ok(())
    }

    async fn unsubscribe(&self, topic: &str, ws: &WsSink) -> Result<(), RoomError> {
        let mut topics = self.topics.write().await;
        if let Some((subs, state)) = topics.get_mut(topic) {
            tracing::trace!("unsubscribing client from '{topic}'");
            subs.remove(ws);
            state.client_count = subs.len();
            state.last_activity = std::time::Instant::now();
            
            if subs.is_empty() {
                topics.remove(topic);
                tracing::info!("Room '{topic}' removed as it has no subscribers");
            }
        }
        Ok(())
    }

    async fn publish(&self, topic: &str, msg: &str) -> Result<(), RoomError> {
        let mut failed = Vec::new();
        {
            let mut topics = self.topics.write().await;
            if let Some((receivers, state)) = topics.get_mut(topic) {
                let client_count = receivers.len();
                tracing::trace!("publishing on {client_count} clients at '{topic}': {msg}");
                for receiver in receivers.iter() {
                    if let Err(e) = receiver.try_send(Message::text(msg)).await {
                        tracing::info!("failed to publish message {msg} on '{topic}': {e}");
                        failed.push(receiver.clone());
                    }
                }
                state.last_activity = std::time::Instant::now();
            } else {
                return Err(RoomError::RoomNotFound(topic.to_string()));
            }
        }
        if !failed.is_empty() {
            let mut topics = self.topics.write().await;
            if let Some((receivers, state)) = topics.get_mut(topic) {
                for f in failed {
                    receivers.remove(&f);
                }
                state.client_count = receivers.len();
                state.last_activity = std::time::Instant::now();
            }
        }
        Ok(())
    }

    async fn remove_client(&self, ws: &WsSink) -> Result<(), RoomError> {
        let mut topics = self.topics.write().await;
        let mut empty_topics = Vec::new();
        
        for (topic, (subs, state)) in topics.iter_mut() {
            subs.remove(ws);
            state.client_count = subs.len();
            state.last_activity = std::time::Instant::now();
            if subs.is_empty() {
                empty_topics.push(topic.clone());
            }
        }
        
        for topic in empty_topics {
            topics.remove(&topic);
            tracing::info!("Room '{topic}' removed as it has no subscribers");
        }
        
        Ok(())
    }

    async fn get_room_stats(&self) -> HashMap<String, RoomStats> {
        let topics = self.topics.read().await;
        topics
            .iter()
            .map(|(topic, (_, state))| {
                (
                    topic.to_string(),
                    RoomStats {
                        client_count: state.client_count,
                        age_seconds: state.created_at.elapsed().as_secs(),
                        last_activity_seconds: state.last_activity.elapsed().as_secs(),
                    },
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct RoomStats {
    pub client_count: usize,
    pub age_seconds: u64,
    pub last_activity_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct SignalingService(RoomManager);

impl SignalingService {
    pub fn new() -> Self {
        SignalingService(RoomManager::new())
    }

    pub async fn publish(&self, topic: &str, msg: Message) -> Result<(), RoomError> {
        if let Message::Text(txt) = msg {
            self.0.publish(topic, txt.as_str()).await
        } else {
            Ok(())
        }
    }

    pub async fn close_topic(&self, topic: &str) -> Result<(), RoomError> {
        let mut topics = self.0.topics.write().await;
        if let Some((subs, _)) = topics.remove(topic) {
            for sub in subs {
                if let Err(e) = sub.close().await {
                    tracing::warn!("failed to close connection on topic '{topic}': {e}");
                }
            }
        }
        Ok(())
    }

    pub async fn close(self) -> Result<(), RoomError> {
        let mut topics = self.0.topics.write_owned().await;
        let mut all_conns = HashSet::new();
        for (_, (subs, _)) in topics.drain() {
            for sub in subs {
                all_conns.insert(sub);
            }
        }

        for conn in all_conns {
            if let Err(e) = conn.close().await {
                tracing::warn!("failed to close connection: {e}");
            }
        }

        Ok(())
    }

    pub async fn get_room_stats(&self) -> HashMap<String, RoomStats> {
        self.0.get_room_stats().await
    }
}

impl Default for SignalingService {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
struct WsSink(Arc<Mutex<SplitSink<WebSocket, Message>>>);

impl WsSink {
    fn new(sink: SplitSink<WebSocket, Message>) -> Self {
        WsSink(Arc::new(Mutex::new(sink)))
    }

    async fn try_send(&self, msg: Message) -> Result<(), ServerError> {
        let mut sink = self.0.lock().await;
        if let Err(e) = sink.send(msg).await {
            sink.close().await?;
            Err(ServerError::Axum(e))
        } else {
            Ok(())
        }
    }

    async fn close(&self) -> Result<(), ServerError> {
        let mut sink = self.0.lock().await;
        sink.close().await?;
        Ok(())
    }
}

impl Hash for WsSink {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let ptr = Arc::as_ptr(&self.0) as usize;
        ptr.hash(state);
    }
}

impl PartialEq<Self> for WsSink {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for WsSink {}

/// Handle incoming signaling connection - it's a websocket connection used by y-webrtc protocol
/// to exchange offering metadata between y-webrtc peers. It also manages topic/room access.
pub async fn signaling_conn(ws: WebSocket, service: SignalingService) -> Result<(), ServerError> {
    let (sink, mut stream) = ws.split();
    let ws = WsSink::new(sink);
    let mut ping_interval = interval(PING_TIMEOUT);
    let mut state = ConnState::default();
    loop {
        select! {
            _ = ping_interval.tick() => {
                if !state.pong_received {
                    ws.close().await?;
                    drop(ping_interval);
                    return Ok(());
                }
                state.pong_received = false;
                ws.try_send(Message::Ping(Bytes::default())).await?;
            },
            res = stream.next() => {
                match res {
                    None => {
                        ws.close().await?;
                        return Ok(());
                    },
                    Some(Err(e)) => {
                        ws.close().await?;
                        return Err(ServerError::Axum(e));
                    },
                    Some(Ok(msg)) => {
                        process_msg(msg, &ws, &mut state, &service).await?;
                    }
                }
            }
        }
    }
}

async fn process_msg(
    msg: Message,
    ws: &WsSink,
    state: &mut ConnState,
    service: &SignalingService,
) -> Result<(), ServerError> {
    match msg {
        Message::Text(txt) => {
            let json = txt.as_str();
            let signal = Signal::from_json(json)?;
            match signal {
                Signal::Subscribe { topics: topic_names } => {
                    for topic in topic_names {
                        service.0.subscribe(topic, ws).await?;
                        state.subscribed_topics.insert(topic.into());
                    }
                }
                Signal::Unsubscribe { topics: topic_names } => {
                    for topic in topic_names {
                        service.0.unsubscribe(topic, ws).await?;
                        state.subscribed_topics.remove(topic);
                    }
                }
                Signal::Publish { topic } => {
                    service.0.publish(topic, json).await?;
                }
            }
        }
        Message::Binary(_bytes) => {
            info!(">>>Binary");
        }
        Message::Close(_close_frame) => {
            service.0.remove_client(ws).await?;
            state.closed = true;
        }
        Message::Ping(_bytes) => {
            ws.try_send(Message::Pong(Bytes::default())).await?;
        }
        Message::Pong(_) => {
            state.pong_received = true;
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ConnState {
    closed: bool,
    pong_received: bool,
    subscribed_topics: HashSet<Arc<str>>,
}

impl Default for ConnState {
    fn default() -> Self {
        ConnState {
            closed: false,
            pong_received: true,
            subscribed_topics: HashSet::new(),
        }
    }
}
