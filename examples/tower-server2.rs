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
use hyper::service::Service;
use tokio::net::TcpListener;

use hyper::service::Service as HyperService;
use tower::{Layer, Service as TowerService};

type BoxBody = http_body_util::combinators::BoxBody<hyper::body::Bytes, hyper::Error>;

pub struct Server<S> {
    service: S,
}

impl Server<()> {
    pub fn new() -> Self {
        Server { service: () }
    }
}

impl<S> Server<S>
where
    S: TowerService<Request<Incoming>, Response = Response<BoxBody>, Error = hyper::Error> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    pub fn layer<L>(self, layer: L) -> Server<L::Service>
    where
        L: Layer<S> + Send + Sync + 'static,
        L::Service: TowerService<Request<Incoming>, Response = Response<BoxBody>, Error = hyper::Error> + Clone + Send + 'static,
        <L::Service as TowerService<Request<Incoming>>>::Future: Send + 'static,
    {
        Server {
            service: layer.layer(self.service),
        }
    }

    pub fn service<NewS>(self, service: NewS) -> Server<NewS>
    where
        NewS: TowerService<Request<Incoming>, Response = Response<BoxBody>, Error = hyper::Error> + Clone + Send + 'static,
        NewS::Future: Send + 'static,
    {
        Server { service }
    }

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
