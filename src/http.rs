use crate::io::Transport;
use std::future::pending;
use std::{future::Future, pin::pin, sync::Arc, time::Duration};
use tokio_rustls::TlsAcceptor;

use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;
use hyper::body::Incoming;
use hyper::service::Service;
use hyper_util::{
    rt::TokioIo,
    server::conn::auto::{Builder as HttpConnectionBuilder, HttpServerConnExec},
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::sleep;
use tokio_stream::Stream;
use tokio_stream::StreamExt as _;
use tracing::{debug, trace};

use crate::fuse::Fuse;

/// Sleeps for a specified duration or waits indefinitely.
///
/// This function is used to implement timeouts or indefinite waiting periods.
///
/// # Arguments
///
/// * `wait_for` - An `Option<Duration>` specifying how long to sleep.
///   If `None`, the function will wait indefinitely.
#[inline]
async fn sleep_or_pending(wait_for: Option<Duration>) {
    match wait_for {
        Some(wait) => sleep(wait).await,
        None => pending().await,
    };
}

/// Serves a single HTTP connection from a hyper service backend.
///
/// This method handles an individual HTTP connection, processing requests through
/// the provided service and managing the connection lifecycle.
///
/// # Type Parameters
///
/// * `B`: The body type for the HTTP response.
/// * `IO`: The I/O type for the HTTP connection.
/// * `S`: The service type that processes HTTP requests.
/// * `E`: The executor type for the HTTP server connection.
///
/// # Arguments
///
/// * `hyper_io`: The I/O object representing the inbound hyper IO stream.
/// * `hyper_service`: The hyper `Service` implementation used to process HTTP requests.
/// * `builder`: A `Builder` used to create and serve the HTTP connection.
/// * `watcher`: An optional `tokio::sync::watch::Receiver` for graceful shutdown signaling.
/// * `max_connection_age`: An optional `Duration` specifying the maximum age of the connection
///   before initiating a graceful shutdown.
#[inline]
pub async fn serve_http_connection<B, IO, S, E>(
    hyper_io: IO,
    hyper_service: S,
    builder: HttpConnectionBuilder<E>,
    mut watcher: Option<tokio::sync::watch::Receiver<()>>,
    max_connection_age: Option<Duration>,
) where
    B: Body + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send + Sync,
    IO: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
    S: Service<Request<Incoming>, Response = Response<B>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
    E: HttpServerConnExec<S::Future, B> + Send + Sync + 'static,
{
    // Spawn a new asynchronous task to handle the incoming hyper IO stream
    tokio::spawn(async move {
        {
            // Set up a fused future for the watcher
            let mut sig = pin!(crate::fuse::Fuse {
                inner: watcher.as_mut().map(|w| w.changed()),
            });

            let builder = builder.clone();
            // TODO(How to accept a preconfigured builder)
            // The API here for hyper_util is poor.
            // Really what you want to do is configure a builder like this
            // and pass it in for use as a builder, however, you cannot
            // the simple way may be to require configuration and
            // then accept an immutable reference to an http2 connection builder
            let mut builder = builder.clone();
            builder
                // HTTP/1 settings
                .http1()
                .half_close(true)
                .keep_alive(true)
                .max_buf_size(64 * 1024)
                .pipeline_flush(true)
                .preserve_header_case(true)
                .title_case_headers(false)

                // HTTP/2 settings
                .http2()
                .initial_stream_window_size(Some(1024 * 1024))
                .initial_connection_window_size(Some(2 * 1024 * 1024))
                .adaptive_window(true)
                .max_frame_size(Some(16 * 1024))
                .max_concurrent_streams(Some(1000))
                .max_send_buf_size(1024 * 1024)
                .enable_connect_protocol()
                .max_header_list_size(16 * 1024)
                .keep_alive_interval(Some(Duration::from_secs(20)))
                .keep_alive_timeout(Duration::from_secs(20));

            // Create and pin the HTTP connection
            let mut conn = pin!(builder.serve_connection_with_upgrades(hyper_io, hyper_service));

            // Set up the sleep future for max connection age
            let sleep = sleep_or_pending(max_connection_age);
            tokio::pin!(sleep);

            // Main loop for serving the HTTP connection
            loop {
                tokio::select! {
                    // Handle the connection result
                    rv = &mut conn => {
                        if let Err(err) = rv {
                            // Log any errors that occur while serving the HTTP connection
                            debug!("failed serving HTTP connection: {:#}", err);
                        }
                        break;
                    },
                    // Handle max connection age timeout
                    _ = &mut sleep  => {
                        // Initiate a graceful shutdown when max connection age is reached
                        conn.as_mut().graceful_shutdown();
                        sleep.set(sleep_or_pending(None));
                    },
                    // Handle graceful shutdown signal
                    _ = &mut sig => {
                        // Initiate a graceful shutdown when signal is received
                        conn.as_mut().graceful_shutdown();
                    }
                }
            }
        }

        // Clean up and log connection closure
        drop(watcher);
        trace!("HTTP connection closed");
    });
}

/// Serves HTTP/HTTPS requests with graceful shutdown capability.
///
/// This function sets up an HTTP/HTTPS server that can handle incoming connections and
/// process requests using the provided service. It supports both plain HTTP and HTTPS
/// connections, as well as graceful shutdown.
///
/// # Type Parameters
///
/// * `E`: The executor type for the HTTP server connection.
/// * `F`: The future type for the shutdown signal.
/// * `I`: The incoming stream of IO objects.
/// * `IO`: The I/O type for the HTTP connection.
/// * `IE`: The error type for the incoming stream.
/// * `ResBody`: The response body type.
/// * `S`: The service type that processes HTTP requests.
///
/// # Arguments
///
/// * `service`: The service used to process HTTP requests.
/// * `incoming`: The stream of incoming connections.
/// * `builder`: The `HttpConnectionBuilder` used to configure the server.
/// * `tls_config`: An optional TLS configuration for HTTPS support.
/// * `signal`: An optional future that, when resolved, signals the server to shut down gracefully.
///
/// # Returns
///
/// A `Result` indicating success or failure of the server operation.
///
/// # Examples
///
/// These examples provide some very basic ways to use the server. With that said,
/// the server is very flexible and can be used in a variety of ways. This is
/// because you as the integrator have control over every level of the stack at
/// construction, with all the native builders exposed via generics.
///
/// Setting up an HTTP server with graceful shutdown:
///
/// ```rust,no_run
/// use std::convert::Infallible;
/// use bytes::Bytes;
/// use http_body_util::Full;
/// use hyper::body::Incoming;
/// use hyper::{Request, Response};
/// use hyper_util::rt::TokioExecutor;
/// use hyper_util::server::conn::auto::Builder as HttpConnectionBuilder;
/// use tokio::net::TcpListener;
/// use tokio_stream::wrappers::TcpListenerStream;
/// use tower::ServiceBuilder;
/// use std::net::SocketAddr;
///
/// use hyper_server::serve_http_with_shutdown;
///
/// async fn hello(_: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
///     Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
/// }
///
/// #[tokio::main(flavor = "current_thread")]
/// async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
///     let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
///     let listener = TcpListener::bind(addr).await?;
///     let incoming = TcpListenerStream::new(listener);
///
///     let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
///
///     let builder = HttpConnectionBuilder::new(TokioExecutor::new());
///     let svc = hyper::service::service_fn(hello);
///     let svc = ServiceBuilder::new().service(svc);
///
///     tokio::spawn(async move {
///         // Simulate a shutdown signal after 60 seconds
///         tokio::time::sleep(std::time::Duration::from_secs(60)).await;
///         let _ = shutdown_tx.send(());
///     });
///
///     serve_http_with_shutdown(
///         svc,
///         incoming,
///         builder,
///         None, // No TLS config for plain HTTP
///         Some(async {
///             shutdown_rx.await.ok();
///         }),
///     ).await?;
///
///     Ok(())
/// }
/// ```
///
/// Setting up an HTTPS server:
///
/// ```rust,no_run
/// use std::convert::Infallible;
/// use std::sync::Arc;
/// use bytes::Bytes;
/// use http_body_util::Full;
/// use hyper::body::Incoming;
/// use hyper::{Request, Response};
/// use hyper_util::rt::TokioExecutor;
/// use hyper_util::server::conn::auto::Builder as HttpConnectionBuilder;
/// use tokio::net::TcpListener;
/// use tokio_stream::wrappers::TcpListenerStream;
/// use tower::ServiceBuilder;
/// use rustls::ServerConfig;
/// use std::io;
/// use std::net::SocketAddr;
/// use std::future::Future;
///
/// use hyper_server::{serve_http_with_shutdown, load_certs, load_private_key};
///
/// async fn hello(_: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
///     Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
/// }
///
/// #[tokio::main(flavor = "current_thread")]
/// async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
///     let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
///     let listener = TcpListener::bind(addr).await?;
///     let incoming = TcpListenerStream::new(listener);
///
///     let builder = HttpConnectionBuilder::new(TokioExecutor::new());
///     let svc = hyper::service::service_fn(hello);
///     let svc = ServiceBuilder::new().service(svc);
///
///     // Set up TLS config
///     let certs = load_certs("examples/sample.pem")?;
///     let key = load_private_key("examples/sample.rsa")?;
///
///     let config = ServerConfig::builder()
///         .with_no_client_auth()
///         .with_single_cert(certs, key)
///         .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
///     let tls_config = Arc::new(config);
///
///     serve_http_with_shutdown(
///         svc,
///         incoming,
///         builder,
///         Some(tls_config),
///         Some(std::future::pending::<()>()), // A never-resolving future as a placeholder
///     ).await?;
///
///     Ok(())
/// }
/// ```
///
/// Setting up an HTTPS server with a Tower service:
///
/// ```rust,no_run
/// use std::convert::Infallible;
/// use std::sync::Arc;
/// use bytes::Bytes;
/// use http_body_util::Full;
/// use hyper::body::Incoming;
/// use hyper::{Request, Response};
/// use hyper_util::rt::TokioExecutor;
/// use hyper_util::server::conn::auto::Builder as HttpConnectionBuilder;
/// use hyper_util::service::TowerToHyperService;
/// use tokio::net::TcpListener;
/// use tokio_stream::wrappers::TcpListenerStream;
/// use tower::{ServiceBuilder, ServiceExt};
/// use rustls::ServerConfig;
/// use std::io;
/// use std::net::SocketAddr;
/// use std::future::Future;
///
/// use hyper_server::{serve_http_with_shutdown, load_certs, load_private_key};
///
/// async fn hello(_: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
///     Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
/// }
///
/// #[tokio::main(flavor = "current_thread")]
/// async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
///     let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
///     let listener = TcpListener::bind(addr).await?;
///     let incoming = TcpListenerStream::new(listener);
///
///     let builder = HttpConnectionBuilder::new(TokioExecutor::new());
///
///     // Set up the Tower service
///     let svc = tower::service_fn(hello);
///     let svc = ServiceBuilder::new()
///         .service(svc);
///
///     // Convert the Tower service to a Hyper service
///     let svc = TowerToHyperService::new(svc);
///
///     // Set up TLS config
///     let certs = load_certs("examples/sample.pem")?;
///     let key = load_private_key("examples/sample.rsa")?;
///
///     let config = ServerConfig::builder()
///         .with_no_client_auth()
///         .with_single_cert(certs, key)
///         .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
///     let tls_config = Arc::new(config);
///
///     serve_http_with_shutdown(
///         svc,
///         incoming,
///         builder,
///         Some(tls_config),
///         Some(std::future::pending::<()>()), // A never-resolving future as a placeholder
///     ).await?;
///
///     Ok(())
/// }
/// ```
///
/// # Notes
///
/// - The server will continue to accept new connections until the `signal` future resolves.
/// - When using TLS, make sure to provide a properly configured `ServerConfig`.
/// - The function will return when all connections have been closed after the shutdown signal.
#[inline]
pub async fn serve_http_with_shutdown<E, F, I, IO, IE, ResBody, S>(
    service: S,
    incoming: I,
    builder: HttpConnectionBuilder<E>,
    tls_config: Option<Arc<rustls::ServerConfig>>,
    signal: Option<F>,
) -> Result<(), super::Error>
where
    F: Future<Output = ()> + Send + 'static,
    I: Stream<Item = Result<IO, IE>> + Send + 'static,
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    IE: Into<crate::Error> + Send + 'static,
    S: Service<Request<Incoming>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
    ResBody: Body<Data = Bytes> + Send + Sync + 'static,
    ResBody::Error: Into<crate::Error> + Send + Sync,
    E: HttpServerConnExec<S::Future, ResBody> + Send + Sync + 'static,
{
    // Prepare the incoming stream of TCP connections
    let incoming = crate::tcp::serve_tcp_incoming(incoming);

    // Create a channel for signaling graceful shutdown
    let (signal_tx, signal_rx) = tokio::sync::watch::channel(());
    let signal_tx = Arc::new(signal_tx);

    let graceful = signal.is_some();
    let mut sig = pin!(Fuse { inner: signal });
    let mut incoming = pin!(incoming);

    // Create TLS acceptor if TLS config is provided
    let tls_acceptor = tls_config.map(TlsAcceptor::from);

    // Main server loop
    loop {
        tokio::select! {
            // Handle shutdown signal
            _ = &mut sig => {
                trace!("signal received, shutting down");
                break;
            },
            // Handle incoming connections
            io = incoming.next() => {
                let io = match io {
                    Some(Ok(io)) => io,
                    Some(Err(e)) => {
                        trace!("error accepting connection: {:#}", e);
                        continue;
                    },
                    None => {
                        break
                    },
                };

                trace!("connection accepted");

                // Prepare the connection for hyper
                let transport = if let Some(tls_acceptor) = &tls_acceptor {
                    match tls_acceptor.accept(io).await {
                        Ok(tls_stream) => Transport::new_tls(tls_stream),
                        Err(e) => {
                            debug!("TLS handshake failed: {:#}", e);
                            continue;
                        }
                    }
                } else {
                    Transport::new_plain(io)
                };

                let hyper_io = TokioIo::new(transport);
                let hyper_svc = service.clone();

                // Serve the HTTP connection
                serve_http_connection(
                    hyper_io,
                    hyper_svc,
                    builder.clone(),
                    graceful.then(|| signal_rx.clone()),
                    None
                ).await;
            }
        }
    }

    // Handle graceful shutdown
    if graceful {
        let _ = signal_tx.send(());
        drop(signal_rx);
        trace!(
            "waiting for {} connections to close",
            signal_tx.receiver_count()
        );

        // Wait for all connections to close
        signal_tx.closed().await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{load_certs, load_private_key};
    use bytes::Bytes;
    use http_body_util::{BodyExt, Empty, Full};
    use hyper::{body::Incoming, Request, Response, StatusCode};
    use hyper_util::rt::TokioExecutor;
    use hyper_util::service::TowerToHyperService;
    use rustls::ServerConfig;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;
    use tokio_stream::wrappers::TcpListenerStream;

    // Utility functions

    async fn echo(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
        match (req.method(), req.uri().path()) {
            (&hyper::Method::GET, "/") => {
                Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
            }
            (&hyper::Method::POST, "/echo") => {
                let body = req.collect().await?.to_bytes();
                Ok(Response::new(Full::new(body)))
            }
            _ => {
                let mut res = Response::new(Full::new(Bytes::from("Not Found")));
                *res.status_mut() = StatusCode::NOT_FOUND;
                Ok(res)
            }
        }
    }

    async fn setup_test_server(addr: SocketAddr) -> (TcpListenerStream, SocketAddr) {
        let listener = TcpListener::bind(addr).await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        let incoming = TcpListenerStream::new(listener);
        (incoming, server_addr)
    }

    async fn create_test_tls_config() -> Arc<ServerConfig> {
        let certs = load_certs("examples/sample.pem").unwrap();
        let key = load_private_key("examples/sample.rsa").unwrap();
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();
        Arc::new(config)
    }

    async fn send_request(
        addr: SocketAddr,
        req: Request<Empty<Bytes>>,
    ) -> Result<Response<Incoming>, Box<dyn std::error::Error>> {
        let stream = TcpStream::connect(addr).await?;
        let io = TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::spawn(async move {
            if let Err(err) = conn.await {
                eprintln!("Connection failed: {:?}", err);
            }
        });

        Ok(sender.send_request(req).await?)
    }

    // HTTP Tests

    mod http_tests {
        use super::*;

        #[tokio::test]
        async fn test_http_basic_requests() {
            let addr = SocketAddr::from(([127, 0, 0, 1], 0));
            let (incoming, server_addr) = setup_test_server(addr).await;

            let (shutdown_tx, shutdown_rx) = oneshot::channel();

            let http_server_builder = HttpConnectionBuilder::new(TokioExecutor::new());
            let tower_service_fn = tower::service_fn(echo);
            let hyper_service = TowerToHyperService::new(tower_service_fn);

            let server = tokio::spawn(serve_http_with_shutdown(
                hyper_service,
                incoming,
                http_server_builder,
                None,
                Some(async {
                    shutdown_rx.await.ok();
                }),
            ));

            // Test GET request
            let req = Request::builder()
                .uri("/")
                .body(Empty::<Bytes>::new())
                .unwrap();
            let res = send_request(server_addr, req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            let body = res.collect().await.unwrap().to_bytes();
            assert_eq!(&body[..], b"Hello, World!");

            // Test POST request
            let req = Request::builder()
                .method(hyper::Method::POST)
                .uri("/echo")
                .body(Empty::<Bytes>::new())
                .unwrap();
            let res = send_request(server_addr, req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);

            // Test 404 response
            let req = Request::builder()
                .uri("/not_found")
                .body(Empty::<Bytes>::new())
                .unwrap();
            let res = send_request(server_addr, req).await.unwrap();
            assert_eq!(res.status(), StatusCode::NOT_FOUND);

            shutdown_tx.send(()).unwrap();
            server.await.unwrap().unwrap();
        }

        #[tokio::test]
        async fn test_http_concurrent_requests() {
            let addr = SocketAddr::from(([127, 0, 0, 1], 0));
            let (incoming, server_addr) = setup_test_server(addr).await;

            let (shutdown_tx, shutdown_rx) = oneshot::channel();

            let http_server_builder = HttpConnectionBuilder::new(TokioExecutor::new());
            let tower_service_fn = tower::service_fn(echo);
            let hyper_service = TowerToHyperService::new(tower_service_fn);

            let server = tokio::spawn(serve_http_with_shutdown(
                hyper_service,
                incoming,
                http_server_builder,
                None,
                Some(async {
                    shutdown_rx.await.ok();
                }),
            ));

            let mut handles = vec![];
            for _ in 0..10 {
                let addr = server_addr;
                let handle = tokio::spawn(async move {
                    let req = Request::builder()
                        .uri("/")
                        .body(Empty::<Bytes>::new())
                        .unwrap();
                    let res = send_request(addr, req).await.unwrap();
                    assert_eq!(res.status(), StatusCode::OK);
                });
                handles.push(handle);
            }

            for handle in handles {
                handle.await.unwrap();
            }

            shutdown_tx.send(()).unwrap();
            server.await.unwrap().unwrap();
        }

        #[tokio::test]
        async fn test_http_graceful_shutdown() {
            let addr = SocketAddr::from(([127, 0, 0, 1], 0));
            let (incoming, server_addr) = setup_test_server(addr).await;

            let (shutdown_tx, shutdown_rx) = oneshot::channel();

            let http_server_builder = HttpConnectionBuilder::new(TokioExecutor::new());
            let tower_service_fn = tower::service_fn(echo);
            let hyper_service = TowerToHyperService::new(tower_service_fn);

            let server = tokio::spawn(serve_http_with_shutdown(
                hyper_service,
                incoming,
                http_server_builder,
                None,
                Some(async {
                    shutdown_rx.await.ok();
                }),
            ));

            // Send a request before shutdown
            let req = Request::builder()
                .uri("/")
                .body(Empty::<Bytes>::new())
                .unwrap();
            let res = send_request(server_addr, req)
                .await
                .expect("Failed to send initial request");
            assert_eq!(res.status(), StatusCode::OK);

            // Initiate graceful shutdown
            shutdown_tx.send(()).unwrap();

            // Wait for the server to shut down
            let shutdown_timeout = Duration::from_millis(150);
            let shutdown_result = tokio::time::timeout(shutdown_timeout, async {
                loop {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    let req = Request::builder()
                        .uri("/")
                        .body(Empty::<Bytes>::new())
                        .unwrap();
                    match send_request(server_addr, req).await {
                        Ok(_) => continue, // Server still accepting connections
                        Err(e) if e.to_string().contains("Connection refused") => {
                            // Server has shut down as expected
                            return Ok(());
                        }
                        Err(e) => return Err(e), // Unexpected error
                    }
                }
            })
            .await;

            match shutdown_result {
                Ok(Ok(())) => println!("Server shut down successfully"),
                Ok(Err(e)) => panic!("Unexpected error during shutdown: {}", e),
                Err(_) => panic!("Timeout waiting for server to shut down"),
            }

            // Ensure the server task completes
            server.await.unwrap().unwrap();
        }
    }

    // HTTPS Tests

    mod https_tests {
        use super::*;

        async fn create_https_client() -> (
            tokio_rustls::TlsConnector,
            rustls::pki_types::ServerName<'static>,
        ) {
            let mut root_cert_store = rustls::RootCertStore::empty();
            root_cert_store.add_parsable_certificates(load_certs("examples/sample.pem").unwrap());

            let client_config = rustls::ClientConfig::builder()
                .with_root_certificates(root_cert_store)
                .with_no_client_auth();

            let tls_connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));
            let domain = rustls::pki_types::ServerName::try_from("localhost")
                .expect("Failed to create ServerName");

            (tls_connector, domain)
        }

        #[tokio::test]
        async fn test_https_connection() {
            let addr = SocketAddr::from(([127, 0, 0, 1], 0));
            let (incoming, server_addr) = setup_test_server(addr).await;

            let tls_config = create_test_tls_config().await;
            let (shutdown_tx, shutdown_rx) = oneshot::channel();

            let http_server_builder = HttpConnectionBuilder::new(TokioExecutor::new());
            let tower_service_fn = tower::service_fn(echo);
            let hyper_service = TowerToHyperService::new(tower_service_fn);

            let server = tokio::spawn(serve_http_with_shutdown(
                hyper_service,
                incoming,
                http_server_builder,
                Some(tls_config),
                Some(async {
                    shutdown_rx.await.ok();
                }),
            ));

            let (tls_connector, domain) = create_https_client().await;

            let tcp_stream = TcpStream::connect(server_addr).await.unwrap();
            let tls_stream = tls_connector.connect(domain, tcp_stream).await.unwrap();

            let (mut sender, conn) =
                hyper::client::conn::http1::handshake(TokioIo::new(tls_stream))
                    .await
                    .unwrap();

            tokio::spawn(async move {
                if let Err(err) = conn.await {
                    eprintln!("Connection failed: {:?}", err);
                }
            });

            let req = Request::builder()
                .uri("/")
                .body(Empty::<Bytes>::new())
                .unwrap();

            let res = sender.send_request(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);

            let body = res.collect().await.unwrap().to_bytes();
            assert_eq!(&body[..], b"Hello, World!");

            shutdown_tx.send(()).unwrap();
            server.await.unwrap().unwrap();
        }

        #[tokio::test]
        async fn test_https_invalid_client_cert() {
            let addr = SocketAddr::from(([127, 0, 0, 1], 0));
            let (incoming, server_addr) = setup_test_server(addr).await;

            let tls_config = create_test_tls_config().await;
            let (shutdown_tx, shutdown_rx) = oneshot::channel();

            let http_server_builder = HttpConnectionBuilder::new(TokioExecutor::new());
            let tower_service_fn = tower::service_fn(echo);
            let hyper_service = TowerToHyperService::new(tower_service_fn);

            let server = tokio::spawn(serve_http_with_shutdown(
                hyper_service,
                incoming,
                http_server_builder,
                Some(tls_config),
                Some(async {
                    shutdown_rx.await.ok();
                }),
            ));

            let client_config = rustls::ClientConfig::builder()
                .with_root_certificates(rustls::RootCertStore::empty())
                .with_no_client_auth();

            let tls_connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));

            let tcp_stream = TcpStream::connect(server_addr).await.unwrap();
            let domain = rustls::pki_types::ServerName::try_from("localhost").unwrap();

            let result = tls_connector.connect(domain, tcp_stream).await;
            assert!(
                result.is_err(),
                "Expected TLS connection to fail due to invalid client certificate"
            );

            shutdown_tx.send(()).unwrap();
            server.await.unwrap().unwrap();
        }
        #[tokio::test]
        async fn test_https_graceful_shutdown() {
            let addr = SocketAddr::from(([127, 0, 0, 1], 0));
            let (incoming, server_addr) = setup_test_server(addr).await;

            let tls_config = create_test_tls_config().await;
            let (shutdown_tx, shutdown_rx) = oneshot::channel();

            let http_server_builder = HttpConnectionBuilder::new(TokioExecutor::new());
            let tower_service_fn = tower::service_fn(echo);
            let hyper_service = TowerToHyperService::new(tower_service_fn);

            let server = tokio::spawn(serve_http_with_shutdown(
                hyper_service,
                incoming,
                http_server_builder,
                Some(tls_config),
                Some(async {
                    shutdown_rx.await.ok();
                }),
            ));

            let (tls_connector, domain) = create_https_client().await;

            // Establish a connection
            let tcp_stream = TcpStream::connect(server_addr).await.unwrap();
            let tls_stream = tls_connector.connect(domain, tcp_stream).await.unwrap();

            let (mut sender, conn) =
                hyper::client::conn::http1::handshake(TokioIo::new(tls_stream))
                    .await
                    .unwrap();

            tokio::spawn(async move {
                if let Err(err) = conn.await {
                    eprintln!("Connection failed: {:?}", err);
                }
            });

            // Send a request
            let req = Request::builder()
                .uri("/")
                .body(Empty::<Bytes>::new())
                .unwrap();

            let res = sender.send_request(req).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK);

            // Initiate graceful shutdown
            shutdown_tx.send(()).unwrap();

            // Wait a bit to allow the server to start shutting down
            tokio::time::sleep(Duration::from_millis(10)).await;

            // Try to send another request, it should fail
            let req = Request::builder()
                .uri("/")
                .body(Empty::<Bytes>::new())
                .unwrap();

            let result = sender.send_request(req).await;
            assert!(
                result.is_err(),
                "Expected request to fail after graceful shutdown"
            );

            server.await.unwrap().unwrap();
        }
    }
}
