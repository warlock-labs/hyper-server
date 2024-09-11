use rustls::ClientConfig;
use rustls::RootCertStore;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use futures::future::join_all;
use http::{Request, Response, StatusCode, Uri};
use http_body_util::{Empty, Full, BodyExt};
use hyper::body::Incoming;
use hyper_util::rt::TokioExecutor;
use hyper_util::server::conn::auto::Builder as HttpConnectionBuilder;
use rustls::ServerConfig;
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::sync::{oneshot, Semaphore};
use tokio_stream::wrappers::TcpListenerStream;
use hyper_util::service::TowerToHyperService;
use tracing::info;
use hyper_server::{load_certs, load_private_key, serve_http_with_shutdown};
use hyper_rustls::HttpsConnectorBuilder;
use hyper_util::client::legacy::Client;

async fn echo(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        (&hyper::Method::GET, "/") => Ok(Response::new(Full::new(Bytes::from("Hello, World!")))),
        (&hyper::Method::POST, "/echo") => {
            let body = req.collect().await?.to_bytes();
            Ok(Response::new(Full::new(body)))
        },
        _ => {
            let mut res = Response::new(Full::new(Bytes::from("Not Found")));
            *res.status_mut() = StatusCode::NOT_FOUND;
            Ok(res)
        }
    }
}

async fn setup_server() -> Result<(TcpListenerStream, SocketAddr, Arc<ServerConfig>), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let listener = TcpListener::bind(addr).await?;
    let server_addr = listener.local_addr()?;
    let incoming = TcpListenerStream::new(listener);

    let certs = load_certs("examples/sample.pem")?;
    let key = load_private_key("examples/sample.rsa")?;

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
    let tls_config = Arc::new(config);

    Ok((incoming, server_addr, tls_config))
}


async fn start_server() -> Result<(SocketAddr, oneshot::Sender<()>), Box<dyn std::error::Error + Send + Sync>> {
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
            Some(async { shutdown_rx.await.ok(); }),
        )
            .await
            .unwrap();
    });
    Ok((server_addr, shutdown_tx))
}

async fn send_request(client: &Client<hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>, Empty<Bytes>>, url: Uri) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let res = client.get(url).await?;
    assert_eq!(res.status(), StatusCode::OK);
    let body = res.into_body().collect().await?.to_bytes();
    assert_eq!(&body[..], b"Hello, World!");
    Ok(())
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
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));

    // ... [keep the rest of the benchmark code as it is] ...

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