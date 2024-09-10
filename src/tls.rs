use crate::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::TlsAcceptor;
use tokio_stream::{Stream, StreamExt};

pub(crate) fn tls_incoming<IO>(
    tcp_stream: impl Stream<Item = Result<IO, Error>>,
    tls: TlsAcceptor,
) -> impl Stream<Item = Result<tokio_rustls::server::TlsStream<IO>, Error>>
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tcp_stream.then(move |result| {
        let tls = tls.clone();
        async move {
            match result {
                Ok(io) => tls.accept(io).await.map_err(Error::from),
                Err(e) => Err(e),
            }
        }
    })
}
