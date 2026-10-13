//! Shared by the TCP benchmark's server, client and load generator: TLS configuration.
use std::sync::Arc;
use tokio_rustls::rustls::{self, pki_types::{CertificateDer, PrivateKeyDer}};

pub fn server_tls(cert: &str, key: &str) -> tokio_rustls::TlsAcceptor {
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert.as_bytes()).collect::<Result<_, _>>().unwrap();
    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut key.as_bytes()).unwrap().unwrap();
    let cfg = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(certs, key).unwrap();
    tokio_rustls::TlsAcceptor::from(Arc::new(cfg))
}

pub fn client_tls(ca: &str) -> tokio_rustls::TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    for c in rustls_pemfile::certs(&mut ca.as_bytes()) {
        roots.add(c.unwrap()).unwrap();
    }
    let cfg = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(cfg))
}

pub fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}
