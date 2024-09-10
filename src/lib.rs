use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body::Body;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::{body::Incoming, service::Service as HyperService};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as HttpConnBuilder;
use hyper_util::server::conn::auto::HttpServerConnExec;
use hyper_util::service::TowerToHyperService;
use pin_project::pin_project;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use std::error::Error as StdError;
use std::future::pending;
use std::net::SocketAddr;
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use std::{fmt, fs, future::Future, io};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::macros::support::poll_fn;
use tokio::net::TcpListener;
use tokio::time::sleep;
use tokio_rustls::TlsAcceptor;
use tokio_stream::Stream;
use tokio_stream::StreamExt as _;
use tower::{Service, ServiceBuilder};
use tracing::{debug, trace};

// From `futures-util` crate, borrowed since this is the only dependency hyper-server requires.
// LICENSE: MIT or Apache-2.0
// A future which only yields `Poll::Ready` once, and thereafter yields `Poll::Pending`.
#[pin_project]
struct Fuse<F> {
    #[pin]
    inner: Option<F>,
}

impl<F> Future for Fuse<F>
where
    F: Future,
{
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.as_mut().project().inner.as_pin_mut() {
            Some(fut) => fut.poll(cx).map(|output| {
                self.project().inner.set(None);
                output
            }),
            None => Poll::Pending,
        }
    }
}

type Source = Box<dyn StdError + Send + Sync + 'static>;

/// Errors that originate from the client hyper-server;
pub struct Error {
    inner: ErrorImpl,
}

struct ErrorImpl {
    kind: Kind,
    source: Option<Source>,
}

#[derive(Debug)]
pub(crate) enum Kind {
    Transport,
}

impl Error {
    pub(crate) fn new(kind: Kind) -> Self {
        Self {
            inner: ErrorImpl { kind, source: None },
        }
    }

    pub(crate) fn with(mut self, source: impl Into<Source>) -> Self {
        self.inner.source = Some(source.into());
        self
    }

    pub(crate) fn from_source(
        source: impl Into<Error> + std::error::Error + std::marker::Send + std::marker::Sync + 'static,
    ) -> Self {
        Error::new(Kind::Transport).with(source)
    }

    fn description(&self) -> &str {
        match &self.inner.kind {
            Kind::Transport => "transport error",
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_tuple("hyper_server::Error");

        f.field(&self.inner.kind);

        if let Some(source) = &self.inner.source {
            f.field(source);
        }

        f.finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.description())
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.inner
            .source
            .as_ref()
            .map(|source| &**source as &(dyn StdError + 'static))
    }
}

async fn sleep_or_pending(wait_for: Option<Duration>) {
    match wait_for {
        Some(wait) => sleep(wait).await,
        None => pending().await,
    };
}

#[derive(Debug, Clone)]
pub struct Logger<S> {
    inner: S,
}
impl<S> Logger<S> {
    pub fn new(inner: S) -> Self {
        Logger { inner }
    }
}
type Req = Request<Incoming>;

impl<S> Service<Req> for Logger<S>
where
    S: Service<Req> + Clone,
{
    type Response = S::Response;

    type Error = S::Error;

    type Future = S::Future;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Req) -> Self::Future {
        println!("processing request: {} {}", req.method(), req.uri().path());
        self.inner.call(req)
    }
}

// Wrapped error type for the server.
fn error(err: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

// Load the public certificate from a file.
fn load_certs(filename: &str) -> io::Result<Vec<CertificateDer<'static>>> {
    // Open certificate file.
    let certfile = fs::File::open(filename)
        .map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
    let mut reader = io::BufReader::new(certfile);

    // Load and return certificate.
    rustls_pemfile::certs(&mut reader).collect()
}

// Load the private key from a file.
fn load_private_key(filename: &str) -> io::Result<PrivateKeyDer<'static>> {
    // Open keyfile.
    let keyfile = fs::File::open(filename)
        .map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
    let mut reader = io::BufReader::new(keyfile);

