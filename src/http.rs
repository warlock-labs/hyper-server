use http::{Request, Response};
use http_body::Body;
use hyper::body::Incoming;
use hyper::service::Service;
use hyper_util::server::conn::auto::{Builder, HttpServerConnExec};
use std::future::pending;
use std::pin::pin;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, trace};

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
    builder: Builder<E>,
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
