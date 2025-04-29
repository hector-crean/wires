use std::sync::Arc;
use tokio::sync::RwLock;
use yrs::{Doc, Text, Transact, GetString};
use wires::client::client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a new Yjs document
    let doc = Doc::new();
    
    // Connect to the server and join a room
    let conn = client("ws://localhost:8080", "my-room", doc).await?;
    
    // Get the shared document
    let doc = conn.awareness().read().await.doc().clone();
    
    // Create a text type in the document
    let text = doc.get_or_insert_text("text");
    
    // Insert some text
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 0, "Hello, World!");
    }
    
    // Print the current text content
    {
        let txn = doc.transact();
        println!("Current text: {}", text.get_string(&txn));
    }
    
    // Keep the client running
    tokio::signal::ctrl_c().await?;
    
    Ok(())
} 