//! 시험은 루프백에만 묶는다. 모든 주소(0.0.0.0·[::])에 묶으면 Windows가 빌드마다 이름이 바뀌는 시험 바이너리마다 방화벽 허용 창을 띄운다.
//! 모든 주소에 묶는 제품 코드(원격 데몬 포트, 포트 점검)는 주소를 따로 정하고 `bind` 줄에 쓰지 않는다 (원격 데몬은 시험에서 루프백)

use std::path::Path;

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target" && n != "node_modules") {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn nothing_binds_every_address_directly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "tests", "saba-chan-cli/src", "updater", "runner", "installer/core"] {
        rust_files(&root.join(dir), &mut files);
    }
    let me = Path::new(file!()).file_name().unwrap();
    let mut found = Vec::new();
    for file in &files {
        let rel = file.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if file.file_name() == Some(me) {
            continue;
        }
        let text = std::fs::read_to_string(file).unwrap_or_default();
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or_default();
            let wildcard = code.contains("UNSPECIFIED") || code.contains("\"0.0.0.0") || code.contains("\"[::]");
            if code.contains("bind(") && wildcard {
                found.push(format!("{rel}:{}", i + 1));
            }
        }
    }
    assert!(found.is_empty(), "bind tests to 127.0.0.1 (or ::1), not every address: {found:?}");
}
