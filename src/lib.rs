// Crate links:
// - tokio: https://docs.rs/tokio/latest/tokio/
// - rustls: https://docs.rs/rustls/latest/rustls/
// - tokio_rustls:https://docs.rs/tokio-rustls/latest/tokio_rustls/
// - hyper: https://docs.rs/hyper/latest/hyper/
// - tower: https://docs.rs/tower/latest/tower/
//
// We take a `SocketAddr` to bind the server to a specific address.
// We use `TcpListener` to bind the server to the specified address at the TCP layer.
// We use `rustls` to create a new `ServerConfig` instance.
// We use `tokio_rustls` to create a new `TlsAcceptor` instance.
// We use `hyper_util::server::conn::auto` to create a new `Connection` instance.
// Which then passes requests to a `hyper::service::Service` instance.
// Which then can optionally pass requests to a `tower::Service` instance.
// Behind that can be axum, tower, tonic,
// or any other service that implements the `tower::Service` trait.

use std::fs;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use http::{Method, Request, Response, StatusCode};
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tower::{Service, ServiceBuilder};

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

pub struct HyperServer {}

impl HyperServer {
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
                if let Err(err) = Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(tls_stream), service)
                    .await
                {
                    eprintln!("failed to serve connection: {err:#}");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_echo_service() {}
}
