//! Ed25519 키페어 생성기.
//!
//! 사용법:
//! ```powershell
//! cd saba-chan
//! cargo run -p saba-chan-updater-lib --example gen_ed25519_keypair
//! ```
//!
//! 출력:
//! - stdout: 공개키 바이트(32바이트) — `[u8; 32]` 리터럴 형태로 `signature.rs`에 붙여넣기.
//! - `%APPDATA%/saba-chan/.keys/updater-v1.sk.hex`: 비밀키(32바이트 seed) hex.
//!
//! 비밀키는 절대 커밋/공유 금지. `%APPDATA%`는 gitignore 대상 밖이므로 안전한 위치.
//! 관리자만 접근 가능한 머신에서만 생성/보관한다.

use ed25519_dalek::{SigningKey, VerifyingKey};
use std::fs;
use std::path::PathBuf;

fn app_data_root() -> PathBuf {
    // saba-chan 규약: 모든 데이터는 `%APPDATA%/saba-chan/`.
    let appdata = std::env::var("APPDATA")
        .or_else(|_| std::env::var("XDG_DATA_HOME"))
        .or_else(|_| {
            std::env::var("HOME").map(|h| {
                let p: PathBuf = [h.as_str(), ".local", "share"].iter().collect();
                p.to_string_lossy().to_string()
            })
        })
        .expect("APPDATA/XDG_DATA_HOME/HOME 환경변수 중 하나가 필요합니다");
    PathBuf::from(appdata).join("saba-chan")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1) 32-byte seed
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| format!("getrandom 실패: {}", e))?;

    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key: VerifyingKey = signing_key.verifying_key();
    let public_bytes = verifying_key.to_bytes();

    // 2) 비밀키 → AppData
    let keys_dir = app_data_root().join(".keys");
    fs::create_dir_all(&keys_dir)?;
    let sk_path = keys_dir.join("updater-v1.sk.hex");
    fs::write(&sk_path, hex::encode(seed))?;

    // 3) stdout
    println!("✓ Ed25519 키페어 생성 완료");
    println!();
    println!("◆ 비밀키 저장 위치:");
    println!("   {}", sk_path.display());
    println!("   (절대 커밋/공유 금지. AppData는 git 추적 대상이 아님.)");
    println!();
    println!("◆ 공개키 ({}바이트):", public_bytes.len());
    println!("   hex:    {}", hex::encode(public_bytes));
    println!();
    println!("◆ signature.rs 에 붙여넣을 리터럴:");
    println!("const SABA_UPDATER_V1: [u8; 32] = [");
    for chunk in public_bytes.chunks(8) {
        let items: Vec<String> = chunk.iter().map(|b| format!("0x{:02x}", b)).collect();
        println!("    {},", items.join(", "));
    }
    println!("];");
    println!();
    println!("※ PublicKeyRegistry::bundled() 에서 이 공개키가 로드됩니다.");
    println!("  키를 로테이션할 경우 keyId를 'saba-updater-v2' 등으로 증가시켜 registry에 병존시키세요.");

    Ok(())
}
