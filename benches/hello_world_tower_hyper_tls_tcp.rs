use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as HttpConnectionBuilder;
use hyper_util::service::TowerToHyperService;
use rustls::pki_types::ServerName;
use rustls::{RootCertStore, ServerConfig};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio_rustls::TlsConnector;
use tokio_stream::wrappers::TcpListenerStream;

use hyper_server::{load_certs, load_private_key, serve_http_with_shutdown};

async fn echo(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        (&hyper::Method::GET, "/") => Ok(Response::new(Full::new(Bytes::from("Hello, World!")))),
        (&hyper::Method::POST, "/echo") => {
            let body = req.collect().await?.to_bytes();
            Ok(Response::new(Full::new(body)))
        }
        _ => {
            let mut res = Response::new(Full::new(Bytes::from("Not Found")));
            *res.status_mut() = StatusCode::NOT_FOUND;
            Ok(res)
        }
    }
}

async fn setup_server(
) -> Result<(TcpListenerStream, SocketAddr, Arc<ServerConfig>), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let listener = TcpListener::bind(addr).await?;
    let server_addr = listener.local_addr()?;
    let incoming = TcpListenerStream::new(listener);

    let certs = load_certs("examples/sample.pem")?;
    let key = load_private_key("examples/sample.rsa")?;
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    let tls_config = Arc::new(config);

    Ok((incoming, server_addr, tls_config))
}

async fn start_server() -> Result<(SocketAddr, oneshot::Sender<()>), Box<dyn std::error::Error>> {
    let (incoming, server_addr, tls_config) = setup_server().await?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let http_server_builder = HttpConnectionBuilder::new(TokioExecutor::new());
    let tower_service_fn = tower::service_fn(echo);
    let hyper_service = TowerToHyperService::new(tower_service_fn);

    tokio::spawn(async move {
        serve_http_with_shutdown(
            hyper_service,
            incoming,
            http_server_builder,
            Some(tls_config),
            Some(async {
                shutdown_rx.await.ok();
            }),
        )
        .await
        .unwrap();
    });

    Ok((server_addr, shutdown_tx))
}

fn create_https_client() -> (TlsConnector, ServerName<'static>) {
    let mut root_cert_store = RootCertStore::empty();
    let certs = load_certs("examples/sample.pem").unwrap();
    root_cert_store.add_parsable_certificates(certs);

    let client_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();

    let tls_connector = TlsConnector::from(Arc::new(client_config));
    let domain = ServerName::try_from("localhost").expect("Failed to create ServerName");

    (tls_connector, domain)
}

async fn send_request(
    tls_connector: &TlsConnector,
    domain: &ServerName<'static>,
    addr: SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
    let tcp_stream = TcpStream::connect(addr).await?;
    let tls_stream = tls_connector.connect(domain.clone(), tcp_stream).await?;
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake(TokioIo::new(tls_stream)).await?;

    tokio::spawn(async move {
        if let Err(err) = conn.await {
            eprintln!("Connection failed: {:?}", err);
        }
    });

    let req = Request::builder().uri("/").body(Empty::<Bytes>::new())?;

    let res = sender.send_request(req).await?;
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.collect().await?.to_bytes();
    assert_eq!(&body[..], b"Hello, World!");
    Ok(())
}

fn bench_server(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let (server_addr, shutdown_tx) = rt.block_on(start_server()).unwrap();

    let (tls_connector, domain) = create_https_client();

    let mut group = c.benchmark_group("hyper_server");

    group.bench_function("new_connection_latency", |b| {
        b.to_async(&rt).iter(|| async {
            send_request(&tls_connector, &domain, server_addr)
                .await
                .unwrap()
        });
    });

    let concurrent_requests = vec![10, 50, 100];
    for &num_requests in &concurrent_requests {
        group.bench_with_input(
            BenchmarkId::new("concurrent_connections", num_requests),
            &num_requests,
            |b, &num_requests| {
                b.to_async(&rt).iter(|| async {
                    let requests = (0..num_requests)
                        .map(|_| send_request(&tls_connector, &domain, server_addr));
                    futures::future::join_all(requests).await
                });
            },
        );
    }

    group.bench_function("throughput", |b| {
        b.to_async(&rt).iter(|| async {
            let start = std::time::Instant::now();
            let mut count = 0;

            while start.elapsed() < Duration::from_secs(1) {
                send_request(&tls_connector, &domain, server_addr)
                    .await
                    .unwrap();
                count += 1;
            }

            count
        });
    });

    group.finish();

    // Gracefully shutdown the server
    rt.block_on(async {
        shutdown_tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await; // Give the server time to shut down
    });
}

criterion_group!(benches, bench_server);
criterion_main!(benches);
