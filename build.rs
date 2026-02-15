fn main() {
    #[cfg(windows)]
    {
        use std::env;
        use std::path::PathBuf;

        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        let res_dir = PathBuf::from(&manifest_dir).join("resources");
        let ico_path = res_dir.join("core-icon.ico");
        let png_path = res_dir.join("core-icon.png");

        // build.rs가 아이콘 변경 시 재실행되도록
        println!("cargo:rerun-if-changed=resources/core-icon.ico");
        println!("cargo:rerun-if-changed=resources/core-icon.png");
        println!("cargo:rerun-if-changed=build.rs");

        if ico_path.exists() {
            println!("cargo:warning=Embedding icon from: {}", ico_path.display());

            // 1) .rc 파일 생성
            let rc_path = out_dir.join("saba-core.rc");
            let ico_escaped = ico_path.to_str().unwrap().replace('\\', "\\\\");
            let rc_content = format!(
                r#"#pragma code_page(65001)
1 VERSIONINFO
FILETYPE 0x1
FILEFLAGSMASK 0x3f
FILEFLAGS 0x0
PRODUCTVERSION 0,1,0,0
FILEVERSION 0,1,0,0
FILEOS 0x40004
FILESUBTYPE 0x0
{{
BLOCK "StringFileInfo"
{{
BLOCK "000004b0"
{{
VALUE "ProductName", "Saba-chan Core Daemon"
VALUE "CompanyName", "Saba-chan"
VALUE "FileVersion", "0.1.0"
VALUE "ProductVersion", "0.1.0"
VALUE "FileDescription", "Saba-chan Game Server Management Core"
VALUE "LegalCopyright", "Copyright (c) 2025 Saba-chan Contributors"
}}
}}
BLOCK "VarFileInfo" {{
VALUE "Translation", 0x0, 0x04b0
}}
}}
1 ICON "{ico}"
"#,
                ico = ico_escaped
            );
            std::fs::write(&rc_path, &rc_content).expect("Failed to write .rc file");

            // 2) rc.exe 탐색 — Windows SDK에서 검색
            let rc_exe = find_rc_exe();
            match rc_exe {
                Some(rc) => {
                    let res_path = out_dir.join("saba-core.res");
                    println!("cargo:warning=Using RC compiler: {}", rc.display());

                    let output = std::process::Command::new(&rc)
                        .args(["/fo", res_path.to_str().unwrap(), rc_path.to_str().unwrap()])
                        .output();

                    match output {
                        Ok(o) if o.status.success() => {
                            // MSVC 링커에 .res 파일 직접 전달
                            println!("cargo:rustc-link-arg={}", res_path.display());
                            println!("cargo:warning=Windows resources compiled successfully (icon + version info embedded)");
                        }
                        Ok(o) => {
                            let stderr = String::from_utf8_lossy(&o.stderr);
                            let stdout = String::from_utf8_lossy(&o.stdout);
                            println!("cargo:warning=rc.exe failed (exit {}): {}{}", o.status, stdout, stderr);
                            // fallback: winres 사용
                            compile_with_winres(&ico_path);
                        }
                        Err(e) => {
                            println!("cargo:warning=Failed to run rc.exe: {}", e);
                            compile_with_winres(&ico_path);
                        }
                    }
                }
                None => {
                    println!("cargo:warning=rc.exe not found, using winres crate fallback");
                    compile_with_winres(&ico_path);
                }
            }
        } else if png_path.exists() {
            println!("cargo:warning=core-icon.png found, but ICO file is required. Please convert PNG to ICO format.");
            println!("cargo:warning=Place the converted ICO file at: {}", ico_path.display());
        } else {
            println!(
                "cargo:warning=Icon file not found. Please place core-icon.ico in: {}",
                res_dir.display()
            );
        }
    }
}

/// Windows SDK에서 rc.exe를 탐색
#[cfg(windows)]
fn find_rc_exe() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    // 일반적인 Windows SDK 경로
    let sdk_roots = [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ];

    for root in &sdk_roots {
        let root_path = PathBuf::from(root);
        if !root_path.exists() {
            continue;
        }

        // 버전 디렉토리를 역순으로 정렬하여 최신 버전 우선
        let mut versions: Vec<_> = std::fs::read_dir(&root_path)
            .ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| n.starts_with("10."))
                    .unwrap_or(false)
            })
            .collect();
        versions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        for ver_dir in versions {
            let rc_path = ver_dir.path().join("x64").join("rc.exe");
            if rc_path.exists() {
                return Some(rc_path);
            }
        }
    }

    None
}

/// winres crate fallback (아이콘만 embed, 버전 정보는 누락될 수 있음)
#[cfg(windows)]
fn compile_with_winres(ico_path: &std::path::Path) {
    let mut res = winres::WindowsResource::new();
    res.set_icon(ico_path.to_str().unwrap());
    match res.compile() {
        Ok(_) => println!("cargo:warning=winres fallback: icon embedded (version info may be missing)"),
        Err(e) => println!("cargo:warning=winres fallback failed: {}", e),
    }
}