//! saba-runner.exe에 파일 버전 정보를 넣는다 (발행 때 versions.py manifest가 exe 버전을 본다).
//! 이 패키지는 데몬이 lib로도 링크하므로 리소스는 `rustc-link-arg-bins`로 이 패키지의 bin에만 넣는다
//! (winres는 lib에도 링크되어 saba-core의 버전 정보와 겹친다).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    windows::embed_version();
}

#[cfg(windows)]
mod windows {
    use std::path::PathBuf;

    pub fn embed_version() {
        let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let version = env!("CARGO_PKG_VERSION");
        let numeric = format!(
            "{},{},{},0",
            env!("CARGO_PKG_VERSION_MAJOR"),
            env!("CARGO_PKG_VERSION_MINOR"),
            env!("CARGO_PKG_VERSION_PATCH")
        );
        let rc_path = out_dir.join("saba-runner.rc");
        let rc = format!(
            r#"#pragma code_page(65001)
1 VERSIONINFO
FILETYPE 0x1
FILEFLAGSMASK 0x3f
FILEFLAGS 0x0
PRODUCTVERSION {numeric}
FILEVERSION {numeric}
FILEOS 0x40004
FILESUBTYPE 0x0
{{
BLOCK "StringFileInfo"
{{
BLOCK "000004b0"
{{
VALUE "ProductName", "Saba-chan"
VALUE "CompanyName", "Saba-chan"
VALUE "FileVersion", "{version}"
VALUE "ProductVersion", "{version}"
VALUE "FileDescription", "Saba-chan Server Runner"
VALUE "LegalCopyright", "Copyright (c) 2025 Saba-chan Contributors"
}}
}}
BLOCK "VarFileInfo" {{
VALUE "Translation", 0x0, 0x04b0
}}
}}
"#
        );
        std::fs::write(&rc_path, rc).expect("Failed to write .rc file");

        let Some(rc_exe) = find_rc_exe() else {
            println!("cargo:warning=rc.exe not found: saba-runner.exe gets no version info");
            return;
        };
        let res_path = out_dir.join("saba-runner.res");
        match std::process::Command::new(&rc_exe)
            .args(["/fo", res_path.to_str().unwrap(), rc_path.to_str().unwrap()])
            .output()
        {
            Ok(o) if o.status.success() => println!("cargo:rustc-link-arg-bins={}", res_path.display()),
            Ok(o) => println!(
                "cargo:warning=rc.exe failed (exit {}): {}{}",
                o.status,
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
            Err(e) => println!("cargo:warning=Failed to run rc.exe: {}", e),
        }
    }

    /// Windows SDK에서 rc.exe를 찾는다 (saba-core build.rs와 같은 방법)
    fn find_rc_exe() -> Option<PathBuf> {
        for root in [r"C:\Program Files (x86)\Windows Kits\10\bin", r"C:\Program Files\Windows Kits\10\bin"] {
            let Ok(dirs) = std::fs::read_dir(root) else { continue };
            let mut versions: Vec<_> = dirs
                .flatten()
                .filter(|e| e.path().is_dir() && e.file_name().to_string_lossy().starts_with("10."))
                .collect();
            versions.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
            if let Some(rc) = versions.iter().map(|v| v.path().join("x64").join("rc.exe")).find(|p| p.exists()) {
                return Some(rc);
            }
        }
        None
    }
}
