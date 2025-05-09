use crate::conn::Connection;
use crate::AwarenessRef;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use axum::extract::ws::{WebSocket, Message};
use yrs::sync::Error;

/// Connection Wrapper over a [WebSocket], which implements a Yjs/Yrs awareness and update exchange
/// protocol.
///
/// This connection implements Future pattern and can be awaited upon in order for a caller to
/// recognize whether underlying websocket connection has been finished gracefully or abruptly.
#[repr(transparent)]
#[derive(Debug)]
pub struct AxumConn(Connection<AxumSink, AxumStream>);

impl AxumConn {
    pub fn new(awareness: AwarenessRef, socket: WebSocket) -> Self {
        let (sink, stream) = socket.split();
        let conn = Connection::new(awareness, AxumSink(sink), AxumStream(stream));
        AxumConn(conn)
    }
}

impl core::future::Future for AxumConn {
    type Output = Result<(), Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.0).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(Error::Other(e.into()))),
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
        }
    }
}

/// An axum websocket sink wrapper, that implements futures `Sink` in a way, that makes it compatible
/// with y-sync protocol, so that it can be used by y-sync crate [BroadcastGroup].
///
/// # Examples
///
/// ```rust
/// use std::net::SocketAddr;
/// use std::str::FromStr;
/// use std::sync::Arc;
/// use tokio::sync::Mutex;
/// use tokio::task::JoinHandle;
/// use futures_util::stream::StreamExt;
/// use axum::{
///     Router,
///     routing::get,
///     extract::ws::{WebSocket, WebSocketUpgrade},
///     extract::State,
///     response::IntoResponse,
/// };
/// use yrs_axum::broadcast::BroadcastGroup;
/// use yrs_axum::ws::{AxumSink, AxumStream};
///
/// async fn start_server(
///     addr: &str,
///     bcast: Arc<BroadcastGroup>,
/// ) -> Result<JoinHandle<()>, Box<dyn std::error::Error>> {
///     let addr = SocketAddr::from_str(addr)?;
///     let listener = tokio::net::TcpListener::bind(addr).await?;
///     
///     let app = Router::new()
///         .route("/my-room", get(ws_handler))
///         .with_state(bcast);
///
///     Ok(tokio::spawn(async move {
///         axum::serve(listener, app.into_make_service())
///             .await
///             .unwrap();
///     }))
/// }
///
/// async fn ws_handler(
///     ws: WebSocketUpgrade,
///     State(bcast): State<Arc<BroadcastGroup>>,
/// ) -> impl IntoResponse {
///     ws.on_upgrade(move |socket| peer(socket, bcast))
/// }
///
/// async fn peer(ws: WebSocket, bcast: Arc<BroadcastGroup>) {
///     let (sink, stream) = ws.split();
///     // convert axum web socket into compatible sink/stream
///     let sink = Arc::new(Mutex::new(AxumSink(sink)));
///     let stream = AxumStream(stream);
///     // subscribe to broadcast group
///     let sub = bcast.subscribe(sink, stream);
///     // wait for subscribed connection to close itself
///     match sub.completed().await {
///         Ok(_) => println!("broadcasting for channel finished successfully"),
///         Err(e) => eprintln!("broadcasting for channel finished abruptly: {}", e),
///     }
/// }
/// ```
#[repr(transparent)]
#[derive(Debug)]
pub struct AxumSink(pub SplitSink<WebSocket, Message>);

impl From<SplitSink<WebSocket, Message>> for AxumSink {
    fn from(sink: SplitSink<WebSocket, Message>) -> Self {
        AxumSink(sink)
    }
}

impl From<AxumSink> for SplitSink<WebSocket, Message> {
    fn from(val: AxumSink) -> Self {
        val.0
    }
}

impl futures_util::Sink<Vec<u8>> for AxumSink {
    type Error = Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match Pin::new(&mut self.0).poll_ready(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(Error::Other(e.into()))),
            Poll::Ready(_) => Poll::Ready(Ok(())),
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        if let Err(e) = Pin::new(&mut self.0).start_send(Message::binary(item)) {
            Err(Error::Other(e.into()))
        } else {
            Ok(())
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match Pin::new(&mut self.0).poll_flush(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(Error::Other(e.into()))),
            Poll::Ready(_) => Poll::Ready(Ok(())),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match Pin::new(&mut self.0).poll_close(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(Error::Other(e.into()))),
            Poll::Ready(_) => Poll::Ready(Ok(())),
        }
    }
}

