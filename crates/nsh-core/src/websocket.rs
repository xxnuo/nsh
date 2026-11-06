//! WebSockets from Rust and the host transport shared with both JavaScript APIs. Rust checks the
//! URL, capabilities and quotas before asking the platform to connect.
//! Each task belongs to its calling engine; dropping it cancels the socket
//! and prevents further events. Browser implementations use the page's
//! WebSocket API without changing the app's JavaScript classes. The
//! `websocket` module pauses a task while the app's unread messages fill
//! their queue ([`WebSocketTask::pause`]) and resumes it as the app reads.

use std::{cell::RefCell, rc::Rc};

use crate::{
    element::{JsError, cleanup, current_system},
    runtime::websocket::{Data, WebSocket as RuntimeWebSocket, WebSocketConnection},
};

/// A text or binary WebSocket message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebSocketData {
    /// UTF-8 text.
    Text(String),
    /// Binary bytes.
    Bytes(Vec<u8>),
}

impl WebSocketData {
    /// The message's byte length.
    pub fn len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Bytes(bytes) => bytes.len(),
        }
    }

    /// Whether the message has no bytes.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A handshake already checked against the app's capabilities.
#[derive(Clone, Debug)]
pub struct WebSocketRequest {
    /// A normalized absolute ws/wss URL.
    pub url: String,
    /// Requested subprotocols, in preference order.
    pub protocols: Vec<String>,
    /// Additional validated headers; browsers only support an empty list.
    pub headers: Vec<(String, String)>,
}

/// An event from one host socket.
#[derive(Clone, Debug)]
pub enum WebSocketEvent {
    /// The handshake succeeded.
    Open {
        /// The negotiated subprotocol, or an empty string.
        protocol: String,
    },
    /// A complete text or binary message.
    Message(WebSocketData),
    /// A transport failure. A close event may follow it.
    Error(String),
    /// The connection ended.
    Close {
        /// The peer's close code, or 1006 for an abnormal end.
        code: u16,
        /// The peer's reason.
        reason: String,
        /// Whether the close handshake completed.
        was_clean: bool,
    },
}

/// Engine-local events. A transport delivers them on its engine's thread,
/// after `connect` returns, and never while a task method is being called.
pub type WebSocketEvents = Box<dyn FnMut(WebSocketEvent)>;

/// One host socket. Dropping it cancels I/O and unregisters its events.
pub trait WebSocketTask: 'static {
    /// Queues a complete message; the caller has checked its size.
    fn send(&mut self, data: WebSocketData) -> Result<(), String>;
    /// Begins a close handshake with the given code and reason.
    fn close(&mut self, code: u16, reason: &str) -> Result<(), String>;
    /// Bytes the host still has queued for sending.
    fn buffered_amount(&self) -> usize;
    /// Stops (`true`) or resumes (`false`) delivering messages: the app's
    /// unread messages fill their queue (back pressure). A transport that
    /// can stop reading from the network does, so the peer is held back;
    /// one that cannot keeps the messages until it resumes. Messages already
    /// on their way may still arrive; the caller bounds what it keeps.
    fn pause(&mut self, paused: bool);
}

/// A platform's WebSocket stack. The factory is shareable, while each
/// socket and its callbacks stay on their owning engine's thread.
pub trait WebSocketTransport: Send + Sync + 'static {
    /// Whether additional handshake headers can be supplied.
    fn supports_headers(&self) -> bool {
        true
    }
    /// Starts a handshake. Events arrive after this method returns.
    fn connect(
        &self,
        request: WebSocketRequest,
        events: WebSocketEvents,
    ) -> Result<Box<dyn WebSocketTask>, String>;
}

/// An engine-local WebSocket, using the same permissions, quotas and queues as JavaScript.
///
/// Connect and operate on it from component handlers or [`crate::element::spawn`] tasks.
/// The creating reactive owner cancels its I/O when disposed, including pending operations.
/// Dropping the connection cancels it too. A default connection is closed.
#[derive(Default)]
#[non_exhaustive]
pub struct WebSocket {
    inner: Rc<RefCell<Option<WebSocketConnection>>>,
}

