//! 릴리즈 manifest 서명 도구 (릴리즈 워크플로와 키 관리용)
//!
//! ```text
//! cargo run -p saba-chan-updater-lib --example release_signing -- gen-key
//! UPDATER_SIGNING_KEY=<base64 pkcs8> cargo run -p saba-chan-updater-lib \
//!     --example release_signing -- sign manifest.json [key-id] > signed.json
//! ```
//!
//! `gen-key`가 출력한 공개키는 `updater/src/signature.rs`의 `TRUSTED_KEYS`에,
//! 개인키는 CI 시크릿 `UPDATER_SIGNING_KEY`에 넣는다.

use saba_chan_updater_lib::signature;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen-key") => {
            let (private, public) = signature::generate_key_pair().expect("key generation");
            println!("private (UPDATER_SIGNING_KEY): {}", private);
            println!("public  (TRUSTED_KEYS):        {}", public);
        }
        Some("sign") if args.len() >= 2 => {
            let key = std::env::var("UPDATER_SIGNING_KEY").expect("UPDATER_SIGNING_KEY is not set");
            let key_id = args.get(2).map(String::as_str).unwrap_or(signature::TRUSTED_KEYS[0].0);
            let text = std::fs::read_to_string(&args[1]).expect("read manifest");
            let manifest: serde_json::Value = serde_json::from_str(&text).expect("manifest is not JSON");
            let signed = signature::sign(manifest, key_id, &key).expect("sign");
            signature::verify(&signed).expect("signature does not verify with the bundled public key");
            println!("{}", serde_json::to_string_pretty(&signed).unwrap());
        }
        _ => {
            eprintln!("usage: release_signing gen-key | sign <manifest.json> [key-id]");
            std::process::exit(2);
        }
    }
}
