//! Future types.
//!
//! This module provides the futures and supporting types for integrating OpenSSL with a hyper/tokio HTTP/TLS server.
//! `OpenSSLAcceptorFuture` is the main public-facing type which wraps around the core logic of establishing an SSL/TLS
//! connection.

use super::OpenSSLConfig;
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

use openssl::ssl::Ssl;
use tokio_openssl::SslStream;

// The OpenSSLAcceptorFuture encapsulates the asynchronous logic of accepting an SSL/TLS connection.
pin_project! {
    /// A Future for establishing an SSL/TLS connection using `OpenSSLAcceptor`.
    ///
    /// This wraps around the process of asynchronously establishing an SSL/TLS connection via OpenSSL.
    /// It waits for the inner non-TLS connection to be established, and then handles the TLS handshake.
    pub struct OpenSSLAcceptorFuture<F, I, S> {
        #[pin]
        inner: AcceptFuture<F, I, S>, // Inner future which manages the state machine of accepting connections.
        config: Option<OpenSSLConfig>, // The SSL/TLS configurati on to use for the handshake.
    }
}

impl<F, I, S> OpenSSLAcceptorFuture<F, I, S> {
    /// Constructs a new `OpenSSLAcceptorFuture`.
    ///
    /// # Arguments
    /// - `future`: The initial future that handles the non-TLS accept phase.
    /// - `config`: SSL/TLS configuration.
    /// - `handshake_timeout`: Maximum duration allowed for the TLS handshake.
    pub(crate) fn new(future: F, config: OpenSSLConfig, handshake_timeout: Duration) -> Self {
        let inner = AcceptFuture::InnerAccepting {
            future,
            handshake_timeout,
        };
        let config = Some(config);

        Self { inner, config }
    }
}

impl<F, I, S> fmt::Debug for OpenSSLAcceptorFuture<F, I, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenSSLAcceptorFuture").finish()
    }
}

// A future for performing the SSL/TLS handshake using an `SslStream`.
pin_project! {
    struct TlsAccept<I> {
        #[pin]
        tls_stream: Option<SslStream<I>>, // The SSL/TLS stream on which the handshake will be performed.
    }
}

impl<I> Future for TlsAccept<I>
where
    I: AsyncRead + AsyncWrite + Unpin, // The inner type must support asynchronous reading and writing.
{
    type Output = io::Result<SslStream<I>>; // The result will be an `SslStream` if the handshake is successful.

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        match this
            .tls_stream
            .as_mut()
            .as_pin_mut()
            .map(|inner| inner.poll_accept(cx))
            .expect("tlsaccept polled after ready")
        {
            Poll::Ready(Ok(())) => {
                let tls_stream = this.tls_stream.take().expect("tls stream vanished?");
                Poll::Ready(Ok(tls_stream))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(Error::new(ErrorKind::Other, e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

// Enumerates the possible states of the accept process, either waiting for the inner non-TLS
// connection to be accepted, or performing the TLS handshake.
pin_project! {
    #[project = AcceptFutureProj]
    enum AcceptFuture<F, I, S> {
        // Waiting for the non-TLS connection to be accepted.
        InnerAccepting {
            #[pin]
            future: F,               // The future representing the non-TLS accept phase.
            handshake_timeout: Duration, // Maximum duration for the TLS handshake.
        },
        // Performing the TLS handshake.
        TlsAccepting {
            #[pin]
            future: Timeout<TlsAccept<I>>, // Future that represents the TLS handshake, with a timeout.
            service: Option<S>,      // The underlying service that will handle the request after the TLS handshake.
        }
    }
}

// Main implementation of the future for `OpenSSLAcceptor`.
impl<F, I, S> Future for OpenSSLAcceptorFuture<F, I, S>
where
    F: Future<Output = io::Result<(I, S)>>, // The initial non-TLS accept future.
    I: AsyncRead + AsyncWrite + Unpin, // The inner type must support asynchronous reading and writing.
{
    type Output = io::Result<(SslStream<I>, S)>; // The output will be an `SslStream` and the service to handle the request.

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        // The inner future here is what is doing the lower level accept, such as
        // our tcp socket.
        //
        // So we poll on that first, when it's ready we then swap our the inner future to
        // one waiting for our ssl layer to accept/install.
        //
        // Then once that's ready we can then wrap and provide the SslStream back out.

        // This loop exists to allow the Poll::Ready from InnerAccept on complete
        // to re-poll immediately. Otherwise all other paths are immediate returns.
        loop {
            match this.inner.as_mut().project() {
                AcceptFutureProj::InnerAccepting {
                    future,
                    handshake_timeout,
                } => match future.poll(cx) {
                    Poll::Ready(Ok((stream, service))) => {
                        let server_config = this.config.take().expect(
                            "config is not set. this is a bug in hyper-server, please report",
                        );

                        // Change to poll::ready(err)
                        let ssl = Ssl::new(server_config.acceptor.context()).unwrap();

                        let tls_stream = SslStream::new(ssl, stream).unwrap();
                        let future = TlsAccept {
                            tls_stream: Some(tls_stream),
                        };

                        let service = Some(service);
                        let handshake_timeout = *handshake_timeout;

                        this.inner.set(AcceptFuture::TlsAccepting {
                            future: timeout(handshake_timeout, future),
                            service,
                        });
                        // the loop is now triggered to immediately poll on
                        // ssl stream accept.
                    }
                    Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                    Poll::Pending => return Poll::Pending,
                },

                AcceptFutureProj::TlsAccepting { future, service } => match future.poll(cx) {
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
