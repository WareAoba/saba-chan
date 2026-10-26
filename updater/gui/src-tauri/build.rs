fn main() {
    #[cfg(feature = "window")]
    tauri_build::build();
    // tauri-build가 알리는 `mobile` cfg를 창 없는 빌드에서도 안다고 알린다
    #[cfg(not(feature = "window"))]
    println!("cargo:rustc-check-cfg=cfg(mobile)");
}
