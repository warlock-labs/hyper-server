#[cfg(feature = "proxy-protocol")]
use crate::proxy_protocol::ProxyProtocolAcceptor;
use crate::{
    accept::{Accept, DefaultAcceptor},
    addr_incoming_config::AddrIncomingConfig,
    handle::Handle,
    http_config::HttpConfig,
    service::{MakeServiceRef, SendService},
};
use futures_util::future::poll_fn;
use http::Request;
use hyper::server::{
    accept::Accept as HyperAccept,
    conn::{AddrIncoming, AddrStream},
};
#[cfg(feature = "proxy-protocol")]
use std::time::Duration;
use std::{
    io::{self, ErrorKind},
    net::SocketAddr,
    pin::Pin,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpListener,
};

/// Represents an HTTP server with customization capabilities for handling incoming requests.
#[derive(Debug)]
pub struct Server<A = DefaultAcceptor> {
    acceptor: A,
    listener: Listener,
    addr_incoming_conf: AddrIncomingConfig,
    handle: Handle,
    http_conf: HttpConfig,
    #[cfg(feature = "proxy-protocol")]
    proxy_acceptor_set: bool,
}

/// Enum representing the ways the server can be initialized - either by binding to an address or from a standard TCP listener.
#[derive(Debug)]
enum Listener {
    Bind(SocketAddr),
    Std(std::net::TcpListener),
}

/// Creates a new [`Server`] instance that binds to the provided address.
pub fn bind(addr: SocketAddr) -> Server {
    Server::bind(addr)
}

/// Creates a new [`Server`] instance using an existing `std::net::TcpListener`.
pub fn from_tcp(listener: std::net::TcpListener) -> Server {
    Server::from_tcp(listener)
}

impl Server {
    /// Constructs a server bound to the provided address.
    pub fn bind(addr: SocketAddr) -> Self {
        let acceptor = DefaultAcceptor::new();
        let handle = Handle::new();

        Self {
            acceptor,
            listener: Listener::Bind(addr),
            addr_incoming_conf: AddrIncomingConfig::default(),
            handle,
            http_conf: HttpConfig::default(),
            #[cfg(feature = "proxy-protocol")]
            proxy_acceptor_set: false,
        }
    }

    /// Constructs a server from an existing `std::net::TcpListener`.
    pub fn from_tcp(listener: std::net::TcpListener) -> Self {
        let acceptor = DefaultAcceptor::new();
        let handle = Handle::new();

        Self {
            acceptor,
            listener: Listener::Std(listener),
            addr_incoming_conf: AddrIncomingConfig::default(),
            handle,
            http_conf: HttpConfig::default(),
            #[cfg(feature = "proxy-protocol")]
            proxy_acceptor_set: false,
        }
    }
}

impl<A> Server<A> {
    /// Replace the current acceptor with a new one.
    pub fn acceptor<Acceptor>(self, acceptor: Acceptor) -> Server<Acceptor> {
        #[cfg(feature = "proxy-protocol")]
        if self.proxy_acceptor_set {
            panic!("Overwriting the acceptor after proxy protocol is enabled is not supported. Configure the acceptor first in the builder, then enable proxy protocol.");
        }

        Server {
            acceptor,
            listener: self.listener,
            addr_incoming_conf: self.addr_incoming_conf,
            handle: self.handle,
            http_conf: self.http_conf,
            #[cfg(feature = "proxy-protocol")]
            proxy_acceptor_set: self.proxy_acceptor_set,
        }
    }

    #[cfg(feature = "proxy-protocol")]
    /// Enable proxy protocol header parsing.
    /// Note has to be called after initial acceptor is set.
    pub fn enable_proxy_protocol(
        self,
        parsing_timeout: Option<Duration>,
    ) -> Server<ProxyProtocolAcceptor<A>> {
        let initial_acceptor = self.acceptor;
        let mut acceptor = ProxyProtocolAcceptor::new(initial_acceptor);

        if let Some(val) = parsing_timeout {
            acceptor = acceptor.parsing_timeout(val);
        }

        Server {
            acceptor,
            listener: self.listener,
            addr_incoming_conf: self.addr_incoming_conf,
            handle: self.handle,
            http_conf: self.http_conf,
            proxy_acceptor_set: true,
        }
    }

