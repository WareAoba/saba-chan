//! 릴리스 매니페스트 서명 도구.
//!
//! 사용법:
//! ```powershell
//! cargo run -p saba-chan-updater-lib --example sign_manifest -- <manifest.json>
//! ```
//!
//! 동작:
//! 1. `%APPDATA%/saba-chan/.keys/updater-v1.sk.hex` 에서 비밀키(32바이트 seed) 로드.
//! 2. 인자로 받은 manifest JSON 을 카노니컬 직렬화하여 서명.
//! 3. stdout 에 `{manifest, signature, keyId}` 형태의 SignedManifest JSON 출력.
//!
//! 출력은 `POST /admin/releases` 의 `signedManifest` 필드에 그대로 집어넣는다.
//! (릴레이는 문자열 그대로 저장·반환하며, 클라이언트가 카노니컬 직렬화로 재검증한다.)

use ed25519_dalek::{Signer, SigningKey};
use std::fs;
use std::path::PathBuf;

use saba_chan_updater_lib::signature;

fn app_data_root() -> PathBuf {
    let appdata = std::env::var("APPDATA")
        .or_else(|_| std::env::var("XDG_DATA_HOME"))
        .expect("APPDATA 환경변수가 필요합니다");
    PathBuf::from(appdata).join("saba-chan")
}

fn load_signing_key() -> Result<SigningKey, Box<dyn std::error::Error>> {
    let path = app_data_root().join(".keys").join("updater-v1.sk.hex");
    if !path.exists() {
        return Err(format!(
            "비밀키를 찾을 수 없습니다: {}\n먼저 `cargo run --example gen_ed25519_keypair` 를 실행하세요.",
            path.display()
        )
        .into());
    }
    let hex_str = fs::read_to_string(&path)?;
    let bytes = hex::decode(hex_str.trim())?;
    if bytes.len() != 32 {
        return Err(format!("비밀키 길이 오류: {} (32 필요)", bytes.len()).into());
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    Ok(SigningKey::from_bytes(&seed))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("사용법: cargo run --example sign_manifest -- <manifest.json>");
        std::process::exit(2);
    }

    let manifest_path = &args[1];
    let raw = fs::read_to_string(manifest_path)?;
    let manifest: serde_json::Value = serde_json::from_str(&raw)?;

    let sk = load_signing_key()?;

    // 카노니컬 직렬화 → 서명
    let canonical = signature::canonical_json_bytes(&manifest)
        .map_err(|e| format!("카노니컬 직렬화 실패: {}", e))?;
    let sig = sk.sign(&canonical);

    use base64::Engine as _;
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

    let signed = serde_json::json!({
        "manifest": manifest,
        "signature": sig_b64,
        "keyId": "saba-updater-v1",
    });

    // stdout에 통째로 출력 — 리다이렉트하여 파일 저장 가능.
    println!("{}", serde_json::to_string_pretty(&signed)?);

    eprintln!("✓ 서명 완료. signedManifest 길이: {} bytes", sig_b64.len());
    eprintln!("  이 JSON 을 POST /admin/releases 의 `signedManifest` 필드에 (JSON 문자열로) 넣으세요.");

    Ok(())
}
