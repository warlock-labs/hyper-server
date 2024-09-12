use crate::Error;
use std::{io, ops::ControlFlow};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_stream::{Stream, StreamExt};
use tracing::debug;

/// Handles errors that occur during TCP connection acceptance.
///
/// This function determines whether an error should be treated as fatal (breaking the accept loop)
/// or non-fatal (allowing the loop to continue). This handler is crucial for maintaining the
/// stability and reliability of the TCP server by appropriately handling different types of
/// errors at the accept stage.
///
/// # Arguments
///
/// * `e` - The error to handle, which can be converted into the crate's [`Error`] type.
///
/// # Returns
///
/// * [`ControlFlow::Continue(())`] if the error is non-fatal and the accept loop should continue.
/// * [`ControlFlow::Break(Error)`] if the error is fatal and the accept loop should terminate.
///
/// # Error Handling
///
/// The function categorizes errors as follows:
/// - Non-fatal errors:
///       [`io::ErrorKind::ConnectionAborted`],
///       [`io::ErrorKind::Interrupted`],
///       [`io::ErrorKind::InvalidData`],
///       [`io::ErrorKind::WouldBlock`]
/// - Fatal errors: All other error types
///
fn handle_accept_error(e: impl Into<Error>) -> ControlFlow<Error> {
    // Convert the input error into our crate's Error type
    let e = e.into();

    // Log the error for debugging purposes
    debug!(error = %e, "TCP accept loop error");

    // Check if the error is an I/O error
    if let Some(e) = e.downcast_ref::<io::Error>() {
        // Determine if the error is non-fatal
        if matches!(
            e.kind(),
            io::ErrorKind::ConnectionAborted  // Connection was aborted by the client or network
                | io::ErrorKind::Interrupted  // The operation was interrupted (e.g., by a signal)
                | io::ErrorKind::InvalidData  // Received invalid data (might be temporary)
                | io::ErrorKind::WouldBlock // Operation would block (common in non-blocking I/O)
        ) {
            // For non-fatal errors, we continue the accept loop
            return ControlFlow::Continue(());
        }
    }

    // If it's not a non-fatal I/O error, then treat it as fatal error
    // This includes all other error types and I/O errors not explicitly handled above
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
/// * [`IO`]: The type of the I/O object yielded by the incoming stream,
///           usually a tokio IO.
/// * [`IE`]: The type of the error that can be produced by the incoming stream,
///           usually a Tokio error.
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
///
/// # Examples
///
/// ```rust,no_run
/// use std::net::SocketAddr;
/// use tokio::net::TcpListener;
/// use tokio_stream::wrappers::TcpListenerStream;
/// use hyper_server::serve_tcp_incoming;
///
/// #[tokio::main]
/// async fn main() {
///     let addr = SocketAddr::from(([127, 0, 0, 1], 0));  // Using port 0 for dynamic port assignment
///     let listener = TcpListener::bind(addr).await.unwrap();
///     let incoming = TcpListenerStream::new(listener);
///     let tcp_stream = serve_tcp_incoming(incoming);
///
///     // Use the tcp_stream for further processing
/// }
/// ```
#[inline]
pub fn serve_tcp_incoming<IO, IE>(
    incoming: impl Stream<Item = Result<IO, IE>> + Send + 'static,
) -> impl Stream<Item = Result<IO, crate::Error>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    IE: Into<Error> + Send + 'static,
{
    async_stream::stream! {
        // Pin the incoming stream to the heap for better performance
        let mut incoming = Box::pin(incoming);

        // Continuously process incoming connections
        while let Some(item) = incoming.next().await {
            match item {
                // If the connection is successfully established, yield it
                Ok(io) => yield Ok(io),
                // Handle errors using the handle_accept_error function
                Err(e) => match handle_accept_error(e.into()) {
                    // For non-fatal errors, continue to the next incoming
                    // connection on the `IO` stream
                    ControlFlow::Continue(()) => continue,
                    // For fatal errors, yield the error and terminate
                    // the stream, breaking the loop and tearing down the
                    // server
                    ControlFlow::Break(e) => yield Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::iter;
    use std::io;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio_stream::Stream;

    // Mock IO struct for testing
    // This struct simulates an AsyncRead and AsyncWrite implementation
    // with controllable behavior for testing various scenarios
    struct MockIO {
        // We use Option here to allow consuming the result in poll_read and poll_write
        read_result: Option<io::Result<()>>,
        write_result: Option<io::Result<usize>>,
    }

    impl AsyncRead for MockIO {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            // Take the result, leaving None in its place
            Poll::Ready(self.read_result.take().unwrap_or(Ok(())))
        }
    }

    impl AsyncWrite for MockIO {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<io::Result<usize>> {
            // Take the result, leaving None in its place
            Poll::Ready(self.write_result.take().unwrap_or(Ok(0)))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl Unpin for MockIO {}

    // Helper function to create a stream of mock IOs
    // This function allows us to easily create test scenarios with predefined outcomes
    fn mock_incoming(
        results: Vec<Result<MockIO, io::Error>>,
    ) -> impl Stream<Item = Result<MockIO, io::Error>> {
        iter(results)
    }

    // Test that non-fatal errors are handled correctly by handle_accept_error
    #[tokio::test]
    async fn test_handle_accept_error_non_fatal() {
        let non_fatal_errors = vec![
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::Interrupted,
            io::ErrorKind::InvalidData,
            io::ErrorKind::WouldBlock,
        ];

        for kind in non_fatal_errors {
            let error = io::Error::new(kind, "Test non-fatal error");
            assert!(matches!(
                handle_accept_error(error),
                ControlFlow::Continue(())
            ));
        }
    }

    // Test that fatal errors are handled correctly by handle_accept_error
    #[tokio::test]
    async fn test_handle_accept_error_fatal() {
        let fatal_error = io::Error::new(io::ErrorKind::PermissionDenied, "Test fatal error");
        assert!(matches!(
            handle_accept_error(fatal_error),
            ControlFlow::Break(_)
        ));
    }

    // Test successful TCP connection acceptance
    #[tokio::test]
    async fn test_serve_tcp_incoming_success() {
        let mock_io = MockIO {
            read_result: Some(Ok(())),
            write_result: Some(Ok(5)),
        };
        let incoming = mock_incoming(vec![Ok(mock_io)]);
        let mut stream = Box::pin(serve_tcp_incoming(incoming));

        if let Some(result) = stream.next().await {
            assert!(result.is_ok());
        } else {
            panic!("Stream ended unexpectedly");
        }
    }

    // Test handling of non-fatal errors during connection acceptance
    #[tokio::test]
    async fn test_serve_tcp_incoming_non_fatal_error() {
        let non_fatal_error = io::Error::new(io::ErrorKind::WouldBlock, "Would block");
        let incoming = mock_incoming(vec![
            Err(non_fatal_error),
            Ok(MockIO {
                read_result: Some(Ok(())),
                write_result: Some(Ok(5)),
            }),
        ]);
        let mut stream = Box::pin(serve_tcp_incoming(incoming));

        if let Some(result) = stream.next().await {
            assert!(result.is_ok());
        } else {
            panic!("Stream ended unexpectedly");
        }
    }

    // Test handling of fatal errors during connection acceptance
    #[tokio::test]
    async fn test_serve_tcp_incoming_fatal_error() {
        let fatal_error = io::Error::new(io::ErrorKind::PermissionDenied, "Permission denied");
        let incoming = mock_incoming(vec![Err(fatal_error)]);
        let mut stream = Box::pin(serve_tcp_incoming(incoming));

        if let Some(result) = stream.next().await {
            assert!(result.is_err());
        } else {
            panic!("Stream ended unexpectedly");
        }

        // Stream should be terminated after a fatal error
        assert!(stream.next().await.is_none());
    }

    // Test a mix of successful connections, non-fatal errors, and fatal errors
    #[tokio::test]
    async fn test_serve_tcp_incoming_mixed_results() {
        let incoming = mock_incoming(vec![
            Ok(MockIO {
                read_result: Some(Ok(())),
                write_result: Some(Ok(5)),
            }),
            Err(io::Error::new(io::ErrorKind::WouldBlock, "Would block")),
            Ok(MockIO {
                read_result: Some(Ok(())),
                write_result: Some(Ok(3)),
            }),
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Permission denied",
            )),
        ]);
        let mut stream = Box::pin(serve_tcp_incoming(incoming));

        // First successful connection
        assert!(stream.next().await.unwrap().is_ok());

        // Second successful connection (after skipping non-fatal error)
        assert!(stream.next().await.unwrap().is_ok());

        // Fatal error
        assert!(stream.next().await.unwrap().is_err());

        // Stream should be terminated after a fatal error
        assert!(stream.next().await.is_none());
    }

    // Test behavior with an empty incoming stream
    #[tokio::test]
    async fn test_serve_tcp_incoming_empty_stream() {
        let incoming = mock_incoming(vec![]);
        let mut stream = Box::pin(serve_tcp_incoming(incoming));

        assert!(stream.next().await.is_none());
    }
}
