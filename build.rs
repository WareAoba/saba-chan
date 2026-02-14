fn main() {
    #[cfg(windows)]
    {
        use std::env;
        use std::path::PathBuf;

        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let png_path = PathBuf::from(&manifest_dir)
            .join("resources")
            .join("core-icon.png");
        let ico_path = PathBuf::from(&manifest_dir)
            .join("resources")
            .join("core-icon.ico");

        // ICO 파일이 있으면 사용, 없으면 경고
        if ico_path.exists() {
            let mut res = winres::WindowsResource::new();
            res.set_icon(ico_path.to_str().unwrap());
            
            if let Err(e) = res.compile() {
                println!("cargo:warning=Failed to compile resources: {}", e);
            }
        } else if png_path.exists() {
            println!("cargo:warning=core-icon.png found, but ICO file is required. Please convert PNG to ICO format using an image editor or online tool.");
            println!("cargo:warning=Place the converted ICO file at: {}", ico_path.display());
        } else {
            println!(
                "cargo:warning=Icon file not found. Please place core-icon.png or core-icon.ico in the resources directory"
            );
        }
    }
}
