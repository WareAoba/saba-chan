//! 파일 무결성 — SHA-256 해시, 검증 다운로드, SHA256SUMS 파싱
//!
//! ## 서브모듈
//! - [`hash`] — `FileHash` 타입, `hash_file`/`hash_bytes`/`verify_file`/`verify_bytes`
//! - [`download`] — `download_verified` (스트리밍 + SHA256 검증, 실패 시 파일 자동 삭제)
//! - [`sha256sums`] — `SHA256SUMS` 형식에서 파일 해시 찾기

pub mod download;
pub mod hash;
pub mod sha256sums;

pub use download::{download_verified, ProgressCallback};
pub use hash::{hash_bytes, hash_file, verify_bytes, verify_file, FileHash, IntegrityError};
pub use sha256sums::find_hash_for_filename;