    /// Maps the current acceptor to a new type.
    pub fn map<Acceptor, F>(self, acceptor: F) -> Server<Acceptor>
    where
        F: FnOnce(A) -> Acceptor,
    {
        Server {
            acceptor: acceptor(self.acceptor),
            listener: self.listener,
            addr_incoming_conf: self.addr_incoming_conf,
            handle: self.handle,
            http_conf: self.http_conf,
            #[cfg(feature = "proxy-protocol")]
            proxy_acceptor_set: self.proxy_acceptor_set,
        }
    }

    /// Retrieves a reference to the server's acceptor.
    pub fn get_ref(&self) -> &A {
        &self.acceptor
    }

    /// Retrieves a mutable reference to the server's acceptor.
    pub fn get_mut(&mut self) -> &mut A {
        &mut self.acceptor
    }

    /// Provides the server with a handle for extra utilities.
    pub fn handle(mut self, handle: Handle) -> Self {
        self.handle = handle;
        self
    }

    /// Replaces the current HTTP configuration.
    pub fn http_config(mut self, config: HttpConfig) -> Self {
        self.http_conf = config;
        self
    }

    /// Replaces the current incoming address configuration.
    pub fn addr_incoming_config(mut self, config: AddrIncomingConfig) -> Self {
        self.addr_incoming_conf = config;
        self
    }

    /// Serves the provided `MakeService`.
    ///
    /// The `MakeService` is responsible for constructing services for each incoming connection.
    /// Each service is then used to handle requests from that specific connection.
    ///
    /// # Arguments
    /// - `make_service`: A mutable reference to a type implementing the `MakeServiceRef` trait.
    ///   This will be used to produce a service for each incoming connection.
    ///
    /// # Errors
    ///
    /// This method can return errors in the following scenarios:
    /// - When binding to an address fails.
    /// - If the `make_service` function encounters an error during its `poll_ready` call.
    ///   It's worth noting that this error scenario doesn't typically occur with `axum` make services.
    ///
    pub async fn serve<M>(self, mut make_service: M) -> io::Result<()>
    where
        M: MakeServiceRef<AddrStream, Request<hyper::Body>>,
        A: Accept<AddrStream, M::Service> + Clone + Send + Sync + 'static,
        A::Stream: AsyncRead + AsyncWrite + Unpin + Send,
        A::Service: SendService<Request<hyper::Body>> + Send,
        A::Future: Send,
    {
        // Extract relevant fields from `self` for easier access.
        let acceptor = self.acceptor;
        let addr_incoming_conf = self.addr_incoming_conf;
        let handle = self.handle;
        let http_conf = self.http_conf;

        // Bind the incoming connections. Notify the handle if an error occurs during binding.
        let mut incoming = match bind_incoming(self.listener, addr_incoming_conf).await {
            Ok(v) => v,
            Err(e) => {
                handle.notify_listening(None);
                return Err(e);
            }
        };

        // Notify the handle about the server's listening state.
        handle.notify_listening(Some(incoming.local_addr()));

        // This is the main loop that accepts incoming connections and spawns tasks to handle them.
        let accept_loop_future = async {
            loop {
                // Wait for a new connection or for the server to be signaled to shut down.
                let addr_stream = tokio::select! {
                    biased;
                    result = accept(&mut incoming) => result?,
                    _ = handle.wait_graceful_shutdown() => return Ok(()),
                };

                // Ensure the `make_service` is ready to produce another service.
                poll_fn(|cx| make_service.poll_ready(cx))
                    .await
                    .map_err(io_other)?;

                // Create a service for this connection.
                let service = match make_service.make_service(&addr_stream).await {
                    Ok(service) => service,
                    Err(_) => continue, // TODO: Consider logging or handling this error in a more detailed manner.
                };

                // Clone necessary objects for the spawned task.
                let acceptor = acceptor.clone();
                let watcher = handle.watcher();
                let http_conf = http_conf.clone();

                // Spawn a new task to handle the connection.
                tokio::spawn(async move {
                    if let Ok((stream, send_service)) = acceptor.accept(addr_stream, service).await
                    {
                        let service = send_service.into_service();

                        let mut serve_future = http_conf
                            .inner
                            .serve_connection(stream, service)
                            .with_upgrades();

                        // Wait for either the server to be shut down or the connection to finish.
                        tokio::select! {
                            biased;
                            _ = watcher.wait_graceful_shutdown() => {
                                // Initiate a graceful shutdown.
                                Pin::new(&mut serve_future).graceful_shutdown();
                                tokio::select! {
                                    biased;
                                    _ = watcher.wait_shutdown() => (),
                                    _ = &mut serve_future => (),
                                }
                            }
                            _ = watcher.wait_shutdown() => (),
                            _ = &mut serve_future => (),
                        }
                    }
                    // TODO: Consider logging or handling any errors that occur during acceptance.
                });
            }
        };

        // Wait for either the server to be fully shut down or an error to occur.
        let result = tokio::select! {
            biased;
            _ = handle.wait_shutdown() => return Ok(()),
            result = accept_loop_future => result,
        };

        // Handle potential errors.
        // TODO: Consider removing the Clippy annotation by restructuring this error handling.
        #[allow(clippy::question_mark)]
        if let Err(e) = result {
            return Err(e);
        }

        // Wait for all connections to end.
        handle.wait_connections_end().await;

        Ok(())
    }
}

