use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use http::{Request, Response, StatusCode, Uri};
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper_rustls::HttpsConnectorBuilder;
use hyper_server::{load_certs, load_private_key, serve_http_with_shutdown};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use hyper_util::server::conn::auto::Builder as HttpConnectionBuilder;
use hyper_util::service::TowerToHyperService;
use rustls::server::ServerSessionMemoryCache;
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpSocket;
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::time::{Duration, Instant};
use tokio_stream::wrappers::TcpListenerStream;
use tracing::info;

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

async fn setup_server() -> Result<
    (TcpListenerStream, SocketAddr, Arc<ServerConfig>),
    Box<dyn std::error::Error + Send + Sync>,
> {
    // Socket configuration
    let addr = SocketAddr::from(([127, 0, 0, 1], 0)); // Listen on all interfaces
    let socket = TcpSocket::new_v4()?;
    socket.set_send_buffer_size(262_144)?; // 256 KB
    socket.set_recv_buffer_size(262_144)?; // 256 KB
    socket.set_nodelay(true)?; // Disable Nagle's algorithm
    socket.bind(addr)?;
    let listener = socket.listen(8192)?; // Increase backlog for high-traffic scenarios
    let server_addr = listener.local_addr()?;
    let incoming = TcpListenerStream::new(listener);

    // Load certificates and private key
    let certs = load_certs("examples/sample.pem")?;
    let key = load_private_key("examples/sample.rsa")?;

    // TLS configuration
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // ALPN configuration
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    // Performance optimizations
    config.max_fragment_size = Some(16384); // Larger fragment size for powerful servers
    config.send_half_rtt_data = true; // Enable 0.5-RTT data
    config.session_storage = ServerSessionMemoryCache::new(10240); // Larger session cache
    config.max_early_data_size = 16384; // Enable 0-RTT data

    let tls_config = Arc::new(config);

    Ok((incoming, server_addr, tls_config))
}

async fn start_server(
) -> Result<(SocketAddr, oneshot::Sender<()>), Box<dyn std::error::Error + Send + Sync>> {
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

async fn send_request(
    client: &Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        Empty<Bytes>,
    >,
    url: Uri,
) -> Result<(Duration, usize), Box<dyn std::error::Error + Send + Sync>> {
    let start = Instant::now();
    let res = client.get(url).await?;
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await?.to_bytes();
    assert_eq!(&body[..], b"Hello, World!");
    Ok((start.elapsed(), body.len()))
}

async fn concurrent_benchmark(
    client: &Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
        Empty<Bytes>,
    >,
    url: Uri,
    num_requests: usize,
) -> (Duration, Vec<Duration>, usize) {
    let start = Instant::now();
    let mut futures = FuturesUnordered::new();

    for _ in 0..num_requests {
        let client = client.clone();
        let url = url.clone();
        futures.push(async move { send_request(&client, url).await });
    }

    let mut request_times = Vec::with_capacity(num_requests);
    let mut total_bytes = 0;
    while let Some(result) = futures.next().await {
        if let Ok((duration, bytes)) = result {
            request_times.push(duration);
            total_bytes += bytes;
        }
    }

    let total_time = start.elapsed();
    (total_time, request_times, total_bytes)
}

fn bench_server(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (server_addr, shutdown_tx, client) = rt.block_on(async {
        let (server_addr, shutdown_tx) = start_server().await.expect("Failed to start server");
        info!("Server started on {}", server_addr);

        let mut root_cert_store = RootCertStore::empty();
        root_cert_store.add_parsable_certificates(load_certs("examples/sample.pem").unwrap());

        let client_config = ClientConfig::builder()
            .with_root_certificates(root_cert_store)
            .with_no_client_auth();

        let https = HttpsConnectorBuilder::new()
            .with_tls_config(client_config)
            .https_or_http()
            .enable_http1()
            .build();

        let client: Client<_, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build(https);

        (server_addr, shutdown_tx, client)
    });

    let url = Uri::builder()
        .scheme("https")
        .authority(format!("localhost:{}", server_addr.port()))
        .path_and_query("/")
        .build()
        .expect("Failed to build URI");

    let mut group = c.benchmark_group("hyper_server");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(30));

    // Latency test
    group.bench_function("latency", |b| {
        let client = client.clone();
        let url = url.clone();
        b.to_async(&rt)
            .iter(|| async { send_request(&client, url.clone()).await.unwrap().0 });
    });

    // Throughput test
    group.throughput(Throughput::Elements(1));
    group.bench_function("throughput", |b| {
        let client = client.clone();
        let url = url.clone();
        b.to_async(&rt)
            .iter(|| async { send_request(&client, url.clone()).await.unwrap() });
    });

    // Concurrency stress test
    let concurrent_requests = vec![1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987]; // Fibonacci sequence
    for &num_requests in &concurrent_requests {
        group.throughput(Throughput::Elements(num_requests as u64));
        group.bench_with_input(
            BenchmarkId::new("concurrent_requests", num_requests),
            &num_requests,
            |b, &num_requests| {
                let client = client.clone();
                let url = url.clone();
                b.to_async(&rt).iter(|| async {
                    concurrent_benchmark(&client, url.clone(), num_requests).await
                });
            },
        );
    }

    group.finish();

    rt.block_on(async {
        shutdown_tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(20))
        .warm_up_time(Duration::from_secs(5));
    targets = bench_server
}

criterion_main!(benches);
