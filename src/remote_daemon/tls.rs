//! 원격 데몬 TLS (D1): 데이터 폴더의 자체 서명 인증서 하나와, 붙는 쪽의 지문 고정.
//! 지문(인증서 DER의 SHA-256, 소문자 hex)은 릴레이가 같은 주인의 PC에만 알려 준다 (05 §4.4 (j))

use std::{path::Path, sync::Arc};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

const CERT_FILE: &str = "remote-cert.der";
const KEY_FILE: &str = "remote-key.der";

/// 붙는 쪽이 SNI로 보내는 이름. 인증서 검사는 지문으로만 하므로 주소와 상관없다
pub const SERVER_NAME: &str = "saba-chan";

pub fn fingerprint(der: &[u8]) -> String {
    hex::encode(Sha256::digest(der))
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

pub struct Identity {
    cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    pub sha256: String,
}

impl Identity {
    /// `dir`의 인증서를 읽고, 없거나 쓸 수 없으면 새로 만든다 (새로 만들면 지문이 바뀐다)
    pub fn load_or_create(dir: &Path) -> anyhow::Result<Self> {
        let (cert_path, key_path) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
        if let (Ok(cert), Ok(key)) = (std::fs::read(&cert_path), std::fs::read(&key_path)) {
            let id = Self::from_der(cert, key);
            if id.server_config().is_ok() {
                return Ok(id);
            }
            tracing::warn!("Remote daemon certificate is unusable; creating a new one");
        }
        let made = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_string()])?;
        let (cert, key) = (made.cert.der().to_vec(), made.key_pair.serialize_der());
        saba_atomic_file::write(&key_path, &key, saba_atomic_file::Mode::Secret)?;
        saba_atomic_file::write(&cert_path, &cert, saba_atomic_file::Mode::Plain)?;
        Ok(Self::from_der(cert, key))
    }

    fn from_der(cert: Vec<u8>, key: Vec<u8>) -> Self {
        let sha256 = fingerprint(&cert);
        Self { cert: CertificateDer::from(cert), key: PrivatePkcs8KeyDer::from(key), sha256 }
    }

    pub fn server_config(&self) -> anyhow::Result<rustls::ServerConfig> {
        Ok(rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(vec![self.cert.clone()], PrivateKeyDer::Pkcs8(self.key.clone_key()))?)
    }
}

/// 인증서 사슬 대신 지문 하나만 믿는다. 서명 검사(핸드셰이크 소유 증명)는 그대로 한다
#[derive(Debug)]
struct Pinned {
    sha256: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if fingerprint(end_entity) == self.sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// 지문이 `sha256`인 인증서에만 붙는 클라이언트 설정
pub fn pinned_client_config(sha256: &str) -> anyhow::Result<rustls::ClientConfig> {
    let provider = provider();
    Ok(rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { sha256: sha256.to_ascii_lowercase(), provider }))
        .with_no_client_auth())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn handshake(server: &Identity, pin: &str) -> std::io::Result<()> {
        let (a, b) = tokio::io::duplex(16 * 1024);
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server.server_config().unwrap()));
        let srv = tokio::spawn(async move {
            let mut s = acceptor.accept(b).await?;
            s.write_all(b"ok").await?;
            s.flush().await
        });
        let connector = tokio_rustls::TlsConnector::from(Arc::new(pinned_client_config(pin).unwrap()));
        let mut c = connector.connect(ServerName::try_from(SERVER_NAME).unwrap(), a).await?;
        let mut buf = [0u8; 2];
        c.read_exact(&mut buf).await?;
        assert_eq!(&buf, b"ok");
        let _ = srv.await;
        Ok(())
    }

    #[test]
    fn the_certificate_is_made_once_and_its_fingerprint_stays() {
        let dir = tempfile::tempdir().unwrap();
        let first = Identity::load_or_create(dir.path()).unwrap();
        assert!(first.sha256.len() == 64 && first.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(Identity::load_or_create(dir.path()).unwrap().sha256, first.sha256);
    }

    #[test]
    fn a_broken_certificate_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let first = Identity::load_or_create(dir.path()).unwrap();
        std::fs::write(dir.path().join(KEY_FILE), b"broken").unwrap();
        let again = Identity::load_or_create(dir.path()).unwrap();
        assert_ne!(again.sha256, first.sha256);
        assert!(again.server_config().is_ok());
    }

    #[tokio::test]
    async fn a_client_connects_only_to_the_pinned_certificate() {
        let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let server = Identity::load_or_create(d1.path()).unwrap();
        let other = Identity::load_or_create(d2.path()).unwrap();
        handshake(&server, &server.sha256).await.expect("pinned certificate");
        handshake(&server, &server.sha256.to_uppercase()).await.expect("case does not matter");
        assert!(handshake(&server, &other.sha256).await.is_err(), "another certificate must be refused");
    }
}