/// Binds the listener based on the provided configuration and returns an [`AddrIncoming`]
/// which will produce [`AddrStream`]s for incoming connections.
///
/// The function takes into account different ways the listener might be set up,
/// either by binding to a provided address or by using an existing standard listener.
///
/// # Arguments
///
/// - `listener`: The listener configuration. Can be either a direct bind address or an existing standard listener.
/// - `addr_incoming_conf`: Configuration for the incoming connections, such as TCP keepalive settings.
///
/// # Errors
///
/// Returns an `io::Error` if:
/// - Binding the listener fails.
/// - Setting the listener to non-blocking mode fails.
/// - The listener cannot be converted to a [`TcpListener`].
/// - An error occurs when creating the [`AddrIncoming`].
///
async fn bind_incoming(
    listener: Listener,
    addr_incoming_conf: AddrIncomingConfig,
) -> io::Result<AddrIncoming> {
    let listener = match listener {
        Listener::Bind(addr) => TcpListener::bind(addr).await?,
        Listener::Std(std_listener) => {
            std_listener.set_nonblocking(true)?;
            TcpListener::from_std(std_listener)?
        }
    };
    let mut incoming = AddrIncoming::from_listener(listener).map_err(io_other)?;

    // Apply configuration settings to the incoming connection handler.
    incoming.set_sleep_on_errors(addr_incoming_conf.tcp_sleep_on_accept_errors);
    incoming.set_keepalive(addr_incoming_conf.tcp_keepalive);
    incoming.set_keepalive_interval(addr_incoming_conf.tcp_keepalive_interval);
    incoming.set_keepalive_retries(addr_incoming_conf.tcp_keepalive_retries);
    incoming.set_nodelay(addr_incoming_conf.tcp_nodelay);

    Ok(incoming)
}

/// Awaits and accepts a new incoming connection.
///
/// This function will poll the given `incoming` object until a new connection is ready to be accepted.
///
/// # Arguments
///
/// - `incoming`: The incoming connection handler from which new connections will be accepted.
///
/// # Returns
///
/// Returns the accepted [`AddrStream`] which represents a specific incoming connection.
///
/// # Panics
///
/// This function will panic if the `poll_accept` method returns `None`, which should never happen as per the Hyper documentation.
///
pub(crate) async fn accept(incoming: &mut AddrIncoming) -> io::Result<AddrStream> {
    let mut incoming = Pin::new(incoming);

    // Always [`Option::Some`].
    // According to: https://docs.rs/hyper/0.14.14/src/hyper/server/tcp.rs.html#165
    poll_fn(|cx| incoming.as_mut().poll_accept(cx))
        .await
        .unwrap()
}

