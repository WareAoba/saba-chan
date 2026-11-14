use std::path::Path;

fn main() {
    share_gui_tokens();
    share_translations();
    #[cfg(feature = "window")]
    tauri_build::build();
    // tauri-build가 알리는 `mobile` cfg를 창 없는 빌드에서도 안다고 알린다
    #[cfg(not(feature = "window"))]
    println!("cargo:rustc-check-cfg=cfg(mobile)");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
}

/// 창 화면(`../src`)은 GUI 디자인 토큰을 그대로 쓴다: `saba-chan-gui/src/styles/tokens.css`를
/// `../src/tokens.css`(저장소에 넣지 않는다)로 복사한다. GUI 글꼴 파일은 실리지 않으니 `@import` 줄은 뺀다.
/// 내용이 같으면 쓰지 않는다 (화면 폴더가 바뀌면 다시 빌드되므로)
fn share_gui_tokens() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let src = Path::new(&manifest).join("../../../saba-chan-gui/src/styles/tokens.css");
    let dst = Path::new(&manifest).join("../src/tokens.css");
    println!("cargo:rerun-if-changed={}", src.display());
    let tokens = std::fs::read_to_string(&src)
        .unwrap_or_else(|e| panic!("GUI design tokens not found at {}: {e}", src.display()));
    let shared: String = tokens
        .lines()
        .filter(|line| !line.trim_start().starts_with("@import"))
        .map(|line| format!("{line}\n"))
        .collect();
    let shared = format!("/* saba-chan-gui/src/styles/tokens.css에서 빌드 때 복사한 파일 (build.rs). 고치려면 원본을 고친다 */\n{shared}");
    if std::fs::read_to_string(&dst).ok().as_deref() != Some(shared.as_str()) {
        std::fs::write(&dst, shared).unwrap_or_else(|e| panic!("write {}: {e}", dst.display()));
    }
}

/// 창 문구는 `locales/<lang>/installer.json`(`installer`: 설치·제거 창, `updater`: 업데이트 적용 창)에 둔다.
/// 빌드 때 모든 언어를 `../src/translations.js`(저장소에 넣지 않는다)의 `window.SABA_TRANSLATIONS`로 묶는다.
/// 내용이 같으면 쓰지 않는다
fn share_translations() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let locales = Path::new(&manifest).join("../../../locales");
    let dst = Path::new(&manifest).join("../src/translations.js");
    println!("cargo:rerun-if-changed={}", locales.display());
    let mut langs: Vec<_> = std::fs::read_dir(&locales)
        .unwrap_or_else(|e| panic!("locales not found at {}: {e}", locales.display()))
        .filter_map(Result::ok)
        .map(|e| e.path().join("installer.json"))
        .filter(|p| p.is_file())
        .collect();
    langs.sort();
    let mut out = String::from("// locales/<lang>/installer.json을 빌드 때 묶은 파일 (build.rs). 고치려면 원본을 고친다\nwindow.SABA_TRANSLATIONS = {\n");
    for file in langs {
        println!("cargo:rerun-if-changed={}", file.display());
        let lang = file.parent().and_then(Path::file_name).and_then(|n| n.to_str()).expect("language folder");
        let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        out.push_str(&format!("{lang:?}: {},\n", text.trim()));
    }
    out.push_str("};\n");
    if std::fs::read_to_string(&dst).ok().as_deref() != Some(out.as_str()) {
        std::fs::write(&dst, out).unwrap_or_else(|e| panic!("write {}: {e}", dst.display()));
    }
}
