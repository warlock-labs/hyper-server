[![License](https://img.shields.io/crates/l/hyper-server)](https://choosealicense.com/licenses/mit/)
[![Crates.io](https://img.shields.io/crates/v/hyper-server)](https://crates.io/crates/hyper-server)
[![Docs](https://img.shields.io/crates/v/hyper-server?color=blue&label=docs)](https://docs.rs/hyper-server/)
![CI](https://github.com/valorem-labs-inc/hyper-server/actions/workflows/CI.yml/badge.svg)
[![codecov](https://codecov.io/gh/valorem-labs-inc/hyper-server/branch/master/graph/badge.svg?token=8W5MEJQSW6)](https://codecov.io/gh/valorem-labs-inc/hyper-server)

# hyper-server

hyper-server is a high performance [hyper] server implementation designed to 
work with [axum], [tonic] and [tower].

## Features

- HTTP/1 and HTTP/2
- HTTPS through [rustls] and openssl.
- High performance through [hyper].
- Using [tower] make service API.
- Exceptional [axum] compatibility. Likely to work with future [axum] releases.
- Superb [tonic] compatibility. Likely to work with future [tonic] releases.

## Usage Example

A simple hello world application can be served like:

```rust
use axum::{routing::get, Router};
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    let app = Router::new().route("/", get(|| async { "Hello, world!" }));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("listening on {}", addr);
    hyper_server::bind(addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}
```

You can find more examples [here](/examples).

## Minimum Supported Rust Version

hyper-server's MSRV is `1.65`.

## Safety

This crate uses `#![forbid(unsafe_code)]` to ensure everything is implemented in 100% safe Rust.

## License

This project is licensed under the [MIT license](LICENSE).

## Why fork

This project is based on the great work in [axum-server], which is no longer actively maintained.
The rationale for forking is that we use this for critical infrastructure and want to be able to
extend the crate and fix bugs as needed.

[axum-server]: https://github.com/programatik29/axum-server
[axum]: https://crates.io/crates/axum
[hyper]: https://crates.io/crates/hyper
[rustls]: https://crates.io/crates/rustls
[tower]: https://crates.io/crates/tower
[tonic]: https://crates.io/crates/tonic
