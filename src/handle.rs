use crate::notify_once::NotifyOnce;
use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{sync::Notify, time::sleep};

/// A handle to manage and interact with the server.
///
/// `Handle` provides methods to access server information, such as the number of active connections,
/// and to perform actions like initiating a shutdown.
#[derive(Clone, Debug, Default)]
pub struct Handle {
    inner: Arc<HandleInner>,
}

#[derive(Debug, Default)]
struct HandleInner {
    addr: Mutex<Option<SocketAddr>>,
    addr_notify: Notify,
    conn_count: AtomicUsize,
    shutdown: NotifyOnce,
    graceful: NotifyOnce,
    graceful_dur: Mutex<Option<Duration>>,
    conn_end: NotifyOnce,
}

impl Handle {
    /// Create a new handle for the server.
    ///
    /// # Returns
    ///
    /// A new `Handle` instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the number of active connections to the server.
    ///
    /// # Returns
    ///
    /// The number of active connections.
    pub fn connection_count(&self) -> usize {
        self.inner.conn_count.load(Ordering::SeqCst)
    }

    /// Initiate an immediate shutdown of the server.
    ///
    /// This method will terminate the server without waiting for active connections to close.
    pub fn shutdown(&self) {
        self.inner.shutdown.notify_waiters();
    }

    /// Initiate a graceful shutdown of the server.
    ///
    /// The server will wait for active connections to close before shutting down. If a duration
    /// is provided, the server will wait up to that duration for active connections to close
    /// before forcing a shutdown.
    ///
    /// # Parameters
    ///
    /// - `duration`: Maximum time to wait for active connections to close. `None` means the server
    /// will wait indefinitely.
    pub fn graceful_shutdown(&self, duration: Option<Duration>) {
        *self.inner.graceful_dur.lock().unwrap() = duration;
        self.inner.graceful.notify_waiters();
    }

    /// Wait until the server starts listening and then returns its local address and port.
    ///
    /// # Returns
    ///
    /// The local `SocketAddr` if the server successfully binds, otherwise `None`.
    pub async fn listening(&self) -> Option<SocketAddr> {
        let notified = self.inner.addr_notify.notified();

        if let Some(addr) = *self.inner.addr.lock().unwrap() {
            return Some(addr);
        }

        notified.await;

        *self.inner.addr.lock().unwrap()
    }

    /// Internal method to notify the handle when the server starts listening on a particular address.
    pub(crate) fn notify_listening(&self, addr: Option<SocketAddr>) {
        *self.inner.addr.lock().unwrap() = addr;
        self.inner.addr_notify.notify_waiters();
    }

    /// Creates a watcher that monitors server status and connection activity.
    pub(crate) fn watcher(&self) -> Watcher {
        Watcher::new(self.clone())
    }

    /// Internal method to wait until the server is shut down.
    pub(crate) async fn wait_shutdown(&self) {
        self.inner.shutdown.notified().await;
    }

    /// Internal method to wait until the server is gracefully shut down.
    pub(crate) async fn wait_graceful_shutdown(&self) {
        self.inner.graceful.notified().await;
    }

    /// Internal method to wait until all connections have ended, or the optional graceful duration has expired.
    pub(crate) async fn wait_connections_end(&self) {
        if self.inner.conn_count.load(Ordering::SeqCst) == 0 {
            return;
        }

        let deadline = *self.inner.graceful_dur.lock().unwrap();

        match deadline {
            Some(duration) => tokio::select! {
                biased;
                _ = sleep(duration) => self.shutdown(),
                _ = self.inner.conn_end.notified() => (),
            },
            None => self.inner.conn_end.notified().await,
        }
    }
}

/// A watcher that monitors server status and connection activity.
///
/// The watcher keeps track of active connections and listens for shutdown or graceful shutdown signals.
pub(crate) struct Watcher {
    handle: Handle,
}

impl Watcher {
    /// Creates a new watcher linked to the given server handle.
    fn new(handle: Handle) -> Self {
        handle.inner.conn_count.fetch_add(1, Ordering::SeqCst);
        Self { handle }
    }

    /// Internal method to wait until the server is gracefully shut down.
    pub(crate) async fn wait_graceful_shutdown(&self) {
        self.handle.wait_graceful_shutdown().await
    }

    /// Internal method to wait until the server is shut down.
    pub(crate) async fn wait_shutdown(&self) {
        self.handle.wait_shutdown().await
    }
}

impl Drop for Watcher {
    /// Reduces the active connection count when a watcher is dropped.
    ///
    /// If the connection count reaches zero and a graceful shutdown has been initiated, the server is notified that
    /// all connections have ended.
    fn drop(&mut self) {
        let count = self.handle.inner.conn_count.fetch_sub(1, Ordering::SeqCst) - 1;

        if count == 0 && self.handle.inner.graceful.is_notified() {
            self.handle.inner.conn_end.notify_waiters();
        }
    }
}
