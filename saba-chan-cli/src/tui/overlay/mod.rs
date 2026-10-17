//! 오버레이 레이어 — 메뉴/탭 위에 겹쳐지는 임시 UI (검색, 파일 브라우저 등)
//!
//! Stage 3: Search 오버레이 (Ctrl+F)
//! 추후 Stage에서 FileBrowser, Confirm 등이 추가됨.

pub mod search;

pub use search::SearchOverlay;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Overlay {
    Search(SearchOverlay),
}

impl Overlay {
    #[allow(dead_code)]
    pub fn title(&self) -> &'static str {
        match self {
            Overlay::Search(_) => "찾기",
        }
    }
}