impl WebSocket {
    /// Connects to a ws/wss URL under the current engine's network permissions.
    /// Shares the engine's 32-connection quota with both JavaScript WebSocket APIs.
    /// Native platforms use the runtime's socket; browsers use the host's transport.
    pub async fn connect(url: impl Into<String>) -> Result<Self, JsError> {
        let system = current_system()?;
        let connection = RuntimeWebSocket::connect(url.into(), None, system)?
            .resolve()
            .await?;
        let socket = Self {
            inner: Rc::new(RefCell::new(Some(connection))),
        };
        let weak = Rc::downgrade(&socket.inner);
        cleanup(move || {
            if let Some(inner) = weak.upgrade() {
                abort(&inner);
            }
        });
        Ok(socket)
    }

    /// Sends one text or binary message, up to 8 MiB; pending writes are bounded to 8 MiB.
    pub async fn write(&self, data: WebSocketData) -> Result<(), JsError> {
        let data = match data {
            WebSocketData::Text(text) => Data::Text(text),
            WebSocketData::Bytes(bytes) => Data::Bytes(bytes),
        };
        let work = self
            .inner
            .borrow()
            .as_ref()
            .ok_or_else(closed)?
            .write(data)?;
        work.resolve().await
    }

    /// Reads the next message, or `None` after a clean close and draining queued messages.
    /// Only one read may be outstanding; unread messages apply transport back pressure.
    pub async fn read(&self) -> Result<Option<WebSocketData>, JsError> {
        let work = self.inner.borrow().as_ref().ok_or_else(closed)?.read()?;
        Ok(work.resolve().await?.map(|data| match data {
            Data::Text(text) => WebSocketData::Text(text),
            Data::Bytes(bytes) => WebSocketData::Bytes(bytes),
        }))
    }

    /// Completes the close handshake; `None` selects code 1000 and an empty reason.
    /// Codes must be 1000 or 3000–4999; reasons are limited to 123 UTF-8 bytes.
    pub async fn close(&self, code: Option<u16>, reason: Option<String>) -> Result<(), JsError> {
        let work = self
            .inner
            .borrow()
            .as_ref()
            .ok_or_else(closed)?
            .close(code.map(f64::from), reason)?;
        work.resolve().await
    }

    /// The peer's close code, or `None` while open; 1006 denotes an abnormal close.
    pub fn close_code(&self) -> Option<u16> {
        self.inner
            .borrow()
            .as_ref()
            .and_then(WebSocketConnection::close_code)
    }

    /// The peer's close reason, or `None` while open.
    pub fn close_reason(&self) -> Option<String> {
        self.inner
            .borrow()
            .as_ref()
            .and_then(WebSocketConnection::close_reason)
    }
}

impl Drop for WebSocket {
    fn drop(&mut self) {
        abort(&self.inner);
    }
}

fn abort(inner: &RefCell<Option<WebSocketConnection>>) {
    let connection = inner.borrow_mut().take();
    if let Some(connection) = connection {
        connection.abort();
    }
}

fn closed() -> JsError {
    JsError::Error("WebSocket connection is closed".into()).with_code("ERR_SOCKET_CLOSED")
}

