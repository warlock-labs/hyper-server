// Crate links:
// - tokio: https://docs.rs/tokio/latest/tokio/
// - rustls: https://docs.rs/rustls/latest/rustls/
// - tokio_rustls:https://docs.rs/tokio-rustls/latest/tokio_rustls/
// - hyper: https://docs.rs/hyper/latest/hyper/
// - tower: https://docs.rs/tower/latest/tower/

// We take a `SocketAddr` to bind the server to a specific address.
// We use `TcpListener` to bind the server to the specified address at the TCP layer.
// We use `rustls` to create a new `ServerConfig` instance.
// We use `tokio_rustls` to create a new `TlsAcceptor` instance.
// We use `hyper_util::server::conn::auto` to create a new `Connection` instance.
// Which then passes requests to a `hyper::service::Service` instance.
// Which then can optionally pass requests to a `tower::Service` instance.
// Behind that can be axum, tower, tonic,
// or any other service that implements the `tower::Service` trait.

pub struct HyperServer {}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_server() {
        // Get a random port from the OS
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        // Create a TCP listener bound to the random address
        let listener = TcpListener::bind(&addr).await.unwrap();
    }
}
