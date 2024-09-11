# hyper-server

[![License](https://img.shields.io/crates/l/hyper-server)](https://choosealicense.com/licenses/mit/)
[![Crates.io](https://img.shields.io/crates/v/hyper-server)](https://crates.io/crates/hyper-server)
[![Docs](https://img.shields.io/crates/v/hyper-server?color=blue&label=docs)](https://docs.rs/hyper-server/)
![CI](https://github.com/warlock-labs/hyper-server/actions/workflows/CI.yml/badge.svg)
[![codecov](https://codecov.io/gh/warlock-labs/hyper-server/branch/master/graph/badge.svg?token=8W5MEJQSW6)](https://codecov.io/gh/warlock-labs/hyper-server)

A high-performance, modular server implementation built on [hyper], designed to 
work seamlessly with [axum], [tonic], [tower], and other tower-compatible 
frameworks.

## Features

- HTTP/1 and HTTP/2 support
- TLS/HTTPS through [rustls]
- High performance leveraging [hyper] 1.0
- Modular architecture based on `tokio::net::TcpListener`, `tokio-rustls::Acceptor`, and `hyper::server::conn::auto`
- Flexible integration with [tower] the ecosystem, supporting various backends:
    - [axum]
    - [tonic]
    - [tungstenite]
    - Any `hyper::service::Service`, `tower::Service`, or `tower::Layer` composition

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
hyper-server = "0.7.0"
```

## Usage

Here's an example of how to use hyper-server with a simple tower service:

```rust
use std::convert::Infallible;
use std::net::SocketAddr;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::Full;
use hyper_util::rt::TokioExecutor;
use hyper_util::server::conn::auto::Builder as HttpConnectionBuilder;
use tower::Service;
use tokio::net::TcpListener;

// A simple tower service
#[derive(Clone)]
struct HelloService;

impl Service<Request<hyper::body::Incoming>> for HelloService {
    type Response = Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&mut self, req: Request<hyper::body::Incoming>) -> Self::Future {
        Box::pin(async move {
            let response = match (req.method(), req.uri().path()) {
                (&hyper::Method::GET, "/") => {
                    Response::new(Full::new(Bytes::from("Hello, World!")))
                }
                _ => {
                    let mut res = Response::new(Full::new(Bytes::from("Not Found")));
                    *res.status_mut() = StatusCode::NOT_FOUND;
                    res
                }
            };
            Ok(response)
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = TcpListener::bind(addr).await?;
    println!("Listening on http://{}", addr);

    let http_builder = HttpConnectionBuilder::new(TokioExecutor::new());
    let service = HelloService;

    hyper_server::serve_http_with_shutdown(
        service,
        tokio_stream::wrappers::TcpListenerStream::new(listener),
        http_builder,
        None,
    )
    .await?;

    Ok(())
}
```

For more advanced usage and examples, including TLS configuration and custom service implementations, please refer to the [examples directory](/examples).

## Architecture

hyper-server provides a layered, composable architecture:

1. TCP Listening: `tokio::net::TcpListener`
2. TLS (optional): `rustls::TlsAcceptor`
3. HTTP: `hyper_util::server::conn::auto`
4. Service: `hyper::service::Service`
5. Middleware: `tower::Service`
6. Application: Your choice of tower-compatible framework (axum, tonic, etc.) or custom service implementation

This structure allows for easy customization and extension at each layer. You 
can integrate your own implementations at any level of the stack, providing 
maximum flexibility for your specific use case.

## Security

hyper-server takes security seriously. We use `rustls` for TLS support, which 
provides modern, secure defaults. However, please ensure that you configure 
your server appropriately for your use case, especially when deploying in 
production environments.

If you discover any security-related issues, please email team@warlock.xyz 
instead of using the issue tracker.

## API

For detailed API documentation, please refer to the [API docs on docs.rs](https://docs.rs/hyper-server/).

## Minimum Supported Rust Version

hyper-server's MSRV is `1.80`.

## Contributing

We welcome contributions to hyper-server! Our contributing guidelines are 
inspired by the Rule of St. Benedict, emphasizing humility, listening, 
and community. Before contributing, please familiarize yourself with these 
principles at [The Rule of St. Benedict](http://www.benedictfriend.org/the-rule.html).

Key points for contributors:

- Listen first, speak second (Chapter 6)
- Be humble in your contributions (Chapter 7)
- Work diligently and carefully (Chapter 48)
- Treat all code and ideas with respect (Chapter 72)

## License

This project is licensed under the MIT License — see 
the [LICENSE](/LICENSE) file for details.

[axum]: https://crates.io/crates/axum
[hyper]: https://crates.io/crates/hyper
[rustls]: https://crates.io/crates/rustls
[tower]: https://crates.io/crates/tower
[tonic]: https://crates.io/crates/tonic
[tungstenite]: https://crates.io/crates/tungstenite