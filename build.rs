fn main() {
    embed_guides();
    #[cfg(windows)]
    println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    #[cfg(windows)]
    {
        use std::env;
        use std::path::PathBuf;

        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        // 저장소에 들어 있는 아이콘이다 (원본 icons/saba-core.png, scripts/make-icons.py로 만든다)
        let ico_path = PathBuf::from(&manifest_dir).join("icons").join("saba-core.ico");

        println!("cargo:rerun-if-changed=icons/saba-core.ico");
        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-changed=saba-core.manifest");
        // Segment Heap 매니페스트 (tests/windows_heap_manifest.rs): 잠깐 커졌다 풀린 힙을 OS에 돌려준다
        let heap_manifest = PathBuf::from(&manifest_dir).join("saba-core.manifest");

        // 버전 정보는 아이콘이 없어도 넣는다: 발행 때 versions.py manifest가 이 파일 버전을 대조한다
        let icon = ico_path.exists().then_some(ico_path.as_path());
        if icon.is_none() {
            // 아이콘 없는 데몬이 릴리스로 나가지 않게 멈춘다
            if env::var("PROFILE").as_deref() == Ok("release") {
                panic!("아이콘 파일이 없습니다: {}", ico_path.display());
            }
            println!(
                "cargo:warning=Icon file not found, embedding version info only: {}",
                ico_path.display()
            );
        }
        {
            // 1) .rc 파일 생성 (버전 정보 + 아이콘)
            let rc_path = out_dir.join("saba-core.rc");
            let icon_line = icon
                .map(|p| format!("1 ICON \"{}\"\n", p.to_str().unwrap().replace('\\', "\\\\")))
                .unwrap_or_default();
            // 파일 속성의 버전은 Cargo.toml(versions.toml에서 sync)을 따른다
            let version = env!("CARGO_PKG_VERSION");
            let numeric = format!(
                "{},{},{},0",
                env!("CARGO_PKG_VERSION_MAJOR"),
                env!("CARGO_PKG_VERSION_MINOR"),
                env!("CARGO_PKG_VERSION_PATCH")
            );
            let rc_content = format!(
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
VALUE "FileDescription", "Saba-chan Game Server Management Core"
VALUE "LegalCopyright", "Copyright (c) 2025 Saba-chan Contributors"
}}
}}
BLOCK "VarFileInfo" {{
VALUE "Translation", 0x0, 0x04b0
}}
}}
{icon_line}1 24 "{manifest}"
"#,
                icon_line = icon_line,
                manifest = heap_manifest.to_str().unwrap().replace('\\', "\\\\"),
                numeric = numeric,
                version = version
            );
            std::fs::write(&rc_path, &rc_content).expect("Failed to write .rc file");

            // 2) rc.exe로 .rc → .res 컴파일
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
                            // [lib]+[[bin]] 패키지에서는 rustc-link-arg-bins로
                            // 바이너리 타겟에만 .res를 전달해야 정상 임베딩됨
                            println!("cargo:rustc-link-arg-bins={}", res_path.display());
                            println!("cargo:warning=Windows resources compiled successfully (version info embedded)");
                        }
                        Ok(o) => {
                            let stderr = String::from_utf8_lossy(&o.stderr);
                            let stdout = String::from_utf8_lossy(&o.stdout);
                            println!("cargo:warning=rc.exe failed (exit {}): {}{}", o.status, stdout, stderr);
                            compile_with_winres(icon, &heap_manifest);
                        }
                        Err(e) => {
                            println!("cargo:warning=Failed to run rc.exe: {}", e);
                            compile_with_winres(icon, &heap_manifest);
                        }
                    }
                }
                None => {
                    println!("cargo:warning=rc.exe not found, using winres crate fallback");
                    compile_with_winres(icon, &heap_manifest);
                }
            }
        }
    }
}

/// Windows SDK에서 rc.exe를 탐색
#[cfg(windows)]
fn find_rc_exe() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    let sdk_roots = [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ];

    for root in &sdk_roots {
        let root_path = PathBuf::from(root);
        if !root_path.exists() {
            continue;
        }

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
        versions.sort_by_key(|b| std::cmp::Reverse(b.file_name()));

        for ver_dir in versions {
            let rc_path = ver_dir.path().join("x64").join("rc.exe");
            if rc_path.exists() {
                return Some(rc_path);
            }
        }
    }

    None
}

/// winres crate fallback (rc.exe 없는 환경용)
#[cfg(windows)]
fn compile_with_winres(icon: Option<&std::path::Path>, manifest: &std::path::Path) {
    let mut res = winres::WindowsResource::new();
    res.set_manifest_file(manifest.to_str().unwrap());
    if let Some(ico_path) = icon {
        res.set_icon(ico_path.to_str().unwrap());
    }
    res.set("ProductName", "Saba-chan");
    res.set("CompanyName", "Saba-chan");
    res.set("FileVersion", env!("CARGO_PKG_VERSION"));
    res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
    res.set("FileDescription", "Saba-chan Game Server Management Core");
    res.set("LegalCopyright", "Copyright (c) 2025 Saba-chan Contributors");
    match res.compile() {
        Ok(_) => println!("cargo:warning=winres fallback: resources embedded"),
        // 아이콘·버전 정보 없는 데몬이 릴리스로 나가지 않게 멈춘다
        Err(e) if std::env::var("PROFILE").as_deref() == Ok("release") => panic!("winres fallback failed: {e}"),
        Err(e) => println!("cargo:warning=winres fallback failed: {}", e),
    }
}

/// 앱에 함께 싣는 가이드 (`guides/<이름>/guide.toml` + `locales/<언어>.json`)를 데몬에 넣는다 (`src/guides.rs` `BUILTIN`).
/// 가이드를 더해도 코드를 고치지 않게 폴더를 훑어 목록을 만든다
fn embed_guides() {
    use std::fmt::Write as _;
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("guides");
    println!("cargo:rerun-if-changed=guides");
    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .map(|r| r.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.join("guide.toml").is_file()).collect())
        .unwrap_or_default();
    dirs.sort();
    let lit = |p: &std::path::Path| format!("include_str!({:?})", p.to_str().unwrap());
    let mut out = String::from("&[\n");
    for dir in dirs {
        let id = dir.file_name().unwrap().to_str().unwrap().to_string();
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut langs: Vec<_> = std::fs::read_dir(dir.join("locales"))
            .map(|r| r.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
            .unwrap_or_default();
        langs.sort();
        let _ = write!(out, "    ({id:?}, {}, &[", lit(&dir.join("guide.toml")));
        for lang in langs {
            println!("cargo:rerun-if-changed={}", lang.display());
            let _ = write!(out, "({:?}, {}), ", lang.file_stem().unwrap().to_str().unwrap(), lit(&lang));
        }
        out.push_str("]),\n");
    }
    out.push(']');
    std::fs::write(std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("builtin_guides.rs"), out).unwrap();
}
