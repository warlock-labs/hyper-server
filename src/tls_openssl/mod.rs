//! Tls implementation using [`openssl`]
//!
//! # Example
//!
//! ```rust,no_run
//! use axum::{routing::get, Router};
//! use hyper_server::tls_openssl::OpenSSLConfig;
//! use std::net::SocketAddr;
//!
//! #[tokio::main]
//! async fn main() {
//!     let app = Router::new().route("/", get(|| async { "Hello, world!" }));
//!
//!     let config = OpenSSLConfig::from_pem_file(
//!         "examples/self-signed-certs/cert.pem",
//!         "examples/self-signed-certs/key.pem",
//!     )
//!     .unwrap();
//!
//!     let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
//!     println!("listening on {}", addr);
//!     hyper_server::bind_openssl(addr, config)
//!         .serve(app.into_make_service())
//!         .await
//!         .unwrap();
//! }
//! ```

use self::future::OpenSSLAcceptorFuture;
use crate::{
    accept::{Accept, DefaultAcceptor},
    server::Server,
};
use openssl::ssl::{
    Error as OpenSSLError, SslAcceptor, SslAcceptorBuilder, SslFiletype, SslMethod,
};
use std::{convert::TryFrom, fmt, net::SocketAddr, path::Path, sync::Arc, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_openssl::SslStream;

pub mod future;

/// Binds a TLS server using OpenSSL to the specified address with the given configuration.
///
/// The server is configured to accept TLS encrypted connections.
///
/// # Arguments
///
/// * `addr`: The address to which the server will bind.
/// * `config`: The TLS configuration for the server.
///
/// # Returns
///
/// A configured `Server` instance ready to be run.
pub fn bind_openssl(addr: SocketAddr, config: OpenSSLConfig) -> Server<OpenSSLAcceptor> {
    let acceptor = OpenSSLAcceptor::new(config);
    Server::bind(addr).acceptor(acceptor)
}

/// Represents a TLS acceptor that uses OpenSSL for cryptographic operations.
///
/// This structure is used for handling TLS encrypted connections.
///
/// The acceptor is backed by OpenSSL, and is used to upgrade incoming non-secure connections
/// to secure TLS connections.
///
/// The default TLS handshake timeout is set to 10 seconds.
#[derive(Clone)]
pub struct OpenSSLAcceptor<A = DefaultAcceptor> {
    inner: A,
    config: OpenSSLConfig,
    handshake_timeout: Duration,
}

impl OpenSSLAcceptor {
    /// Constructs a new instance of the OpenSSL acceptor.
    ///
    /// # Arguments
    ///
    /// * `config`: Configuration options for the OpenSSL server.
    pub fn new(config: OpenSSLConfig) -> Self {
        let inner = DefaultAcceptor::new();

        // Default handshake timeout is 10 seconds.
        #[cfg(not(test))]
        let handshake_timeout = Duration::from_secs(10);

        // For tests, use a shorter timeout to avoid unnecessary delays.
        #[cfg(test)]
        let handshake_timeout = Duration::from_secs(1);

        Self {
            inner,
            config,
            handshake_timeout,
        }
    }

    /// Overrides the default TLS handshake timeout.
    ///
    /// # Arguments
    ///
    /// * `val`: The duration to set as the new handshake timeout.
    ///
    /// # Returns
    ///
    /// A modified version of the current acceptor with the new timeout value.
    pub fn handshake_timeout(mut self, val: Duration) -> Self {
        self.handshake_timeout = val;
        self
    }
}

impl<A, I, S> Accept<I, S> for OpenSSLAcceptor<A>
where
    A: Accept<I, S>,
    A::Stream: AsyncRead + AsyncWrite + Unpin,
{
    type Stream = SslStream<A::Stream>;
    type Service = A::Service;
    type Future = OpenSSLAcceptorFuture<A::Future, A::Stream, A::Service>;

    /// Handles the incoming stream, initiates a TLS handshake, and upgrades it to a secure connection.
    fn accept(&self, stream: I, service: S) -> Self::Future {
        let inner_future = self.inner.accept(stream, service);
        let config = self.config.clone();

        OpenSSLAcceptorFuture::new(inner_future, config, self.handshake_timeout)
    }
}

impl<A> fmt::Debug for OpenSSLAcceptor<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenSSLAcceptor").finish()
    }
}

/// Represents configuration options for an OpenSSL-based server.
///
/// This configuration is used when constructing a new `OpenSSLAcceptor`.
#[derive(Clone)]
pub struct OpenSSLConfig {
    acceptor: Arc<SslAcceptor>,
}

impl OpenSSLConfig {
    /// Creates a new configuration using a PEM formatted certificate and key.
    ///
    /// # Arguments
    ///
    /// * `cert`: Path to the PEM-formatted certificate file.
    /// * `key`: Path to the PEM-formatted private key file.
    ///
    /// # Returns
    ///
    /// A `Result` that contains an `OpenSSLConfig` or an `OpenSSLError`.
    pub fn from_pem_file<A: AsRef<Path>, B: AsRef<Path>>(
        cert: A,
        key: B,
    ) -> Result<Self, OpenSSLError> {
        let mut tls_builder = SslAcceptor::mozilla_modern_v5(SslMethod::tls())?;

        tls_builder.set_certificate_file(cert, SslFiletype::PEM)?;
        tls_builder.set_private_key_file(key, SslFiletype::PEM)?;
        tls_builder.check_private_key()?;

        let acceptor = Arc::new(tls_builder.build());

        Ok(OpenSSLConfig { acceptor })
    }

