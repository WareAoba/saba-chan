//! Windows exe에 아이콘(생선 파비콘)과 버전 정보를 넣는다. 데몬이 쓰는 lib에는 넣지 않는다

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=icons/saba-runner.ico");
    println!("cargo:rerun-if-changed=build.rs");
    // 데몬은 프로토콜(lib)만 쓴다 (default-features = false): 그때는 exe를 만들지 않는다
    if std::env::var_os("CARGO_FEATURE_BIN").is_none() {
        return;
    }
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let ico = manifest_dir.join("icons").join("saba-runner.ico");
    let version = env!("CARGO_PKG_VERSION");
    let numeric = format!(
        "{},{},{},0",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    );
    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{ico}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS 0x40004
FILETYPE 0x1
{{
BLOCK "StringFileInfo"
{{
BLOCK "000004b0"
{{
VALUE "ProductName", "Saba-chan Runner"
VALUE "CompanyName", "Saba-chan"
VALUE "FileDescription", "Saba-chan game server console runner"
VALUE "FileVersion", "{version}"
VALUE "ProductVersion", "{version}"
}}
}}
BLOCK "VarFileInfo" {{
VALUE "Translation", 0x0, 0x04b0
}}
}}
"#,
        ico = ico.display().to_string().replace('\\', "\\\\"),
    );
    let rc_path = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("saba-runner.rc");
    std::fs::write(&rc_path, rc).expect("saba-runner.rc");
    // rc 컴파일러가 없거나 실패하면 아이콘 없는 exe가 나가지 않게 빌드를 멈춘다
    if let Err(e) = embed_resource::compile_for(&rc_path, ["saba-runner"], embed_resource::NONE).manifest_required() {
        panic!("{e}");
    }
}
