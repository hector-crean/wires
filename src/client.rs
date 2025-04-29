use crate::broadcast::BroadcastGroup;
use crate::conn::Connection;
use crate::ws::{AxumSink, AxumStream};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{ready, SinkExt, Stream, StreamExt};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, Notify, RwLock};
use tokio::task;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use axum::{
    Router,
    routing::get,
    extract::ws::{WebSocket, WebSocketUpgrade},
    extract::State,
    response::IntoResponse,
};
use yrs::sync::{Awareness, Error};
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, Subscription, Text, Transact};

pub async fn start_server(
    addr: &str,
    bcast: Arc<BroadcastGroup>,
) -> Result<JoinHandle<()>, Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    
    let app = Router::new()
        .route("/my-room", get(ws_handler))
        .with_state(bcast);

    Ok(tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .unwrap();
    }))
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(bcast): State<Arc<BroadcastGroup>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| peer(socket, bcast))
}

pub async fn peer(ws: WebSocket, bcast: Arc<BroadcastGroup>) {
    let (sink, stream) = ws.split();
    let sink = Arc::new(Mutex::new(AxumSink(sink)));
    let stream = AxumStream(stream);
    let sub = bcast.subscribe(sink, stream);
    match sub.completed().await {
        Ok(_) => println!("broadcasting for channel finished successfully"),
        Err(e) => eprintln!("broadcasting for channel finished abruptly: {}", e),
    }
}

pub struct TungsteniteSink(SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>);

impl futures_util::Sink<Vec<u8>> for TungsteniteSink {
    type Error = Error;

    fn poll_ready(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let sink = unsafe { Pin::new_unchecked(&mut self.0) };
        let result = ready!(sink.poll_ready(cx));
        match result {
            Ok(_) => Poll::Ready(Ok(())),
            Err(e) => Poll::Ready(Err(Error::Other(Box::new(e)))),
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        let sink = unsafe { Pin::new_unchecked(&mut self.0) };
        let result = sink.start_send(Message::binary(item));
        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(Error::Other(Box::new(e))),
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let sink = unsafe { Pin::new_unchecked(&mut self.0) };
        let result = ready!(sink.poll_flush(cx));
        match result {
            Ok(_) => Poll::Ready(Ok(())),
            Err(e) => Poll::Ready(Err(Error::Other(Box::new(e)))),
        }
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        let sink = unsafe { Pin::new_unchecked(&mut self.0) };
        let result = ready!(sink.poll_close(cx));
        match result {
            Ok(_) => Poll::Ready(Ok(())),
            Err(e) => Poll::Ready(Err(Error::Other(Box::new(e)))),
        }
    }
}

pub struct TungsteniteStream(SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>);
impl Stream for TungsteniteStream {
    type Item = Result<Vec<u8>, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let stream = unsafe { Pin::new_unchecked(&mut self.0) };
        let result = ready!(stream.poll_next(cx));
        match result {
            None => Poll::Ready(None),
            Some(Ok(msg)) => Poll::Ready(Some(Ok(msg.into_data()))),
            Some(Err(e)) => Poll::Ready(Some(Err(Error::Other(Box::new(e))))),
        }
    }
}

pub async fn client(
    addr: &str,
    doc: Doc,
) -> Result<Connection<TungsteniteSink, TungsteniteStream>, Box<dyn std::error::Error>> {
    let (stream, _) = tokio_tungstenite::connect_async(addr).await?;
    let (sink, stream) = stream.split();
    let sink = TungsteniteSink(sink);
    let stream = TungsteniteStream(stream);
    Ok(Connection::new(
        Arc::new(RwLock::new(Awareness::new(doc))),
        sink,
        stream,
    ))
}

pub fn create_notifier(doc: &Doc) -> (Arc<Notify>, Subscription) {
    let n = Arc::new(Notify::new());
    let sub = {
        let n = n.clone();
        doc.observe_update_v1(move |_, _| n.notify_waiters())
            .unwrap()
    };
    (n, sub)
}

pub const TIMEOUT: Duration = Duration::from_secs(5);