    /// Creates a new configuration using a PEM formatted certificate chain and key.
    ///
    /// # Arguments
    ///
    /// * `chain`: Path to the PEM-formatted certificate chain file.
    /// * `key`: Path to the PEM-formatted private key file.
    ///
    /// # Returns
    ///
    /// A `Result` that contains an `OpenSSLConfig` or an `OpenSSLError`.
    pub fn from_pem_chain_file<A: AsRef<Path>, B: AsRef<Path>>(
        chain: A,
        key: B,
    ) -> Result<Self, OpenSSLError> {
        let mut tls_builder = SslAcceptor::mozilla_modern_v5(SslMethod::tls())?;

        tls_builder.set_certificate_chain_file(chain)?;
        tls_builder.set_private_key_file(key, SslFiletype::PEM)?;
        tls_builder.check_private_key()?;

        let acceptor = Arc::new(tls_builder.build());

        Ok(OpenSSLConfig { acceptor })
    }
}

impl TryFrom<SslAcceptorBuilder> for OpenSSLConfig {
    type Error = OpenSSLError;

    /// Constructs an `OpenSSLConfig` from an `SslAcceptorBuilder`.
    ///
    /// This conversion allows for finer control over the TLS settings when creating
    /// the configuration.
    fn try_from(tls_builder: SslAcceptorBuilder) -> Result<Self, Self::Error> {
        tls_builder.check_private_key()?;
        let acceptor = Arc::new(tls_builder.build());
        Ok(OpenSSLConfig { acceptor })
    }
}

impl fmt::Debug for OpenSSLConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenSSLConfig").finish()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        handle::Handle,
        tls_openssl::{self, OpenSSLConfig},
    };
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

    use openssl::ssl::{Ssl, SslConnector, SslMethod, SslVerifyMode};
    use std::pin::Pin;
    use tokio_openssl::SslStream;

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

    #[tokio::test]
    async fn test_graceful_shutdown() {
        let (handle, server_task, addr) = start_server().await;

        let (mut client, conn) = connect(addr).await;

        handle.graceful_shutdown(None);

        let (_parts, body) = send_empty_request(&mut client).await;

        assert_eq!(body.as_ref(), b"Hello, world!");

        // Disconnect client.
        conn.abort();

        // Server task should finish soon.
        let server_result = timeout(Duration::from_secs(1), server_task)
            .await
            .unwrap()
            .unwrap();

        assert!(server_result.is_ok());
    }

    #[ignore]
    #[tokio::test]
    async fn test_graceful_shutdown_timed() {
        let (handle, server_task, addr) = start_server().await;

        let (mut client, _conn) = connect(addr).await;

        handle.graceful_shutdown(Some(Duration::from_millis(250)));

        let (_parts, body) = send_empty_request(&mut client).await;

        assert_eq!(body.as_ref(), b"Hello, world!");

        // Don't disconnect client.
        // conn.abort();

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

            let config = OpenSSLConfig::from_pem_file(
                "examples/self-signed-certs/cert.pem",
                "examples/self-signed-certs/key.pem",
            )
            .unwrap();

            let addr = SocketAddr::from(([127, 0, 0, 1], 0));

            tls_openssl::bind_openssl(addr, config)
                .handle(server_handle)
                .serve(app.into_make_service())
                .await
        });

        let addr = handle.listening().await.unwrap();

        (handle, server_task, addr)
    }

    async fn connect(addr: SocketAddr) -> (SendRequest<Body>, JoinHandle<()>) {
        let stream = TcpStream::connect(addr).await.unwrap();
        let tls_stream = tls_connector(dns_name(), stream).await;

        let (send_request, connection) = handshake(tls_stream).await.unwrap();

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

    async fn tls_connector(hostname: &str, stream: TcpStream) -> SslStream<TcpStream> {
        let mut tls_parms = SslConnector::builder(SslMethod::tls_client()).unwrap();
        tls_parms.set_verify(SslVerifyMode::NONE);
        let hostname_owned = hostname.to_string();
        tls_parms.set_client_hello_callback(move |ssl_ref, _ssl_alert| {
            ssl_ref
                .set_hostname(hostname_owned.as_str())
                .map(|()| openssl::ssl::ClientHelloResponse::SUCCESS)
        });
        let tls_parms = tls_parms.build();

        let ssl = Ssl::new(tls_parms.context()).unwrap();
        let mut tls_stream = SslStream::new(ssl, stream).unwrap();

        SslStream::connect(Pin::new(&mut tls_stream)).await.unwrap();

        tls_stream
    }

    fn dns_name() -> &'static str {
        "localhost"
    }
}
