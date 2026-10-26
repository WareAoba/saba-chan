//! 사용자 데이터 폴더 밖에 사바쨩이 남기는 것. 제거기는 무엇을 남기든 여기 적힌 것을 모두 지운다.
//! 새 흔적(임시 파일·폴더, 앱 데이터 폴더)을 만들면 여기에 더한다:
//! `every_temp_name_in_the_source_is_listed`가 소스에서 임시 폴더 이름을 모아 빠진 것이 있으면 실패한다.
//! 데이터 폴더 안은 `saba_chan_updater_lib::uninstall_entry`가 나눈다

use std::path::{Path, PathBuf};

/// 임시 폴더 바로 아래, 이름이 정해진 것
pub const TEMP_NAMES: &[&str] = &[
    "saba-chan-installer",
    "saba-chan-updater",
    "saba-chan-temp",
    // 설치기 기록 (`log_file_path`), 업데이터 기록
    "saba-chan-installer.log",
    "saba-updater.log",
];

/// 임시 폴더 바로 아래 `<접두어><pid>`인 것. 그 PID가 아직 돌면 둔다
pub const TEMP_PID_PREFIXES: &[&str] = &[crate::release::WORK_DIR_PREFIX];

/// 창 앱(Tauri)의 식별자. WebView 데이터·캐시가 이 이름의 폴더에 쌓인다
pub const APP_IDENTIFIERS: &[&str] = &[saba_chan_updater_lib::notify_app_id::GUI_APP_ID, "com.saba-chan.installer", "com.saba-chan.updater"];

/// 지금 도는 제거기의 WebView 폴더 식별자: 끝날 때까지 잠겨 있어 정리 스크립트가 지운다
pub const SELF_IDENTIFIER: &str = "com.saba-chan.installer";

/// 제거기가 끝날 때까지 쓰고 있는 기록 파일 (`log_file_path`): 정리 스크립트가 지운다
pub const SELF_LOG: &str = "saba-chan-installer.log";

/// 흔적이 놓이는 곳
#[derive(Debug, Clone)]
pub struct Roots {
    pub temp: PathBuf,
    /// 앱 데이터 폴더들 (Windows `%LOCALAPPDATA%`, 리눅스 `~/.local/share`·`~/.cache`)
    pub app_data: Vec<PathBuf>,
    /// 리눅스 캐시 (`~/.cache`): 업데이트 받기 폴더 `saba-chan/updates`가 여기 있다 (`constants::resolve_staging_dir`)
    pub cache: Option<PathBuf>,
    /// 리눅스 바로가기 폴더 (`~/.local/share/applications`)
    pub applications: Option<PathBuf>,
}

impl Roots {
    pub fn current() -> Roots {
        let mut app_data: Vec<PathBuf> = [dirs::data_local_dir(), dirs::cache_dir()].into_iter().flatten().collect();
        app_data.dedup();
        let unix = !cfg!(windows);
        Roots {
            temp: std::env::temp_dir(),
            app_data,
            cache: dirs::cache_dir().filter(|_| unix),
            applications: dirs::data_dir().filter(|_| unix).map(|d| d.join("applications")),
        }
    }
}

/// 지울 흔적 (있는 것만). `running`은 지금 도는 PID 목록: 못 읽으면(None) PID가 붙은 것은 고르지 않는다.
/// `skip_self`면 이 제거기가 쥐고 있는 것은 뺀다 (정리 스크립트 몫, [`held_by_self`])
pub fn found(roots: &Roots, running: Option<&[u32]>, skip_self: bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> =
        TEMP_NAMES.iter().filter(|n| !(skip_self && **n == SELF_LOG)).map(|n| roots.temp.join(n)).collect();
    if let Some(running) = running {
        for entry in std::fs::read_dir(&roots.temp).into_iter().flatten().flatten() {
            let path = entry.path();
            if pid_of(&path).is_some_and(|pid| !running.contains(&pid)) {
                out.push(path);
            }
        }
    }
    for dir in &roots.app_data {
        out.extend(APP_IDENTIFIERS.iter().filter(|id| !(skip_self && **id == SELF_IDENTIFIER)).map(|id| dir.join(id)));
    }
    if let Some(cache) = &roots.cache {
        out.push(cache.join(saba_chan_updater_lib::constants::APP_NAME));
    }
    if let Some(apps) = &roots.applications {
        out.push(apps.join("saba-chan.desktop"));
    }
    out.retain(|p| p.symlink_metadata().is_ok());
    out.sort();
    out.dedup();
    out
}

/// 이 제거기가 끝날 때까지 쥐고 있는 흔적: WebView 폴더와 기록 파일 (있는 것만)
pub fn held_by_self(roots: &Roots) -> Vec<PathBuf> {
    roots.app_data.iter().map(|d| d.join(SELF_IDENTIFIER)).chain([roots.temp.join(SELF_LOG)]).filter(|p| p.exists()).collect()
}

/// `<접두어><pid>`의 pid
fn pid_of(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    TEMP_PID_PREFIXES.iter().find_map(|prefix| {
        let pid = name.strip_prefix(prefix)?;
        pid.bytes().all(|b| b.is_ascii_digit()).then(|| pid.parse().ok()).flatten()
    })
}

