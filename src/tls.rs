use crate::Error;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::{fs, io};
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
        // This is necessary because the acceptor is moved into the async block
        let tls = tls.clone();

        async move {
            match result {
                // If the TCP connection was successfully established
                Ok(io) => {
                    // Attempt to perform the TLS handshake
                    // If successful, return the TLS stream; otherwise, wrap the error
                    tls.accept(io).await.map_err(Error::from)
                }
                // If there was an error establishing the TCP connection, propagate it
                Err(e) => Err(e),
            }
        }
    })
}

/// Load the public certificate from a file.
///
/// This function reads a PEM-encoded certificate file and returns a vector of
/// parsed certificates.
///
/// # Arguments
///
/// * `filename`: The path to the certificate file.
///
/// # Returns
///
/// A `Result` containing a vector of `CertificateDer` on success, or an `io::Error` on failure.
fn load_certs(filename: &str) -> io::Result<Vec<CertificateDer<'static>>> {
    // Open certificate file
    let certfile = fs::File::open(filename)?;
    let mut reader = io::BufReader::new(certfile);

    // Load and return certificates
    // The `collect()` method is used to gather all certificates into a vector
    rustls_pemfile::certs(&mut reader).collect()
}

/// Load the private key from a file.
///
/// This function reads a PEM-encoded private key file and returns the parsed private key.
///
/// # Arguments
///
/// * `filename`: The path to the private key file.
///
/// # Returns
///
/// A `Result` containing a `PrivateKeyDer` on success, or an `io::Error` on failure.
fn load_private_key(filename: &str) -> io::Result<PrivateKeyDer<'static>> {
    // Open keyfile
    let keyfile = fs::File::open(filename)?;
    let mut reader = io::BufReader::new(keyfile);

    // Load and return a single private key
    // The `?` operator is used for error propagation
    rustls_pemfile::private_key(&mut reader)?
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "No private key found in file"))
}