/// An axum websocket stream wrapper, that implements futures `Stream` in a way, that makes it compatible
/// with y-sync protocol, so that it can be used by y-sync crate [BroadcastGroup].
///
/// # Examples
///
/// ```rust
/// use std::net::SocketAddr;
/// use std::str::FromStr;
/// use std::sync::Arc;
/// use tokio::sync::Mutex;
/// use tokio::task::JoinHandle;
/// use futures_util::stream::StreamExt;
/// use axum::{
///     Router,
///     routing::get,
///     extract::ws::{WebSocket, WebSocketUpgrade},
///     extract::State,
///     response::IntoResponse,
/// };
/// use yrs_axum::broadcast::BroadcastGroup;
/// use yrs_axum::ws::{AxumSink, AxumStream};
///
/// async fn start_server(
///     addr: &str,
///     bcast: Arc<BroadcastGroup>,
/// ) -> Result<JoinHandle<()>, Box<dyn std::error::Error>> {
///     let addr = SocketAddr::from_str(addr)?;
///     let listener = tokio::net::TcpListener::bind(addr).await?;
///     
///     let app = Router::new()
///         .route("/my-room", get(ws_handler))
///         .with_state(bcast);
///
///     Ok(tokio::spawn(async move {
///         axum::serve(listener, app.into_make_service())
///             .await
///             .unwrap();
///     }))
/// }
///
/// async fn ws_handler(
///     ws: WebSocketUpgrade,
///     State(bcast): State<Arc<BroadcastGroup>>,
/// ) -> impl IntoResponse {
///     ws.on_upgrade(move |socket| peer(socket, bcast))
/// }
///
/// async fn peer(ws: WebSocket, bcast: Arc<BroadcastGroup>) {
///     let (sink, stream) = ws.split();
///     // convert axum web socket into compatible sink/stream
///     let sink = Arc::new(Mutex::new(AxumSink(sink)));
///     let stream = AxumStream(stream);
///     // subscribe to broadcast group
///     let sub = bcast.subscribe(sink, stream);
///     // wait for subscribed connection to close itself
///     match sub.completed().await {
///         Ok(_) => println!("broadcasting for channel finished successfully"),
///         Err(e) => eprintln!("broadcasting for channel finished abruptly: {}", e),
///     }
/// }
/// ```
#[derive(Debug)]
pub struct AxumStream(pub SplitStream<WebSocket>);

impl From<SplitStream<WebSocket>> for AxumStream {
    fn from(stream: SplitStream<WebSocket>) -> Self {
        AxumStream(stream)
    }
}

impl From<AxumStream> for SplitStream<WebSocket> {
    fn from(val: AxumStream) -> Self {
        val.0
    }
}

impl Stream for AxumStream {
    type Item = Result<Vec<u8>, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.0).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Ready(Some(res)) => match res {
                Ok(item) => Poll::Ready(Some(Ok(item.into_data().to_vec()))),
                Err(e) => Poll::Ready(Some(Err(Error::Other(e.into())))),
            },
        }
    }
}






#[cfg(test)]
pub mod test {
    use crate::broadcast::BroadcastGroup;
    use crate::client::{client, create_notifier, start_server, TIMEOUT};
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
   

    #[tokio::test]
    async fn change_introduced_by_server_reaches_subscribed_clients() {
        let doc = Doc::with_client_id(1);
        let text = doc.get_or_insert_text("test");
        let awareness = Arc::new(RwLock::new(Awareness::new(doc)));
        let bcast = BroadcastGroup::new(awareness.clone(), 10).await;
        let _server = start_server("0.0.0.0:16600", Arc::new(bcast)).await.unwrap();

        let doc = Doc::new();
        let (n, _sub) = create_notifier(&doc);
        let c1 = client("ws://localhost:16600/my-room", doc).await.unwrap();

        {
            let lock = awareness.write().await;
            text.push(&mut lock.doc().transact_mut(), "abc");
        }

        timeout(TIMEOUT, n.notified()).await.unwrap();

        {
            let awareness = c1.awareness().read().await;
            let doc = awareness.doc();
            let text = doc.get_or_insert_text("test");
            let str = text.get_string(&doc.transact());
            assert_eq!(str, "abc".to_string());
        }
    }

    #[tokio::test]
    async fn subscribed_client_fetches_initial_state() {
        let doc = Doc::with_client_id(1);
        let text = doc.get_or_insert_text("test");

        text.push(&mut doc.transact_mut(), "abc");

        let awareness = Arc::new(RwLock::new(Awareness::new(doc)));
        let bcast = BroadcastGroup::new(awareness.clone(), 10).await;
        let _server = start_server("0.0.0.0:16601", Arc::new(bcast)).await.unwrap();

        let doc = Doc::new();
        let (n, _sub) = create_notifier(&doc);
        let c1 = client("ws://localhost:16601/my-room", doc).await.unwrap();

        timeout(TIMEOUT, n.notified()).await.unwrap();

        {
            let awareness = c1.awareness().read().await;
            let doc = awareness.doc();
            let text = doc.get_or_insert_text("test");
            let str = text.get_string(&doc.transact());
            assert_eq!(str, "abc".to_string());
        }
    }

