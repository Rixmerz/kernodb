//! TLS for both transports, with rustls and the `ring` provider: the certificate chain and key are
//! read once at start, and a file that does not parse stops the server before it listens.

use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::TlsAcceptor;

use crate::args::Tls as Files;

/// The TLS setup of one server.
pub struct Tls {
    /// What the HTTP transport accepts with.
    pub http: TlsAcceptor,
    /// What the gRPC transport hands to tonic.
    pub grpc: tonic::transport::ServerTlsConfig,
}

fn read(path: &std::path::Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

pub fn load(files: &Files) -> Result<Tls, String> {
    let (cert_pem, key_pem) = (read(&files.cert)?, read(&files.key)?);
    let chain = CertificateDer::pem_slice_iter(&cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("--tls-cert {}: {e}", files.cert.display()))?;
    if chain.is_empty() {
        return Err(format!("--tls-cert {}: no certificate in the file", files.cert.display()));
    }
    let key = PrivateKeyDer::from_pem_slice(&key_pem).map_err(|e| format!("--tls-key {}: {e}", files.key.display()))?;
    // ring is the only provider compiled in, so tonic, which builds its own config from the
    // process default, picks it too. A second provider in the tree would make that ambiguous.
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|b| b.with_no_client_auth().with_single_cert(chain, key))
        .map_err(|e| format!("--tls-cert/--tls-key: {e}"))?;
    let identity = tonic::transport::Identity::from_pem(cert_pem, key_pem);
    Ok(Tls {
        http: TlsAcceptor::from(Arc::new(config)),
        grpc: tonic::transport::ServerTlsConfig::new().identity(identity),
    })
}
