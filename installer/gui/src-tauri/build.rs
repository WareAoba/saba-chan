fn main() {
    #[cfg(feature = "window")]
    tauri_build::build();
    // tauri-build가 알리는 `mobile` cfg를 창 없는 빌드에서도 안다고 알린다
    #[cfg(not(feature = "window"))]
    println!("cargo:rustc-check-cfg=cfg(mobile)");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
}
