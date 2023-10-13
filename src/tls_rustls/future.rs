//! Module containing futures specific to the `rustls` TLS acceptor for the server.
//!
//! This module primarily provides the `RustlsAcceptorFuture` which is responsible for performing the TLS handshake
//! using the `rustls` library.

use crate::tls_rustls::RustlsConfig;
use pin_project_lite::pin_project;
use std::io::{Error, ErrorKind};
use std::time::Duration;
use std::{
    fmt,
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::{timeout, Timeout};
use tokio_rustls::{server::TlsStream, Accept, TlsAcceptor};

pin_project! {
    /// A future representing the asynchronous TLS handshake using the `rustls` library.
    ///
    /// Once completed, it yields a `TlsStream` which is a wrapper around the actual underlying stream, with
    /// encryption and decryption operations applied to it.
    pub struct RustlsAcceptorFuture<F, I, S> {
        #[pin]
        inner: AcceptFuture<F, I, S>,
        config: Option<RustlsConfig>,
    }
}

impl<F, I, S> RustlsAcceptorFuture<F, I, S> {
    /// Constructs a new `RustlsAcceptorFuture`.
    ///
    /// * `future`: The future that resolves to the original non-encrypted stream.
    /// * `config`: The rustls configuration to use for the handshake.
    /// * `handshake_timeout`: The maximum duration to wait for the handshake to complete.
    pub(crate) fn new(future: F, config: RustlsConfig, handshake_timeout: Duration) -> Self {
        let inner = AcceptFuture::Inner {
            future,
            handshake_timeout,
        };
        let config = Some(config);

        Self { inner, config }
    }
}

impl<F, I, S> fmt::Debug for RustlsAcceptorFuture<F, I, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RustlsAcceptorFuture").finish()
    }
}

pin_project! {
    /// Internal states of the handshake process.
    #[project = AcceptFutureProj]
    enum AcceptFuture<F, I, S> {
        /// Initial state where we have a future that resolves to the original non-encrypted stream.
        Inner {
            #[pin]
            future: F,
            handshake_timeout: Duration,
        },
        /// State after receiving the stream where the handshake is performed asynchronously.
        Accept {
            #[pin]
            future: Timeout<Accept<I>>,
            service: Option<S>,
        },
    }
}

impl<F, I, S> Future for RustlsAcceptorFuture<F, I, S>
where
    F: Future<Output = io::Result<(I, S)>>,
    I: AsyncRead + AsyncWrite + Unpin,
{
    type Output = io::Result<(TlsStream<I>, S)>;

    /// Advances the handshake state machine.
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            match this.inner.as_mut().project() {
                AcceptFutureProj::Inner {
                    future,
                    handshake_timeout,
                } => {
                    // Poll the future to get the original stream.
                    match future.poll(cx) {
                        Poll::Ready(Ok((stream, service))) => {
                            let server_config = this.config
                                .take()
                                .expect("config is not set. this is a bug in hyper-server, please report")
                                .get_inner();

                            let acceptor = TlsAcceptor::from(server_config);
                            let future = acceptor.accept(stream);

                            let service = Some(service);
                            let handshake_timeout = *handshake_timeout;

                            this.inner.set(AcceptFuture::Accept {
                                future: timeout(handshake_timeout, future),
                                service,
                            });
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => return Poll::Pending,
                    }
                }
                AcceptFutureProj::Accept { future, service } => match future.poll(cx) {
                    Poll::Ready(Ok(Ok(stream))) => {
                        let service = service.take().expect("future polled after ready");

                        return Poll::Ready(Ok((stream, service)));
                    }
                    Poll::Ready(Ok(Err(e))) => return Poll::Ready(Err(e)),
                    Poll::Ready(Err(timeout)) => {
                        return Poll::Ready(Err(Error::new(ErrorKind::TimedOut, timeout)))
                    }
                    Poll::Pending => return Poll::Pending,
                },
            }
        }
    }
}
