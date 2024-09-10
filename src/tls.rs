use std::sync::Arc;
use std::{fs, io};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

fn error(err: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err)
}

pub fn create_tls_acceptor(cert_path: &str, key_path: &str) -> io::Result<TlsAcceptor> {
    // Load public certificate.
    let certs = load_certs(cert_path)?;
    // Load private key.
    let key = load_private_key(key_path)?;

    // Build TLS configuration.
    let mut server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| error(e.to_string()))?;
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

// Load public certificate from file.
fn load_certs(filename: &str) -> io::Result<Vec<CertificateDer<'static>>> {
    // Open certificate file.
    let certfile = fs::File::open(filename)
        .map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
    let mut reader = io::BufReader::new(certfile);

    // Load and return certificate.
    rustls_pemfile::certs(&mut reader).collect()
}

// Load private key from file.
fn load_private_key(filename: &str) -> io::Result<PrivateKeyDer<'static>> {
    // Open keyfile.
    let keyfile = fs::File::open(filename)
        .map_err(|e| error(format!("failed to open {}: {}", filename, e)))?;
    let mut reader = io::BufReader::new(keyfile);

    // Load and return a single private key.
    rustls_pemfile::private_key(&mut reader).map(|key| key.unwrap())
}

pub async fn tls_accept(acceptor: TlsAcceptor, tcp_stream: TcpStream) -> Result<tokio_rustls::server::TlsStream<TcpStream>, std::io::Error> {
    acceptor.accept(tcp_stream).await.map_err(|e| error(format!("failed to perform tls handshake: {}", e)))
}