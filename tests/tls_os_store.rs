//! HTTPS·WSS 클라이언트가 OS 인증서 저장소의 루트를 믿는지 본다.
//! 회사 프록시나 사설 인증서를 OS에 등록한 PC에서도 릴레이·업데이트가 붙어야 한다.
//! 리눅스에서는 OS 저장소를 `SSL_CERT_FILE`로 바꿔 사설 CA를 넣는다 (rustls-native-certs가 읽는 값).
#![cfg(target_os = "linux")]

use std::sync::{Arc, OnceLock};

use futures_util::StreamExt;
use rcgen::{BasicConstraints, Certificate, CertificateParams, IsCa, Issuer, KeyPair};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::Connector;

struct Ca {
    cert: Certificate,
    issuer: Issuer<'static, KeyPair>,
}

fn make_ca() -> Ca {
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let cert = params.self_signed(&key).unwrap();
    Ca { cert, issuer: Issuer::new(params, key) }
}

/// OS 저장소에 넣은 사설 CA. 테스트 프로세스에서 한 번만 정한다
fn trusted_ca() -> &'static Ca {
    static CA: OnceLock<Ca> = OnceLock::new();
    CA.get_or_init(|| {
        let ca = make_ca();
        let path = std::env::temp_dir().join(format!("saba-tls-os-store-{}.pem", std::process::id()));
        let pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, ca.cert.der().as_ref())
        );
        std::fs::write(&path, pem).unwrap();
        // 이 파일의 테스트만 이 값을 읽는다. 인증서 검증기를 만들기 전에 정한다
        unsafe { std::env::set_var("SSL_CERT_FILE", &path) };
        ca
    })
}

/// `ca`가 서명한 localhost 인증서로 TLS를 받는 서버. 연결 하나를 `serve`에 넘긴다
async fn tls_server<F, Fut>(ca: &Ca, serve: F) -> u16
where
    F: FnOnce(tokio_rustls::server::TlsStream<tokio::net::TcpStream>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec!["localhost".to_string()]).unwrap().signed_by(&key, &ca.issuer).unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        if let Ok(tls) = acceptor.accept(tcp).await {
            serve(tls).await;
        }
    });
    port
}

async fn answer_ok(mut tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>) {
    let mut buf = [0u8; 1024];
    let _ = tls.read(&mut buf).await;
    let _ = tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok").await;
    let _ = tls.shutdown().await;
}

fn http() -> reqwest::Client {
    saba_chan_updater_lib::tls::http_client_builder().no_proxy().build().unwrap()
}

#[tokio::test]
async fn https_trusts_private_ca_in_os_store() {
    let ca = trusted_ca();
    let port = tls_server(ca, answer_ok).await;
    let body = http().get(format!("https://localhost:{port}/")).send().await.unwrap().text().await.unwrap();
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn https_rejects_ca_not_in_os_store() {
    trusted_ca();
    let port = tls_server(&make_ca(), answer_ok).await;
    assert!(http().get(format!("https://localhost:{port}/")).send().await.is_err());
}

async fn ws_connect(port: u16) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    let config = saba_chan_updater_lib::tls::client_config().unwrap();
    tokio_tungstenite::connect_async_tls_with_config(
        format!("wss://localhost:{port}/"),
        None,
        false,
        Some(Connector::Rustls(config)),
    )
    .await
    .map(|_| ())
}

async fn accept_ws(tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>) {
    if let Ok(mut ws) = tokio_tungstenite::accept_async(tls).await {
        let _ = ws.next().await;
    }
}

#[tokio::test]
async fn wss_trusts_private_ca_in_os_store() {
    let ca = trusted_ca();
    let port = tls_server(ca, accept_ws).await;
    ws_connect(port).await.unwrap();
}

#[tokio::test]
async fn wss_rejects_ca_not_in_os_store() {
    trusted_ca();
    let port = tls_server(&make_ca(), accept_ws).await;
    assert!(ws_connect(port).await.is_err());
}
