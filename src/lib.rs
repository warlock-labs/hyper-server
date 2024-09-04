use std::{convert::Infallible, net::SocketAddr};

use http_body_util::Full;
use hyper::{
    body::{Bytes, Incoming},
    Request,
    Response, server::conn::http1,
};
use hyper_util::{rt::TokioIo, service::TowerToHyperService};
use tokio::net::TcpListener;
use tower::{Service, ServiceBuilder};

// Define just a simple hello world endpoint that always responds with "Hello, World!"
// and is some sort of really stripped down request/response handler.
async fn hello(_: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    Ok(Response::new(Full::new(Bytes::from("Hello, World!"))))
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 127.0.0.1:3000
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    // Bind 127.0.0.1:3000
    let listener = TcpListener::bind(addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        tokio::spawn(async move {
            // This is an implementation of a tower service
            // Here you might use a tower "service of services" to route requests
            let svc = tower::service_fn(hello);
            // Wrap it in a service builder
            let svc = ServiceBuilder::new().service(svc);
            // Convert it to hyper service
            let svc = TowerToHyperService::new(svc);
            // Serve http1 connections
            if let Err(err) = http1::Builder::new().serve_connection(io, svc).await {
                eprintln!("server error: {}", err);
            }
        });
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run().await
}
