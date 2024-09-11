use std::{fs, io};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use crate::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsAcceptor;
use tokio_stream::{Stream, StreamExt};

/// Creates a stream of TLS-encrypted connections from a stream of TCP connections.
///
/// This function takes a stream of TCP connections and a TLS acceptor, and produces
/// a new stream that yields TLS-encrypted connections. It handles both the successful
/// case of establishing a TLS connection and the error cases.
///
/// # Type Parameters
///
/// * `IO`: The I/O type representing the underlying TCP connection. It must implement
///   `AsyncRead`, `AsyncWrite`, `Unpin`, `Send`, and have a static lifetime.
///
/// # Arguments
///
/// * `tcp_stream`: A stream that yields `Result<IO, Error>` items, representing incoming
///   TCP connections or errors.
/// * `tls`: A `TlsAcceptor` used to perform the TLS handshake on each TCP connection.
///
/// # Returns
///
/// A new `Stream` that yields `Result<tokio_rustls::server::TlsStream<IO>, Error>` items.
/// Each item is either a successfully established TLS connection or an error.
///
/// # Error Handling
///
/// - If the input `tcp_stream` yields an error, that error is propagated.
/// - If the TLS handshake fails, the error is wrapped in the crate's `Error` type.
pub(crate) fn tls_incoming<IO>(
    tcp_stream: impl Stream<Item = Result<IO, Error>>,
    tls: TlsAcceptor,
) -> impl Stream<Item = Result<tokio_rustls::server::TlsStream<IO>, Error>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Transform each item in the TCP stream into a TLS stream
    tcp_stream.then(move |result| {
        // Clone the TLS acceptor for each connection
        let tls = tls.clone();

        async move {
            match result {
                // TODO(Can we get at the raw IO here so that it looks the same after the handshake?)
                Ok(io) => tls.accept(io).await.map_err(Error::from),
                // TODO(Unwrap into crate error and handle)
                Err(e) => Err(e),
            }
        }
    })
}

// Load the public certificate from a file.
fn load_certs(filename: &str) -> io::Result<Vec<CertificateDer<'static>>> {
    // Open certificate file.
    let certfile = fs::File::open(filename).unwrap();
    let mut reader = io::BufReader::new(certfile);

    // Load and return certificate.
    rustls_pemfile::certs(&mut reader).collect()
}

// Load the private key from a file.
fn load_private_key(filename: &str) -> io::Result<PrivateKeyDer<'static>> {
    // Open keyfile.
    let keyfile = fs::File::open(filename).unwrap();
    let mut reader = io::BufReader::new(keyfile);

    // Load and return a single private key.
    rustls_pemfile::private_key(&mut reader).map(|key| key.unwrap())
}