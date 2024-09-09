//! This module implements an HTTP server with graceful shutdown capabilities.
//! It demonstrates how to handle incoming connections, serve a basic "Hello, World!"
//! response, and gracefully shut down the server when a termination signal is received.

use hyper::{
    body::{Bytes, Incoming},
    server::conn::http1,
    Request, Response,
};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use http_body_util::Full;
use std::{convert::Infallible, future, net::SocketAddr};
use std::sync::Arc;
use hyper::service::Service;
use tokio::net::TcpListener;
use tower::layer::util::{Identity, Stack};
use tower::{Layer, ServiceBuilder};
use tower::util::BoxService;

type BoxBody = http_body_util::combinators::BoxBody<hyper::body::Bytes, hyper::Error>;


pub struct Server<L = tower::layer::util::Identity> {
    /// The layer stack that will be applied to each service
    builder: Arc<L>,
}

impl Default for Server<tower::layer::util::Identity> {
    fn default() -> Self {
        Self {
            builder: Arc::new(tower::layer::util::Identity::new()),
        }
    }
}

impl Server {
    fn new() -> Self {
        Server::default()
    }
}

impl<L> Server<L>
where
    L: Layer<BoxService<hyper::Request<hyper::body::Incoming>, hyper::Response<BoxBody>, hyper::Error>> + Send + Sync + 'static,
    L::Service: Send + 'static,
{
    /// Add layer
    pub fn layer<NewLayer>(self, layer: NewLayer) -> Server<Stack<NewLayer, L>>
    where
        NewLayer: Layer<BoxService<hyper::Request<hyper::body::Incoming>, hyper::Response<BoxBody>, hyper::Error>> + Send + Sync + 'static,
    {
        Server {
            builder: Arc::new(ServiceBuilder::new().layer(layer).chain(Arc::try_unwrap(self.builder).unwrap_or_else(|arc| (*arc).clone()))),
        }
    }

    /// Add service
    pub fn service<S>(self, svc: S) -> Self
    where
        S: Service<hyper::Request<hyper::body::Incoming>, Response = hyper::Response<BoxBody>, Error = hyper::Error> + Clone + Send + 'static,
        S::Future: Send + 'static,
    {
        let builder = self.builder.clone();
        Server {
            builder: Arc::new(ServiceBuilder::new().service_fn(move |req| {
                let svc = builder.service(svc.clone());
                svc.call(req)
            })),
        }
    }


    /// Starts the server and handles incoming connections.
    /// Starts the server and handles incoming connections.
    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(addr).await?;
        let http = http1::Builder::new();
        let graceful = hyper_util::server::graceful::GracefulShutdown::new();
        let mut signal = std::pin::pin!(Self::shutdown_signal());

        println!("Server listening on {}", addr);

        loop {
            tokio::select! {
                Ok((stream, _)) = listener.accept() => {
                    let io = TokioIo::new(stream);
                    let builder = self.builder.clone();

                    let svc = TowerToHyperService::new(self.builder.clone());
                    let conn = http.serve_connection(io, svc);
                    let fut = graceful.watch(conn);

                    tokio::spawn(async move {
                        if let Err(e) = fut.await {
                            eprintln!("Error serving connection: {:?}", e);
                        }
                    });
                },
                _ = &mut signal => {
                    eprintln!("Graceful shutdown signal received");
                    break;
                }
            }
        }

        tokio::select! {
            _ = graceful.shutdown() => {
                eprintln!("All connections gracefully closed");
                Ok(())
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                Err(Box::from("Timed out waiting for connections to close"))
            }
        }
    }

    async fn shutdown_signal() {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install CTRL+C signal handler");
    }
}


/// A simple handler that responds with "Hello, World!" for any incoming request.
///
/// This function serves as our basic request handler, demonstrating a minimal
/// HTTP service implementation.
pub async fn hello(_: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 54321));
    let svc = tower::service_fn(hello);

    Server::new()
        .service(svc)
        .serve(addr)
        .await?;

    Ok(())
}
