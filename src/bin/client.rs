use std::sync::Arc;
use std::time::Duration;
use axum::extract::ws::Message;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::task;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_util::codec::{Decoder, Encoder, FramedRead, FramedWrite, LengthDelimitedCodec};
use wires::client::client;
use wires::signaling::Signal;
use yrs::sync::{Awareness, Error, Message, SyncMessage};
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, Subscription, Text, Transact};




#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let doc_1 = Doc::with_client_id(1);
    let _ = doc_1.get_or_insert_text("test");

    let client_1 = client("ws://localhost:8000/signaling", doc_1).await?;

     // Subscribe to the room
     let signal = Signal::Subscribe { topics: vec!["room_1"] };
     let signal_json = serde_json::to_string(&signal)?;
     client_1.send(Message::Text(signal_json)).await?;








   
    // Keep the client running
    tokio::signal::ctrl_c().await?;
    
    Ok(())
} 