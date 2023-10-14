//! Future types for PROXY protocol support.
use crate::accept::Accept;
use crate::proxy_protocol::ForwardClientIp;
use pin_project_lite::pin_project;
use std::{
    fmt,
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Timeout;

// A `pin_project` is a procedural macro used for safe field projection in conjunction
// with the Rust Pin API, which guarantees that certain types will not move in memory.
pin_project! {
    /// This struct represents the future for the ProxyProtocolAcceptor.
    /// The generic types are:
    /// F: The future type.
    /// A: The type that implements the Accept trait.
    /// I: The IO type that supports both AsyncRead and AsyncWrite.
    /// S: The service type.
    pub struct ProxyProtocolAcceptorFuture<F, A, I, S>
    where
        A: Accept<I, S>,
    {
        #[pin]
        inner: AcceptFuture<F, A, I, S>,
    }
}

impl<F, A, I, S> ProxyProtocolAcceptorFuture<F, A, I, S>
where
    A: Accept<I, S>,
    I: AsyncRead + AsyncWrite + Unpin,
{
    // Constructor for creating a new ProxyProtocolAcceptorFuture.
    pub(crate) fn new(future: Timeout<F>, acceptor: A, service: S) -> Self {
        let inner = AcceptFuture::ReadHeader {
            future,
            acceptor,
            service: Some(service),
        };
        Self { inner }
    }
}

// Implement Debug trait for ProxyProtocolAcceptorFuture to allow
// debugging and logging.
impl<F, A, I, S> fmt::Debug for ProxyProtocolAcceptorFuture<F, A, I, S>
where
    A: Accept<I, S>,
    I: AsyncRead + AsyncWrite + Unpin,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyProtocolAcceptorFuture").finish()
    }
}

pin_project! {
    // AcceptFuture represents the internal states of ProxyProtocolAcceptorFuture.
    // It can either be waiting to read the header or forward the client IP.
    #[project = AcceptFutureProj]
    enum AcceptFuture<F, A, I, S>
    where
        A: Accept<I, S>,
    {
        ReadHeader {
            #[pin]
            future: Timeout<F>,
            acceptor: A,
            service: Option<S>,
        },
        ForwardIp {
            #[pin]
            future: A::Future,
            client_address: Option<SocketAddr>,
        },
    }
}

impl<F, A, I, S> Future for ProxyProtocolAcceptorFuture<F, A, I, S>
where
    A: Accept<I, S>,
    I: AsyncRead + AsyncWrite + Unpin,
    // Future whose output is a result with either a tuple of stream and optional address,
    // or an io::Error.
    F: Future<Output = Result<(I, Option<SocketAddr>), io::Error>>,
{
    type Output = io::Result<(A::Stream, ForwardClientIp<A::Service>)>;

    // The main poll function that drives the future towards completion.
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            // Check the current state of the inner future.
            match this.inner.as_mut().project() {
                AcceptFutureProj::ReadHeader {
                    future,
                    acceptor,
                    service,
                } => match future.poll(cx) {
                    Poll::Ready(Ok(Ok((stream, client_address)))) => {
                        let service = service.take().expect("future polled after ready");
                        let future = acceptor.accept(stream, service);

                        // Transition to the ForwardIp state after successfully reading the header.
                        this.inner.set(AcceptFuture::ForwardIp {
                            future,
                            client_address,
                        });
                    }
                    Poll::Ready(Ok(Err(e))) => return Poll::Ready(Err(e)),
                    Poll::Ready(Err(timeout)) => {
                        return Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, timeout)))
                    }
                    Poll::Pending => return Poll::Pending,
                },
                AcceptFutureProj::ForwardIp {
                    future,
                    client_address,
                } => {
                    return match future.poll(cx) {
                        Poll::Ready(Ok((stream, service))) => {
                            let service = ForwardClientIp {
                                inner: service,
                                client_address: *client_address,
                            };

                            // Return the successfully processed stream and service.
                            Poll::Ready(Ok((stream, service)))
                        }
                        Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                        Poll::Pending => Poll::Pending,
                    };
                }
            }
        }
    }
}
