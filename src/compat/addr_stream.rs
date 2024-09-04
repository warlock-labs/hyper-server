//!
//! This is a compatibility type to bridge between hyper 0.14 and hyper 1.x
//!

use std::io;
use std::net::SocketAddr;
use std::os::fd::{AsRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

pin_project_lite::pin_project! {

    /// A transport returned yielded by `AddrIncoming`.
    #[derive(Debug)]
    pub struct AddrStream {
        #[pin]
        inner: TcpStream,
        pub(super) remote_addr: SocketAddr,
        pub(super) local_addr: SocketAddr
    }
}

impl AddrStream {
    pub(super) fn new(
        tcp: TcpStream,
        remote_addr: SocketAddr,
        local_addr: SocketAddr,
    ) -> AddrStream {
        AddrStream {
            inner: tcp,
            remote_addr,
            local_addr,
        }
    }

    /// Returns the remote (peer) address of this connection.
    #[inline]
    pub fn remote_addr(&self) -> SocketAddr {
        self.remote_addr
    }

    /// Returns the local address of this connection.
    #[inline]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Consumes the AddrStream and returns the underlying IO object
    #[inline]
    pub fn into_inner(self) -> TcpStream {
        self.inner
    }

    /// Attempt to receive data on the socket, without removing that data
    /// from the queue, registering the current task for wakeup if data is
    /// not yet available.
    pub fn poll_peek(
        &mut self,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<usize>> {
        self.inner.poll_peek(cx, buf)
    }
}

impl AsyncRead for AddrStream {
    #[inline]
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.project().inner.poll_read(cx, buf)
    }
}

impl AsyncWrite for AddrStream {
    #[inline]
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.project().inner.poll_write(cx, buf)
    }

    #[inline]
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.project().inner.poll_write_vectored(cx, bufs)
    }

    #[inline]
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // TCP flush is a noop
        Poll::Ready(Ok(()))
    }

    #[inline]
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.project().inner.poll_shutdown(cx)
    }

    #[inline]
    fn is_write_vectored(&self) -> bool {
        // Note that since `self.inner` is a `TcpStream`, this could
        // *probably* be hard-coded to return `true`...but it seems more
        // correct to ask it anyway (maybe we're on some platform without
        // scatter-gather IO?)
        self.inner.is_write_vectored()
    }
}

#[cfg(unix)]
impl AsRawFd for AddrStream {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}