    // Load and return a single private key.
    rustls_pemfile::private_key(&mut reader).map(|key| key.unwrap())
}

// Custom echo service, handling two different routes and a
// catch-all 404/not-found responder.
async fn echo(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let mut response = Response::new(Full::default());
    match (req.method(), req.uri().path()) {
        // Help route.
        (&Method::GET, "/") => {
            *response.body_mut() = Full::from("Try POST /echo\n");
        }
        // Echo service route.
        (&Method::POST, "/echo") => {
            *response.body_mut() = Full::from(req.into_body().collect().await?.to_bytes());
        }
        // Catch-all 404.
        _ => {
            *response.status_mut() = StatusCode::NOT_FOUND;
        }
    };
    Ok(response)
}

pub struct Server {}

impl Server {
    pub async fn serve(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Get a random port from the OS
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));

        // Create a TCP listener bound to the random address
        let incoming = TcpListener::bind(&addr).await?;

        // Load public certificate.
        let certs = load_certs("examples/sample.pem")?;

        // Load private key.
        let key = load_private_key("examples/sample.rsa")?;

        // Build TLS configuration.
        let mut server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .unwrap();

        // Enable ALPN with HTTP/2 and HTTP/1.1 support.
        server_config.alpn_protocols =
            vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];

        // Create a rustls TlsAcceptor
        let tls_acceptor = TlsAcceptor::from(Arc::new(server_config));

        // Create a tower service
        let service = tower::service_fn(echo);
        let service = ServiceBuilder::new().layer_fn(Logger::new).service(service);

        // Convert it to a hyper service
        let service = TowerToHyperService::new(service);

        // Begin the server loop
        loop {
            // Wait for an incoming tcp stream
            let (tcp_stream, _remote_addr) = incoming.accept().await.unwrap();

            // Clone a new instance of the tls_acceptor
            let tls_acceptor = tls_acceptor.clone();

            // Clone a new instance of the service
            let service = service.clone();

            // Spawn a new async task to handle the incoming connection
            tokio::spawn(async move {
                // Perform the TLS handshake
                let tls_stream = match tls_acceptor.accept(tcp_stream).await {
                    Ok(tls_stream) => tls_stream,
                    Err(err) => {
                        eprintln!("failed to perform tls handshake: {err:#}");
                        return;
                    }
                };

                // Serve the http connection
                if let Err(err) = HttpConnBuilder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(tls_stream), service)
                    .await
                {
                    eprintln!("failed to serve connection: {err:#}");
                }
            });
        }
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
    /// # Parameters
    ///
    /// * `hyper_io`: The I/O object representing the inbound hyper IO stream.
    /// * `hyper_svc`: The hyper `Service` implementation used to process HTTP requests.
    /// * `builder`: An `HttpConnBuilder` used to create and serve the HTTP connection.
    /// * `watcher`: An optional `tokio::sync::watch::Receiver` for graceful shutdown signaling.
    /// * `max_connection_age`: An optional `Duration` specifying the maximum age of the connection
    ///   before initiating a graceful shutdown.
    async fn serve_http_connection<B, IO, S, E>(
        hyper_io: IO,
        hyper_service: S,
        builder: HttpConnBuilder<E>,
        mut watcher: Option<tokio::sync::watch::Receiver<()>>,
        max_connection_age: Option<Duration>,
    ) where
        B: Body + Send + 'static,
        B::Data: Send,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send + Sync,
        IO: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
        S: HyperService<Request<Incoming>, Response=Response<B>> + Clone + Send + 'static,
        S::Future: Send + 'static,
        S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
        E: HttpServerConnExec<S::Future, B> + Send + Sync + 'static,
    {
        // Spawn a new asynchronous task to handle the incoming hyper IO stream
        tokio::spawn(async move {
            {
                // Set up a fused future for the watcher
                let mut sig = pin!(Fuse {
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
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_echo_service() {}
}
