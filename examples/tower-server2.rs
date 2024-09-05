//! This module implements an HTTP server with graceful shutdown capabilities.
//! It demonstrates how to handle incoming connections, serve a basic "Hello, World!"
//! response, and gracefully shut down the server when a termination signal is received.

use hyper::{
    body::{Bytes, Incoming},
    server::conn::http1,
    Request, Response,
};
use tower::layer::util::Identity;
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use http_body_util::Full;
use std::{convert::Infallible, net::SocketAddr};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use http_body::Body;
use tokio::net::TcpListener;
use tower_service::Service;
use hyper_server::CompositeService;

// Type alias for the complex error type
type BoxError = Box<dyn std::error::Error + Send + Sync>;

// Type alias for the future returned by the service
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

// Type alias for the specific service signature
type SpecificService = dyn Service<
    Request<Incoming>,
    Response = Response<Full<Bytes>>,
    Error = BoxError,
    Future = BoxFuture<Result<Response<Full<Bytes>>, BoxError>>> + Send + Sync;



/// Represents our HTTP server.
pub struct Server {
    /// The socket address on which the server will listen.
    socket_addr: SocketAddr,
    services: CompositeService<Box<SpecificService>, Request<Incoming>>,
}

impl Server {
    /// Creates a new Server instance.
    ///
    /// # Arguments
    ///
    /// * `addr` - The socket address on which the server will listen.
    pub async fn new(addr: SocketAddr) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Server {
            socket_addr: addr,
            services: CompositeService::new(),
        })
    }

    /// Add a tower service to the server
    pub fn add_service<S>(&mut self, service: S)
    where
        S: Service<
            Request<Incoming>,
            Response = Response<Full<Bytes>>,
            Error = BoxError,
            Future = BoxFuture<Result<Response<Full<Bytes>>, BoxError>>
        > + Send + Sync + 'static
    {
        self.services.push(Box::new(service) as Box<SpecificService>);
    }


    /// Starts the server and handles incoming connections.
    ///
    /// This method sets up the TCP listener, initializes the HTTP server,
    /// and enters the main service loop. It handles incoming connections
    /// and manages the graceful shutdown process.
    pub async fn serve(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Bind to the specified address
        let listener = TcpListener::bind(self.socket_addr).await?;

        // Specify our HTTP settings (http1, http2, auto all work)
        let http = http1::Builder::new();

        // Initialize the graceful shutdown mechanism
        let graceful = hyper_util::server::graceful::GracefulShutdown::new();

        // Prepare the shutdown signal future
        let mut signal = std::pin::pin!(Server::shutdown_signal());

        println!("Server listening on {}", self.socket_addr);

        // Main server loop
        let services = self.services.clone();
        loop {
            tokio::select! {
                // Handle incoming connections
                Ok((stream, _addr)) = listener.accept() => {
                    let io = TokioIo::new(stream);

                    // Create a new service for each connection
                    let conn = http.serve_connection(io, services);
                    // Watch the connection for graceful shutdown
                    let fut = graceful.watch(conn);

                    // Spawn a new task for each connection, watching for closure
                    tokio::spawn(async move {
                        if let Err(e) = fut.await {
                            eprintln!("Error serving connection: {:?}", e);
                        }
                    });
                },
                // Handle shutdown signal
                _ = &mut signal => {
                    eprintln!("Graceful shutdown signal received");
                    // Stop the accept loop
                    break;
                }
            }
        }

        // Graceful shutdown process
        tokio::select! {
            _ = graceful.shutdown() => {
                eprintln!("All connections gracefully closed");
                Ok(())
            },
            _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {
                eprintln!("Timed out waiting for all connections to close");
                Err(Box::from("Timed out waiting for connections to close"))
            }
        }
    }

    /// Waits for a CTRL+C signal to initiate the shutdown process.
    ///
    /// This function uses tokio's signal handling to wait for a CTRL+C signal,
    /// which will trigger the graceful shutdown of our server.
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
    let mut server = Server::new(addr).await?;

    // Create a tower service
    let svc = tower::service_fn(hello);
    server.add_service(Arc::new(svc));

    // Start the server in a separate task
    let server_task = tokio::spawn(async move {
        server.serve().await
    });

    // Wait for the server to shut down
    server_task.await?
}