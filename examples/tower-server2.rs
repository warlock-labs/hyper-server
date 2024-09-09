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
use http_body_util::{Full, BodyExt};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower::{Layer, Service as TowerService};
use tower::layer::util::Stack;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;


pub struct ServerBuilder<L> {
    layers: L,
}

pub struct Server<S> {
    service: S,
}

impl ServerBuilder<tower::layer::util::Identity> {
    pub fn new() -> Self {
        ServerBuilder {
            layers: tower::layer::util::Identity::new(),
        }
    }
}

impl<L> ServerBuilder<L> {
    pub fn layer<NewLayer>(self, layer: NewLayer) -> ServerBuilder<Stack<NewLayer, L>> {
        ServerBuilder {
            layers: Stack::new(layer, self.layers),
        }
    }

    pub fn service<S>(self, service: S) -> Server<L::Service>
    where
        L: Layer<S> + Send + Sync + 'static,
        S: TowerService<Request<Incoming>, Response = Response<BoxBody>, Error = hyper::Error> + Clone + Send + 'static,
        S::Future: Send + 'static,
        L::Service: TowerService<Request<Incoming>, Response = Response<BoxBody>, Error = hyper::Error> + Clone + Send + 'static,
        <L::Service as TowerService<Request<Incoming>>>::Future: Send + 'static,
    {
        Server {
            service: self.layers.layer(service),
        }
    }
}

impl<S> Server<S>
where
    S: TowerService<Request<Incoming>, Response = Response<BoxBody>, Error = hyper::Error> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
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
                    let service = self.service.clone();

                    let svc = TowerToHyperService::new(service);
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
pub async fn hello(_: Request<Incoming>) -> Result<Response<BoxBody>, hyper::Error> {
    Ok(Response::new(
        Full::new(Bytes::from("Hello, World!"))
            .map_err(|never| match never {})
            .boxed()
    ))
}


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 54321));
    let svc = tower::service_fn(hello);

    ServerBuilder::new()
        .layer(tower::limit::ConcurrencyLimitLayer::new(64))
        .service(svc)
        .serve(addr)
        .await?;

    Ok(())
}