/// Type definition for a boxed error which can be sent between threads and is Sync.
type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Converts any error into an `io::Error` of kind `Other`.
///
/// This function can be used to create a uniform `io::Error` response for various error types.
///
/// # Arguments
///
/// - `error`: The error to be converted.
///
/// # Returns
///
/// Returns an `io::Error` with the kind set to `Other` and the provided error as its cause.
///
pub(crate) fn io_other<E: Into<BoxError>>(error: E) -> io::Error {
    io::Error::new(ErrorKind::Other, error)
}

#[cfg(test)]
mod tests {
    use crate::{handle::Handle, server::Server};
    use axum::{routing::get, Router};
    use bytes::Bytes;
    use http::{response, Request};
    use hyper::{
        client::conn::{handshake, SendRequest},
        Body,
    };
    use std::{io, net::SocketAddr, time::Duration};
    use tokio::{net::TcpStream, task::JoinHandle, time::timeout};
    use tower::{Service, ServiceExt};

    #[tokio::test]
    async fn start_and_request() {
        let (_handle, _server_task, addr) = start_server().await;

        let (mut client, _conn) = connect(addr).await;

        let (_parts, body) = send_empty_request(&mut client).await;

        assert_eq!(body.as_ref(), b"Hello, world!");
    }

    #[tokio::test]
    async fn test_shutdown() {
        let (handle, _server_task, addr) = start_server().await;

        let (mut client, conn) = connect(addr).await;

        handle.shutdown();

        let response_future_result = client
            .ready()
            .await
            .unwrap()
            .call(Request::new(Body::empty()))
            .await;

        assert!(response_future_result.is_err());

        // Connection task should finish soon.
        let _ = timeout(Duration::from_secs(1), conn).await.unwrap();
    }

    // #[tokio::test]
    async fn test_graceful_shutdown() {
        let (handle, server_task, addr) = start_server().await;

        let (mut client, conn) = connect(addr).await;

        handle.graceful_shutdown(None);

        let (_parts, body) = send_empty_request(&mut client).await;

        assert_eq!(body.as_ref(), b"Hello, world!");

        // Disconnect client.
        conn.abort();

        // TODO(This does not shut down gracefully)
        // Server task should finish soon.
        let server_result = timeout(Duration::from_secs(1), server_task)
            .await
            .unwrap()
            .unwrap();

        assert!(server_result.is_ok());
    }

    // #[tokio::test]
    async fn test_graceful_shutdown_timed() {
        let (handle, server_task, addr) = start_server().await;

        let (mut client, _conn) = connect(addr).await;

        handle.graceful_shutdown(Some(Duration::from_millis(250)));

        let (_parts, body) = send_empty_request(&mut client).await;

        assert_eq!(body.as_ref(), b"Hello, world!");

        // Server task should finish soon.
        let server_result = timeout(Duration::from_secs(1), server_task)
            .await
            .unwrap()
            .unwrap();

        assert!(server_result.is_ok());
    }

    async fn start_server() -> (Handle, JoinHandle<io::Result<()>>, SocketAddr) {
        let handle = Handle::new();

        let server_handle = handle.clone();
        let server_task = tokio::spawn(async move {
            let app = Router::new().route("/", get(|| async { "Hello, world!" }));

            let addr = SocketAddr::from(([127, 0, 0, 1], 0));

            Server::bind(addr)
                .handle(server_handle)
                .serve(app.into_make_service())
                .await
        });

        let addr = handle.listening().await.unwrap();

        (handle, server_task, addr)
    }

    async fn connect(addr: SocketAddr) -> (SendRequest<Body>, JoinHandle<()>) {
        let stream = TcpStream::connect(addr).await.unwrap();

        let (send_request, connection) = handshake(stream).await.unwrap();

        let task = tokio::spawn(async move {
            let _ = connection.await;
        });

        (send_request, task)
    }

    async fn send_empty_request(client: &mut SendRequest<Body>) -> (response::Parts, Bytes) {
        let (parts, body) = client
            .ready()
            .await
            .unwrap()
            .call(Request::new(Body::empty()))
            .await
            .unwrap()
            .into_parts();
        let body = hyper::body::to_bytes(body).await.unwrap();

        (parts, body)
    }
}
