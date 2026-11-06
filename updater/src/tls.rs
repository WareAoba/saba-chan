//! HTTPS·WSS 클라이언트의 TLS. 모든 컴포넌트가 rustls 하나를 쓴다.
//! 암호 구현은 ring(데몬 원격 포트·서명 검증과 같은 것), 인증서 검증은 OS 인증서 저장소에 맡긴다
//! (rustls-platform-verifier). 회사 프록시나 사설 인증서를 OS에 등록한 PC에서도 그대로 붙는다.

use std::sync::{Arc, Once};

/// 프로세스 기본 암호 구현을 ring으로 정한다. 여러 번 불러도 된다.
/// reqwest는 기본 암호 구현이 없으면 클라이언트를 만들 때 멈추므로(panic) 그 전에 부른다
pub fn install_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // 다른 곳이 먼저 정했으면 그것을 쓴다
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// reqwest 클라이언트는 이것으로 만든다 (`clippy.toml`이 직접 생성을 막는다)
#[allow(clippy::disallowed_methods)]
pub fn http_client_builder() -> reqwest::ClientBuilder {
    install_crypto_provider();
    reqwest::Client::builder()
}

/// reqwest 밖의 TLS 연결(릴레이 웹소켓 등)이 쓰는 설정. reqwest와 같은 검증을 한다
pub fn client_config() -> Result<Arc<rustls::ClientConfig>, rustls::Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls_platform_verifier::Verifier::new(provider.clone())?;
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    Ok(Arc::new(config))
}
