use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::server::TlsStream;

pub(crate) enum Transport<IO> {
    Plain(IO),
    Tls(Box<TlsStream<IO>>),
}

impl<IO> Transport<IO> {
    pub(crate) fn new_plain(io: IO) -> Self {
        Self::Plain(io)
    }

    pub(crate) fn new_tls(io: TlsStream<IO>) -> Self {
        Self::Tls(Box::from(io))
    }
}

impl<IO> AsyncRead for Transport<IO>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Transport::Plain(io) => Pin::new(io).poll_read(cx, buf),
            Transport::Tls(io) => Pin::new(io).poll_read(cx, buf),
        }
    }
}

impl<IO> AsyncWrite for Transport<IO>
where
    IO: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Transport::Plain(io) => Pin::new(io).poll_write(cx, buf),
            Transport::Tls(io) => Pin::new(io).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Transport::Plain(io) => Pin::new(io).poll_flush(cx),
            Transport::Tls(io) => Pin::new(io).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Transport::Plain(io) => Pin::new(io).poll_shutdown(cx),
            Transport::Tls(io) => Pin::new(io).poll_shutdown(cx),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        match self.get_mut() {
            Transport::Plain(io) => Pin::new(io).poll_write_vectored(cx, bufs),
            Transport::Tls(io) => Pin::new(io).poll_write_vectored(cx, bufs),
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Transport::Plain(io) => io.is_write_vectored(),
            Transport::Tls(io) => io.is_write_vectored(),
        }
    }
}