    #[tokio::test]
    async fn changes_from_one_client_reach_others() {
        let doc = Doc::with_client_id(1);
        let _ = doc.get_or_insert_text("test");

        let awareness = Arc::new(RwLock::new(Awareness::new(doc)));
        let bcast = BroadcastGroup::new(awareness.clone(), 10).await;
        let _server = start_server("0.0.0.0:16602", Arc::new(bcast)).await.unwrap();

        let d1 = Doc::with_client_id(2);
        let c1 = client("ws://localhost:16602/my-room", d1).await.unwrap();
        // by default changes made by document on the client side are not propagated automatically
        let _sub11 = {
            let sink = c1.sink();
            let a = c1.awareness().write().await;
            let doc = a.doc();
            doc.observe_update_v1(move |_, e| {
                let update = e.update.to_owned();
                if let Some(sink) = sink.upgrade() {
                    task::spawn(async move {
                        let msg = yrs::sync::Message::Sync(yrs::sync::SyncMessage::Update(update))
                            .encode_v1();
                        let mut sink = sink.lock().await;
                        sink.send(msg).await.unwrap();
                    });
                }
            })
            .unwrap()
        };

        let d2 = Doc::with_client_id(3);
        let (n2, _sub2) = create_notifier(&d2);
        let c2 = client("ws://localhost:16602/my-room", d2).await.unwrap();

        {
            let a = c1.awareness().write().await;
            let doc = a.doc();
            let text = doc.get_or_insert_text("test");
            text.push(&mut doc.transact_mut(), "def");
        }

        timeout(TIMEOUT, n2.notified()).await.unwrap();

        {
            let awareness = c2.awareness().read().await;
            let doc = awareness.doc();
            let text = doc.get_or_insert_text("test");
            let str = text.get_string(&doc.transact());
            assert_eq!(str, "def".to_string());
        }
    }

    #[tokio::test]
    async fn client_failure_doesnt_affect_others() {
        let doc = Doc::with_client_id(1);
        let _text = doc.get_or_insert_text("test");

        let awareness = Arc::new(RwLock::new(Awareness::new(doc)));
        let bcast = BroadcastGroup::new(awareness.clone(), 10).await;
        let _server = start_server("0.0.0.0:16603", Arc::new(bcast)).await.unwrap();

        let d1 = Doc::with_client_id(2);
        let c1 = client("ws://localhost:16603/my-room", d1).await.unwrap();
        // by default changes made by document on the client side are not propagated automatically
        let _sub11 = {
            let sink = c1.sink();
            let a = c1.awareness().write().await;
            let doc = a.doc();
            doc.observe_update_v1(move |_, e| {
                let update = e.update.to_owned();
                if let Some(sink) = sink.upgrade() {
                    task::spawn(async move {
                        let msg = yrs::sync::Message::Sync(yrs::sync::SyncMessage::Update(update))
                            .encode_v1();
                        let mut sink = sink.lock().await;
                        sink.send(msg).await.unwrap();
                    });
                }
            })
            .unwrap()
        };

        let d2 = Doc::with_client_id(3);
        let (n2, sub2) = create_notifier(&d2);
        let c2 = client("ws://localhost:16603/my-room", d2).await.unwrap();

        let d3 = Doc::with_client_id(4);
        let (n3, sub3) = create_notifier(&d3);
        let c3 = client("ws://localhost:16603/my-room", d3).await.unwrap();

        {
            let a = c1.awareness().write().await;
            let doc = a.doc();
            let text = doc.get_or_insert_text("test");
            text.push(&mut doc.transact_mut(), "abc");
        }

        // on the first try both C2 and C3 should receive the update
        //timeout(TIMEOUT, n2.notified()).await.unwrap();
        //timeout(TIMEOUT, n3.notified()).await.unwrap();
        sleep(TIMEOUT).await;

        {
            let awareness = c2.awareness().read().await;
            let doc = awareness.doc();
            let text = doc.get_or_insert_text("test");
            let str = text.get_string(&doc.transact());
            assert_eq!(str, "abc".to_string());
        }
        {
            let awareness = c3.awareness().read().await;
            let doc = awareness.doc();
            let text = doc.get_or_insert_text("test");
            let str = text.get_string(&doc.transact());
            assert_eq!(str, "abc".to_string());
        }

        // drop client, causing abrupt ending
        drop(c3);
        drop(n3);
        drop(sub3);
        // C2 notification subscription has been realized, we need to refresh it
        drop(n2);
        drop(sub2);

        let (n2, _sub2) = {
            let a = c2.awareness().write().await;
            let doc = a.doc();
            create_notifier(doc)
        };

        {
            let a = c1.awareness().write().await;
            let doc = a.doc();
            let text = doc.get_or_insert_text("test");
            text.push(&mut doc.transact_mut(), "def");
        }

        timeout(TIMEOUT, n2.notified()).await.unwrap();

        {
            let awareness = c2.awareness().read().await;
            let doc = awareness.doc();
            let text = doc.get_or_insert_text("test");
            let str = text.get_string(&doc.transact());
            assert_eq!(str, "abcdef".to_string());
        }
    }
}