impl std::fmt::Debug for WebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocket")
            .field("close_code", &self.close_code())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        future::Future,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll, Waker},
    };

    use super::*;
    use crate::{
        access::{Access, Capabilities},
        element::{Owner, Runtime, System, codes},
        runtime::{MAX_WEBSOCKETS, WEBSOCKETS},
    };

    thread_local! {
        static EVENTS: RefCell<Option<WebSocketEvents>> = RefCell::new(None);
        static SENT: RefCell<Vec<WebSocketData>> = const { RefCell::new(Vec::new()) };
        static PAUSES: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
        static BUFFERED: Cell<usize> = const { Cell::new(0) };
    }

    #[derive(Default)]
    struct Transport {
        connections: AtomicUsize,
        drops: Arc<AtomicUsize>,
    }
    struct Socket {
        drops: Arc<AtomicUsize>,
    }
    impl Drop for Socket {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl WebSocketTask for Socket {
        fn send(&mut self, data: WebSocketData) -> Result<(), String> {
            SENT.with(|sent| sent.borrow_mut().push(data));
            Ok(())
        }
        fn close(&mut self, _: u16, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn buffered_amount(&self) -> usize {
            BUFFERED.get()
        }
        fn pause(&mut self, paused: bool) {
            PAUSES.with(|pauses| pauses.borrow_mut().push(paused));
        }
    }
    impl WebSocketTransport for Transport {
        fn supports_headers(&self) -> bool {
            false
        }
        fn connect(
            &self,
            _: WebSocketRequest,
            events: WebSocketEvents,
        ) -> Result<Box<dyn WebSocketTask>, String> {
            self.connections.fetch_add(1, Ordering::SeqCst);
            EVENTS.with(|slot| *slot.borrow_mut() = Some(events));
            Ok(Box::new(Socket {
                drops: self.drops.clone(),
            }))
        }
    }
    fn services(transport: Arc<Transport>) -> System {
        SENT.with(|sent| sent.borrow_mut().clear());
        PAUSES.with(|pauses| pauses.borrow_mut().clear());
        BUFFERED.set(0);
        let mut system = System::default().with_websocket(Some(transport));
        system.access = Arc::new(Access::all());
        system
    }
    fn emit(event: WebSocketEvent) {
        EVENTS.with(|slot| slot.borrow_mut().as_mut().unwrap()(event));
    }
    fn poll<F: Future>(future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
        future.poll(&mut Context::from_waker(Waker::noop()))
    }
    fn connect(owner: &Owner) -> WebSocket {
        owner.run(|| {
            let mut opening = std::pin::pin!(WebSocket::connect("ws://example.com"));
            assert!(poll(opening.as_mut()).is_pending());
            emit(WebSocketEvent::Open {
                protocol: String::new(),
            });
            match poll(opening.as_mut()) {
                Poll::Ready(Ok(socket)) => socket,
                other => panic!("connection did not open: {other:?}"),
            }
        })
    }

    #[tokio::test]
    async fn permissions_host_refusals_and_quotas_precede_transport_calls() {
        let runtime = Runtime::default();
        let _entered = runtime.enter();
        let transport = Arc::new(Transport::default());
        let mut system = services(transport.clone());
        system.access = Arc::new(Access {
            declared: Capabilities::default(),
            granted: None,
        });
        runtime.set_system(system.clone());
        assert_eq!(
            WebSocket::connect("ws://example.com")
                .await
                .unwrap_err()
                .code(),
            Some(codes::ACCESS_DENIED)
        );
        system.access = Arc::new(Access::all());
        system.refuse_connections = Some(Arc::new(|_| {
            JsError::Error("test refusal".into()).with_code("ERR_TEST_REFUSED")
        }));
        runtime.set_system(system.clone());
        assert_eq!(
            WebSocket::connect("ws://example.com")
                .await
                .unwrap_err()
                .code(),
            Some("ERR_TEST_REFUSED")
        );
        system.refuse_connections = None;
        let held = system
            .quotas
            .reserve(WEBSOCKETS, MAX_WEBSOCKETS - 1, MAX_WEBSOCKETS)
            .unwrap();
        runtime.set_system(system.clone());
        let owner = Owner::detached();
        let socket = connect(&owner);
        assert_eq!(system.quotas.held(WEBSOCKETS), MAX_WEBSOCKETS);
        let error =
            RuntimeWebSocket::connect("ws://example.com".into(), None, system.clone()).unwrap_err();
        assert_eq!(error.code(), Some(codes::QUOTA_EXCEEDED));
        assert_eq!(transport.connections.load(Ordering::SeqCst), 1);
        owner.dispose();
        assert_eq!(system.quotas.held(WEBSOCKETS), MAX_WEBSOCKETS - 1);
        assert_eq!(
            socket.read().await.unwrap_err().code(),
            Some("ERR_SOCKET_CLOSED")
        );
        drop(held);
        assert_eq!(system.quotas.held(WEBSOCKETS), 0);
    }

    #[tokio::test]
    async fn messages_eof_and_back_pressure_use_the_shared_runtime() {
        let runtime = Runtime::default();
        let _entered = runtime.enter();
        let transport = Arc::new(Transport::default());
        runtime.set_system(services(transport));
        let owner = Owner::detached();
        let socket = connect(&owner);
        socket
            .write(WebSocketData::Text("hello".into()))
            .await
            .unwrap();
        socket
            .write(WebSocketData::Bytes(vec![3, 7]))
            .await
            .unwrap();
        SENT.with(|sent| {
            assert_eq!(
                *sent.borrow(),
                vec![
                    WebSocketData::Text("hello".into()),
                    WebSocketData::Bytes(vec![3, 7])
                ]
            )
        });
        for _ in 0..8 {
            emit(WebSocketEvent::Message(WebSocketData::Bytes(vec![7])));
        }
        PAUSES.with(|pauses| assert_eq!(*pauses.borrow(), vec![true]));
        assert_eq!(
            socket.read().await.unwrap(),
            Some(WebSocketData::Bytes(vec![7]))
        );
        PAUSES.with(|pauses| assert_eq!(*pauses.borrow(), vec![true, false]));
        emit(WebSocketEvent::Close {
            code: 1000,
            reason: "bye".into(),
            was_clean: true,
        });
        for _ in 0..7 {
            assert!(socket.read().await.unwrap().is_some());
        }
        assert_eq!(socket.read().await.unwrap(), None);
        assert_eq!(socket.close_code(), Some(1000));
        assert_eq!(socket.close_reason().as_deref(), Some("bye"));
        owner.dispose();
    }

    #[tokio::test]
    async fn owner_disposal_cancels_retained_reads_writes_and_close() {
        let runtime = Runtime::default();
        let _entered = runtime.enter();
        let transport = Arc::new(Transport::default());
        let system = services(transport.clone());
        runtime.set_system(system.clone());
        let owner = Owner::detached();
        let socket = connect(&owner);
        let mut read = std::pin::pin!(socket.read());
        assert!(poll(read.as_mut()).is_pending());
        assert!(matches!(
            poll(std::pin::pin!(socket.read()).as_mut()),
            Poll::Ready(Err(JsError::Type(_)))
        ));
        BUFFERED.set(1);
        let mut write = std::pin::pin!(socket.write(WebSocketData::Text("pending".into())));
        assert!(poll(write.as_mut()).is_pending());
        let mut close = std::pin::pin!(socket.close(None, None));
        assert!(poll(close.as_mut()).is_pending());
        owner.dispose();
        assert_eq!(transport.drops.load(Ordering::SeqCst), 1);
        assert_eq!(system.quotas.held(WEBSOCKETS), 0);
        assert!(matches!(poll(read.as_mut()), Poll::Ready(Err(_))));
        // A pending browser write checks for errors after its bounded drain interval.
        assert_eq!(write.await.unwrap_err().code(), Some("ERR_SOCKET_CLOSED"));
        assert!(matches!(poll(close.as_mut()), Poll::Ready(Ok(()))));
        assert_eq!(
            socket
                .write(WebSocketData::Bytes(vec![]))
                .await
                .unwrap_err()
                .code(),
            Some("ERR_SOCKET_CLOSED")
        );
    }

    #[tokio::test]
    async fn cancelled_handshakes_release_transport_and_quota() {
        let runtime = Runtime::default();
        let _entered = runtime.enter();
        let executor = Rc::new(crate::element::task::Executor::default());
        runtime.set_services(executor.clone());
        let transport = Arc::new(Transport::default());
        let system = services(transport.clone());
        runtime.set_system(system.clone());
        let owner = Owner::detached();
        owner.run(|| {
            crate::element::spawn(WebSocket::connect("ws://example.com"));
        });
        executor.poll_ready();
        assert_eq!(system.quotas.held(WEBSOCKETS), 1);
        owner.dispose();
        assert_eq!(transport.drops.load(Ordering::SeqCst), 1);
        assert_eq!(system.quotas.held(WEBSOCKETS), 0);
    }

    #[tokio::test]
    async fn missing_services_and_invalid_close_frames_keep_error_codes() {
        assert_eq!(
            WebSocket::connect("ws://example.com")
                .await
                .unwrap_err()
                .code(),
            Some(codes::UNAVAILABLE)
        );
        let runtime = Runtime::default();
        let _entered = runtime.enter();
        let transport = Arc::new(Transport::default());
        runtime.set_system(services(transport));
        let owner = Owner::detached();
        let socket = connect(&owner);
        for (code, reason) in [(Some(1001), None), (Some(1000), Some("é".repeat(62)))] {
            assert_eq!(
                socket.close(code, reason).await.unwrap_err().code(),
                Some("ERR_OUT_OF_RANGE")
            );
        }
        assert_eq!(
            socket
                .write(WebSocketData::Bytes(vec![0; 8 * 1024 * 1024 + 1]))
                .await
                .unwrap_err()
                .code(),
            Some("ERR_OUT_OF_RANGE")
        );
        owner.dispose();
    }
}
