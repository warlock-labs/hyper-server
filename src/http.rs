use std::future::pending;
use std::{future::Future, pin::pin, sync::Arc, time::Duration};

use bytes::Bytes;
use http::{Request, Response};
use http_body::Body;
use hyper::body::Incoming;
use hyper::service::Service;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
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

            // Create and pin the HTTP connection
            let mut conn = pin!(builder.serve_connection(hyper_io, hyper_service));

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

/// Serves HTTP requests with graceful shutdown capability.
///
/// This function sets up an HTTP server that can handle incoming connections and
/// process requests using the provided service. It also supports graceful shutdown.
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
/// * `signal`: An optional future that, when resolved, signals the server to shut down gracefully.
///
/// # Returns
///
/// A `Result` indicating success or failure of the server operation.
pub async fn serve_http_with_shutdown<E, F, I, IO, IE, ResBody, S>(
    service: S,
    incoming: I,
    builder: HttpConnectionBuilder<E>,
    signal: Option<F>,
) -> Result<(), super::Error>
where
    F: Future<Output = ()>,
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
                let hyper_io = TokioIo::new(io);
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
    use std::net::SocketAddr;
    use std::time::Duration;

    use bytes::Bytes;
    use http_body_util::{BodyExt, Empty, Full};
    use hyper::{body::Incoming, Request, Response, StatusCode};
    use hyper_util::service::TowerToHyperService;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;
    use tokio_stream::wrappers::TcpListenerStream;

    use super::*;

    // Echo service
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

    async fn send_request(
        addr: SocketAddr,
        req: Request<Empty<Bytes>>,
    ) -> hyper::Result<Response<Incoming>> {
        let stream = TcpStream::connect(addr).await.unwrap();
        let io = TokioIo::new(stream);

        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::spawn(async move {
            if let Err(err) = conn.await {
                eprintln!("Connection failed: {:?}", err);
            }
        });

        sender.send_request(req).await
    }

    #[tokio::test]
    async fn test_serve_http_with_shutdown_basic() {
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

        // Shutdown the server
        shutdown_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("Server didn't shut down within the timeout period")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn test_serve_http_with_concurrent_requests() {
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
            Some(async {
                shutdown_rx.await.ok();
            }),
        ));

        let mut handles = vec![];
        for _ in 0..10 {
            let handle = tokio::spawn(async move {
                let req = Request::builder()
                    .uri("/")
                    .body(Empty::<Bytes>::new())
                    .unwrap();
                let res = send_request(server_addr, req).await.unwrap();
                assert_eq!(res.status(), StatusCode::OK);
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        // Shutdown the server
        shutdown_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("Server didn't shut down within the timeout period")
            .unwrap()
            .unwrap();
    }
}
