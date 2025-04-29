use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;
use yrs::sync::{Awareness, SyncMessage};
use yrs::updates::encoder::Encode;
use yrs::{Doc, Text, Transact, Update, updates::decoder::Decode};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for better debugging
    tracing_subscriber::fmt::init();

    // Create a Yrs document with a unique client ID
    let doc = Doc::with_client_id(rand::random());

    // Create a text shared type in the document
    let text = doc.get_or_insert_text("shared-text");

    // Create an awareness instance for this document
    let awareness = Arc::new(RwLock::new(Awareness::new(doc)));

    // Parse the WebSocket URL
    let url = Url::parse("ws://localhost:8000/signaling")?;
    println!("Connecting to {}", url);

    // Connect to the WebSocket server
    let (ws_stream, _) = connect_async(url).await?;
    println!("Connected to the server");

    // Split the WebSocket into sender and receiver
    let (mut write, mut read) = ws_stream.split();

    // First, subscribe to a room
    let subscribe_msg = Message::Text(String::from(
        "{\"type\":\"subscribe\",\"topics\":[\"room1\"]}",
    ));
    write.send(subscribe_msg).await?;
    println!("Subscribed to room1");

    // Set up a task to observe document changes and send them to the server
    let write_clone = Arc::new(tokio::sync::Mutex::new(write));
    let write_for_updates = write_clone.clone();

    // Observe document updates and send them to the server
    {
        let awareness_lock = awareness.write().await;
        let doc = awareness_lock.doc();

        let write_mutex = write_for_updates.clone();
        let _subscription = doc
            .observe_update_v1(move |_, event| {
                let update = event.update.clone();
                let write_mutex = write_mutex.clone();

                tokio::spawn(async move {
                    let msg = yrs::sync::Message::Sync(SyncMessage::Update(update)).encode_v1();
                    let mut write = write_mutex.lock().await;
                    // Wrap the Yrs protocol message in a publish message for the signaling server
                    let publish_msg = Message::Text(format!(
                        "{{\"type\":\"publish\",\"topic\":\"room1\",\"content\":{:?}}}",
                        msg
                    ));
                    if let Err(e) = write.send(publish_msg).await {
                        eprintln!("Failed to send update: {}", e);
                    }
                });
            })
            .unwrap();
    }

    // Spawn a task to handle incoming messages
    let awareness_for_read = awareness.clone();
    let write_for_receive = write_clone.clone();

    // Add a heartbeat task to keep the connection alive
    let write_for_heartbeat = write_clone.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(15));
        loop {
            interval.tick().await;
            let mut write = write_for_heartbeat.lock().await;
            // Use Signal::Ping instead of WebSocket ping
            let ping_msg = Message::Text(String::from("{\"type\":\"ping\"}"));
            if let Err(e) = write.send(ping_msg).await {
                eprintln!("Failed to send heartbeat: {}", e);
                break;
            }
            println!("Sent heartbeat ping");
        }
    });

    let receive_task = tokio::spawn(async move {
        while let Some(message) = read.next().await {
            match message {
                Ok(msg) => {
                    if let Message::Text(text) = msg {
                        println!("Received message: {}", text);

                        // Try to parse as a signaling message
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                            if let Some(msg_type) = json.get("type").and_then(|t| t.as_str()) {
                                match msg_type {
                                    "pong" => {
                                        // Server responded to our ping
                                        println!("Received pong from server");
                                        continue;
                                    }
                                    _ => {}
                                }
                            }

                            if let Some(content) = json.get("content") {
                                if let Ok(binary_data) =
                                    serde_json::from_value::<Vec<u8>>(content.clone())
                                {
                                    // This could be a Yrs protocol message
                                    if let Ok(yrs_msg) = yrs::sync::Message::decode_v1(&binary_data)
                                    {
                                        match yrs_msg {
                                            yrs::sync::Message::Sync(SyncMessage::Update(
                                                update,
                                            )) => {
                                                let mut awareness =
                                                    awareness_for_read.write().await;
                                                let update =
                                                    yrs::sync::AwarenessUpdate::decode_v1(&update)
                                                        .unwrap();
                                                awareness.apply_update(update);
                                                println!("Applied document update");
                                            }
                                            // Handle other message types as needed
                                            _ => println!("Received other Yrs message type"),
                                        }
                                    }
                                }
                            }
                        }
                    } else if let Message::Binary(data) = msg {
                        // This might be a direct Yrs protocol message
                        if let Ok(yrs_msg) = yrs::sync::Message::decode_v1(&data) {
                            match yrs_msg {
                                yrs::sync::Message::Sync(SyncMessage::Update(update)) => {
                                    let mut awareness = awareness_for_read.write().await;
                                    let update =
                                        yrs::sync::AwarenessUpdate::decode_v1(&update).unwrap();
                                    awareness.apply_update(update);
                                    println!("Applied document update from binary message");
                                }
                                // Handle other message types as needed
                                _ => println!("Received other Yrs message type from binary"),
                            }
                        } else {
                            println!("Received binary data: {} bytes", data.len());
                        }
                    } else if let Message::Close(_) = msg {
                        println!("Server closed the connection");
                        break;
                    } else if let Message::Ping(data) = msg {
                        // Respond to ping with pong
                        let mut write = write_for_receive.lock().await;
                        if let Err(e) = write.send(Message::Pong(data)).await {
                            eprintln!("Failed to respond to ping: {}", e);
                        }
                        println!("Responded to server ping");
                    } else if let Message::Pong(_) = msg {
                        // Server responded to our ping
                        println!("Received pong from server");
                    }
                }
                Err(e) => {
                    eprintln!("Error receiving message: {}", e);
                    break;
                }
            }
        }
    });

    // Simple message loop from stdin to modify the document
    let mut line = String::new();
    loop {
        line.clear();
        std::io::stdin().read_line(&mut line)?;

        // Trim the newline
        let line = line.trim();

        if line == "exit" {
            break;
        }

        // Modify the shared text
        {
            let mut awareness_lock = awareness.write().await;
            let doc = awareness_lock.doc_mut();
            let mut txn = doc.transact_mut();
            text.push(&mut txn, line);
            println!("Added text to document: {}", line);
        }
    }

    // Close the connection
    // let mut write = write_clone.lock().await;
    // write.send(Message::Close(None)).await?;

    // Wait for both tasks to complete
    let _ = tokio::try_join!(receive_task, heartbeat_task);

    println!("Connection closed");
    Ok(())
}
