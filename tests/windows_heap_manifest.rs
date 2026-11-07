//! 데몬 exe가 Segment Heap 매니페스트를 품었는가 (Windows 전용)
//!
//! 기본 NT 힙은 큰 JSON 응답·프로세스 목록처럼 잠깐 커졌다 풀린 메모리를 OS에 돌려주지 않아,
//! 서버 하나 없이도 데몬이 100MB 가까이 쥐고 있었다(빈 공간이 힙 세그먼트의 66~93%).
#![cfg(windows)]

use windows_sys::Win32::Foundation::FreeLibrary;
use windows_sys::Win32::System::LibraryLoader::{
    FindResourceW, LoadLibraryExW, LoadResource, LockResource, SizeofResource, LOAD_LIBRARY_AS_DATAFILE,
    LOAD_LIBRARY_AS_IMAGE_RESOURCE,
};

/// RT_MANIFEST(24)의 CREATEPROCESS_MANIFEST_RESOURCE_ID(1) 내용
fn embedded_manifest(exe: &str) -> Option<String> {
    let wide: Vec<u16> = exe.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let module = LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE);
        if module.is_null() {
            return None;
        }
        let found = FindResourceW(module, 1 as *const u16, 24 as *const u16);
        let text = (!found.is_null()).then(|| {
            let data = LockResource(LoadResource(module, found)) as *const u8;
            let len = SizeofResource(module, found) as usize;
            String::from_utf8_lossy(std::slice::from_raw_parts(data, len)).into_owned()
        });
        FreeLibrary(module);
        text
    }
}

#[test]
fn the_daemon_runs_on_the_segment_heap() {
    let manifest = embedded_manifest(env!("CARGO_BIN_EXE_saba-core")).expect("saba-core.exe has no embedded manifest");
    assert!(manifest.contains("<heapType") && manifest.contains("SegmentHeap"), "{manifest}");
}