/// 흔적을 지우고, 지우지 못한 것을 돌려준다
pub fn remove(roots: &Roots, running: Option<&[u32]>, skip_self: bool) -> Vec<PathBuf> {
    for path in found(roots, running, skip_self) {
        tracing::info!("[Uninstall] Removing trace: {:?}", path);
        if path.is_dir() {
            let _ = crate::uninstall::remove_dir_robust(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    found(roots, running, skip_self)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(base: &Path) -> Roots {
        Roots {
            temp: base.join("temp"),
            app_data: vec![base.join("local")],
            cache: Some(base.join("cache")),
            applications: Some(base.join("apps")),
        }
    }

    /// 알려진 흔적을 모두 만들고 지우면, 남의 것과 아직 도는 작업 폴더·이 제거기의 WebView 폴더만 남는다
    #[test]
    fn every_known_trace_is_removed_and_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        let r = roots(tmp.path());
        let mk = |p: PathBuf, dir: bool| {
            std::fs::create_dir_all(if dir { &p } else { p.parent().unwrap() }).unwrap();
            if dir {
                std::fs::write(p.join("x"), b"").unwrap();
            } else {
                std::fs::write(&p, b"").unwrap();
            }
        };
        for n in TEMP_NAMES {
            mk(r.temp.join(n), !n.ends_with(".log"));
        }
        mk(r.temp.join("saba-chan-installer-41"), true);
        mk(r.temp.join("saba-chan-installer-42"), true); // 아직 도는 설치기
        mk(r.temp.join("saba-chan-installer-backup"), true); // 숫자가 아니면 남의 것
        mk(r.temp.join("other.txt"), false);
        for id in APP_IDENTIFIERS {
            mk(r.app_data[0].join(id).join("EBWebView"), true);
        }
        mk(r.app_data[0].join("com.other.app"), true);
        mk(r.cache.as_ref().unwrap().join("saba-chan").join("updates"), true);
        mk(r.applications.as_ref().unwrap().join("saba-chan.desktop"), false);

        assert!(remove(&r, Some(&[42]), true).is_empty());

        let mut left: Vec<String> = walkdir(tmp.path());
        left.sort();
        let mut expected = [
                "apps",
                "cache",
                "local",
                "local/com.installer-self",
                "local/com.other.app",
                "temp",
                "temp/other.txt",
                "temp/saba-chan-installer.log",
                "temp/saba-chan-installer-42",
                "temp/saba-chan-installer-backup",
            ]
            .map(|s| s.replace("com.installer-self", SELF_IDENTIFIER));
        expected.sort();
        assert_eq!(left, expected);
        assert_eq!(held_by_self(&r), [r.app_data[0].join(SELF_IDENTIFIER), r.temp.join(SELF_LOG)]);
        // 정리 스크립트가 없는 명령창 제거는 자기 폴더도 바로 지운다
        assert!(remove(&r, Some(&[42]), false).is_empty());
        assert!(held_by_self(&r).is_empty());
        // 프로세스 목록을 못 읽으면 PID가 붙은 작업 폴더는 건드리지 않는다
        assert!(found(&r, None, false).is_empty());
        assert!(r.temp.join("saba-chan-installer-42").exists());
    }

    fn walkdir(base: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for top in std::fs::read_dir(base).unwrap().flatten() {
            let name = top.file_name().to_string_lossy().into_owned();
            out.push(name.clone());
            for child in std::fs::read_dir(top.path()).into_iter().flatten().flatten() {
                out.push(format!("{}/{}", name, child.file_name().to_string_lossy()));
            }
        }
        out
    }

    /// 소스에서 `temp_dir().join("이름")`·`temp_dir().join(format!("접두어{}", ...))`으로 만드는 임시 항목은
    /// 모두 위 목록에 있다 (테스트 코드는 뺀다: 파일에서 처음 나오는 `#[cfg(test)]` 뒤는 보지 않는다)
    #[test]
    fn every_temp_name_in_the_source_is_listed() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut names = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if path.is_dir() {
                    if !["target", "node_modules", ".git", "tests"].contains(&name.as_str()) {
                        stack.push(path);
                    }
                } else if name.ends_with(".rs") {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    let code = text.split("#[cfg(test)]").next().unwrap_or("");
                    for (i, _) in code.match_indices("temp_dir().join(") {
                        let rest = &code[i + "temp_dir().join(".len()..];
                        let rest = rest.trim_start().strip_prefix("format!(").unwrap_or(rest).trim_start();
                        if let Some(lit) = rest.strip_prefix('"').and_then(|r| r.split('"').next()) {
                            // `format!("{}{}", PREFIX, pid)`: 앞의 `{}`는 그 자리의 상수 이름으로 읽는다
                            let name = match lit.strip_prefix("{}") {
                                Some(tail) => {
                                    let after = rest[lit.len() + 2..].trim_start().trim_start_matches(',').trim_start();
                                    let ident: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                                    format!("{ident}{}", tail.split('{').next().unwrap_or(""))
                                }
                                None => lit.split('{').next().unwrap_or(lit).to_string(),
                            };
                            names.push((path.strip_prefix(&root).unwrap_or(&path).display().to_string(), name));
                        }
                    }
                }
            }
        }
        assert!(names.iter().any(|(_, n)| n == "saba-chan-installer.log"), "the scan finds temp names: {names:?}");
        // 정리 스크립트는 스스로 지운다. `test-isolation` 기능의 테스트 루트는 테스트 빌드에만 있다
        let listed = |n: &str| {
            TEMP_NAMES.contains(&n) || n == "WORK_DIR_PREFIX" || n == "saba-chan-cleanup.cmd" || n.contains("-test-")
        };
        let missing: Vec<_> = names.iter().filter(|(_, n)| !listed(n)).collect();
        assert!(missing.is_empty(), "temp names not in traces.rs (the uninstaller would leave them): {missing:?}");
    }
}
