use crate::Error;
use std::{io, ops::ControlFlow};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_stream::{Stream, StreamExt};
use tracing::debug;

/// Handles errors that occur during TCP connection acceptance.
///
/// This function determines whether an error should be treated as fatal (breaking the accept loop)
/// or non-fatal (allowing the loop to continue).
///
/// # Arguments
///
/// * `e` - The error to handle, which can be converted into the crate's `Error` type.
///
/// # Returns
///
/// * `ControlFlow::Continue(())` if the error is non-fatal and the accept loop should continue.
/// * `ControlFlow::Break(Error)` if the error is fatal and the accept loop should terminate.
///
/// # Error Handling
///
/// The function categorizes errors as follows:
/// - Non-fatal errors: ConnectionAborted, Interrupted, InvalidData, WouldBlock
/// - Fatal errors: All other error types
fn handle_accept_error(e: impl Into<Error>) -> ControlFlow<Error> {
    let e = e.into();
    debug!(error = %e, "TCP accept loop error");

    // Check if the error is an I/O error
    if let Some(e) = e.downcast_ref::<io::Error>() {
        // Determine if the error is non-fatal
        if matches!(
            e.kind(),
            io::ErrorKind::ConnectionAborted
                | io::ErrorKind::Interrupted
                | io::ErrorKind::InvalidData
                | io::ErrorKind::WouldBlock
        ) {
            return ControlFlow::Continue(());
        }
    }

    // If not a non-fatal I/O error, treat as fatal
    ControlFlow::Break(e)
}

/// Creates a stream that yields a TCP stream for each incoming connection.
///
/// This function takes a stream of incoming connections and handles errors that may occur
/// during the acceptance process. It will continue to yield connections even if non-fatal
/// errors occur, but will terminate if a fatal error is encountered.
///
/// # Type Parameters
///
/// * `IO`: The type of the I/O object yielded by the incoming stream.
/// * `IE`: The type of the error that can be produced by the incoming stream.
///
/// # Arguments
///
/// * `incoming`: A stream that yields results of incoming connection attempts.
///
/// # Returns
///
/// A stream that yields `Result<IO, Error>` for each incoming connection.
///
/// # Error Handling
///
/// This function uses `handle_accept_error` to determine whether to continue accepting
/// connections after an error occurs. Non-fatal errors are logged and skipped, while
/// fatal errors cause the stream to yield an error and terminate.
pub(crate) fn serve_tcp_incoming<IO, IE>(
    incoming: impl Stream<Item = Result<IO, IE>> + Send + 'static,
) -> impl Stream<Item = Result<IO, crate::Error>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    IE: Into<Error> + Send + 'static,
{
    async_stream::stream! {
        let mut incoming = Box::pin(incoming);

        while let Some(item) = incoming.next().await {
            match item {
                Ok(io) => yield Ok(io),
                Err(e) => match handle_accept_error(e.into()) {
                    ControlFlow::Continue(()) => continue,
                    ControlFlow::Break(e) => yield Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::pin::Pin;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_stream::wrappers::TcpListenerStream;
    use tokio_stream::StreamExt;

    #[tokio::test]
    async fn test_handle_accept_error() {
        // Test non-fatal errors
        let non_fatal_errors = vec![
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::Interrupted,
            io::ErrorKind::InvalidData,
            io::ErrorKind::WouldBlock,
        ];

        for kind in non_fatal_errors {
            let error = io::Error::new(kind, "Test error");
            assert!(matches!(
                handle_accept_error(error),
                ControlFlow::Continue(())
            ));
        }

        // Test fatal error
        let fatal_error = io::Error::new(io::ErrorKind::PermissionDenied, "Permission denied");
        assert!(matches!(
            handle_accept_error(fatal_error),
            ControlFlow::Break(_)
        ));
    }

    #[tokio::test]
    async fn test_serve_tcp_incoming_success() -> Result<(), Box<dyn std::error::Error>> {
        // Set up a TCP listener
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = TcpListener::bind(addr).await?;
        let bound_addr = listener.local_addr()?;
        let stream = TcpListenerStream::new(listener);
        let mut incoming = Box::pin(serve_tcp_incoming(stream));

        // Spawn a task to accept one connection
        let accept_task = tokio::spawn(async move { incoming.next().await });

        // Connect to the server
        let _client = TcpStream::connect(bound_addr).await?;

        // Check that the connection was accepted
        let result = accept_task.await?.unwrap();
        assert!(result.is_ok());

        Ok(())
    }

    #[tokio::test]
    async fn test_serve_tcp_incoming_with_errors() {
        // Create a mock stream that yields both successful connections and errors
        let mock_stream = tokio_stream::iter(vec![
            Ok(MockIO),
            Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Aborted")),
            Ok(MockIO),
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Permission denied",
            )),
        ]);

        let mut incoming = Box::pin(serve_tcp_incoming(mock_stream));

        // First connection should be successful
        assert!(incoming.next().await.unwrap().is_ok());

        // Second connection (aborted) should be skipped
        // Third connection should be successful
        assert!(incoming.next().await.unwrap().is_ok());

        // Fourth connection (permission denied) should break the stream
        assert!(incoming.next().await.unwrap().is_err());

        // Stream should be exhausted
        assert!(incoming.next().await.is_none());
    }

    // Mock IO type for testing
    struct MockIO;

    impl AsyncRead for MockIO {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for MockIO {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<Result<usize, io::Error>> {
            std::task::Poll::Ready(Ok(0))
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }
}
