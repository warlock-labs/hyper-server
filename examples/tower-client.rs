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
use std::{convert::Infallible, io, net::SocketAddr};
use std::io::{Error, ErrorKind, Write};
use futures_util::TryFutureExt;
use http_body_util::{Empty, Full,BodyExt};
use hyper::body::Body;
use tokio::net::{TcpListener, TcpStream};
use tower::{BoxError, ServiceBuilder};


/// Converts any error into an `io::Error` of kind `Other`.
///
/// This function can be used to create a uniform `io::Error` response for various error types.
///
/// # Arguments
///
/// - `error`: The error to be converted.
///
/// # Returns
///
/// Returns an `io::Error` with the kind set to `Other` and the provided error as its cause.
///
pub(crate) fn io_other<E: Into<BoxError>>(error: E) -> io::Error {
    io::Error::new(ErrorKind::Other, error)
}

async fn fetch_url (url: hyper::Uri) -> Result<(), std::io::Error> {
    let host = url.host().expect("uri has no host");
    let port = url.port_u16().unwrap_or(80);
    let addr = format!("{}:{}", host, port);
    let stream = TcpStream::connect(addr).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await
        .map_err(io_other)?;
    tokio::task::spawn(async move {
        if let Err(err) = conn.await {
            println!("Connection failed: {:?}", err);
        }
    });

    let authority = url.authority().unwrap().clone();

    let path = url.path();
    let req = Request::builder()
        .uri(path)
        .header(hyper::header::HOST, authority.as_str())
        .body(Empty::<Bytes>::new())
        .map_err(io_other)?;

    let mut res = sender.send_request(req).await
        .map_err(io_other)?;

    println!("Response: {}", res.status());
    println!("Headers: {:#?}\n", res.headers());

    // Stream the body, writing each chunk to stdout as we get it
    // (instead of buffering and printing at the end).
    while let Some(next) = res.frame().await {
        let frame = next.map_err(io_other)?;
        if let Some(chunk) = frame.data_ref() {
            io::stdout().write_all(&chunk)?;
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
    let url = "http://127.0.0.1:54321".parse::<hyper::Uri>().unwrap();
    fetch_url(url).await
}