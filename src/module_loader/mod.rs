//! module.toml 계약: 데몬에 주는 공개 메타데이터 타입(`schema`), TOML 원본 구조체·읽기·값 검사(`raw`),
//! 모듈 폴더·ZIP 로더(`loader`). 바깥에서는 지금까지처럼 `module_loader::`로 쓴다

mod loader;
mod raw;
mod schema;

pub use loader::*;
pub use raw::*;
pub use schema::*;

#[cfg(test)]
mod tests;
