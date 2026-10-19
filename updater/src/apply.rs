//! 받은 파일을 설치 폴더에 적용: 모듈·익스텐션 폴더 교체, 바이너리 교체, GUI 교체, 압축 해제

use super::*;

impl UpdateManager {
    /// 모듈 업데이트 적용 — 기존 zip 파일을 압축 해제하여 디렉터리에 배치
    pub(crate) async fn apply_module_update(&self, module_name: &str, staged_path: &str) -> Result<Vec<PathBuf>> {
        let target_dir = self.modules_dir.join(module_name);
        tracing::info!("[Updater] Applying module update: {} → {}", module_name, target_dir.display());
        let files = self.replace_dir(&target_dir, Path::new(staged_path), &format!("{}_backup", module_name))?;
        tracing::info!("[Updater] Module '{}' updated successfully", module_name);
        Ok(files)
    }

    /// 익스텐션 업데이트 적용 — zip 압축 해제하여 extensions/ 디렉터리에 배치
    pub(crate) async fn apply_extension_update(&self, ext_name: &str, staged_path: &str) -> Result<Vec<PathBuf>> {
        let target_dir = self.resolve_ext_dir(ext_name);
        tracing::info!("[Updater] Applying extension update: {} → {}", ext_name, target_dir.display());
        let files = self.replace_dir(&target_dir, Path::new(staged_path), &format!("{}_ext_backup", ext_name))?;
        tracing::info!("[Updater] Extension '{}' updated successfully", ext_name);
        Ok(files)
    }

    /// `target_dir`을 받은 zip 내용으로 바꾼다. 도중에 실패하면 백업으로 되돌린다 (C13).
    /// `__pycache__`와 점(.)으로 시작하는 항목은 남긴다
    pub(crate) fn replace_dir(&self, target_dir: &Path, staged: &Path, backup_name: &str) -> Result<Vec<PathBuf>> {
        let backup_dir = self.staging_dir.join(backup_name);
        let had_target = target_dir.exists();
        if had_target {
            if backup_dir.exists() {
                std::fs::remove_dir_all(&backup_dir)?;
            }
            self.copy_dir_recursive(target_dir, &backup_dir)?;
        }
        let result = (|| -> Result<Vec<PathBuf>> {
            if staged.extension().is_some_and(|e| e == "zip") {
                if had_target {
                    self.clean_module_dir(target_dir)?;
                }
                extract_zip(staged, target_dir, |_| Ok(()))
            } else {
                std::fs::copy(staged, target_dir)?;
                Ok(Vec::new())
            }
        })();
        let files = match result {
            Ok(files) => files,
            Err(e) => {
            tracing::error!("[Updater] Replacing {} failed, restoring backup: {}", target_dir.display(), e);
            if had_target {
                let _ = self.clean_module_dir(target_dir);
                if let Err(re) = self.copy_dir_recursive(&backup_dir, target_dir) {
                    tracing::error!("[Updater] Restore of {} failed: {}", target_dir.display(), re);
                }
            } else {
                // 새로 설치하던 것: 반쯤 풀린 폴더를 남기지 않는다
                let _ = std::fs::remove_dir_all(target_dir);
            }
            return Err(e);
            }
        };
        std::fs::remove_file(staged).ok();
        let _ = std::fs::remove_dir_all(&backup_dir);
        Ok(files)
    }

    pub(crate) fn rename_with_retry(from: &Path, to: &Path, max_retries: u32) -> Result<()> {
        // 기존 백업 파일이 있으면 먼저 삭제 시도
        if to.exists() {
            std::fs::remove_file(to).ok();
        }

        let mut last_err = None;
        for attempt in 0..=max_retries {
            match std::fs::rename(from, to) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_err = Some(e);
                    if attempt < max_retries {
                        let delay = std::time::Duration::from_millis(200 * 2u64.pow(attempt));
                        tracing::warn!(
                            "[Updater] rename {} -> {} failed (attempt {}/{}), retrying in {:?}...",
                            from.display(), to.display(), attempt + 1, max_retries + 1, delay
                        );
                        std::thread::sleep(delay);
                    }
                }
            }
        }
        Err(anyhow::anyhow!(
            "Failed to rename {} -> {} after {} attempts: {}",
            from.display(), to.display(), max_retries + 1,
            last_err.unwrap()
        ))
    }

    pub(crate) async fn apply_binary_update(&self, binary_name: &str, staged_path: &str) -> Result<Vec<PathBuf>> {
        let staged = Path::new(staged_path);

        let exe_dir = self.install_root.clone();

        // Windows: 대상 프로세스가 실행 중이라면 종료를 대기
        #[cfg(target_os = "windows")]
        {
            let process_names: Vec<&str> = match binary_name {
                n if n.contains("daemon") || n.contains("core") => vec!["saba-core.exe"],
                n if n.contains("cli") => vec!["saba-chan-cli.exe"],
                n if n.contains("gui") => GUI_EXE_NAMES.to_vec(),
                _ => vec![],
            };
            for proc in &process_names {
                if ProcessChecker::is_running(proc) {
                    tracing::info!("[Updater] Waiting for {} to exit before applying update...", proc);
                    let exited = ProcessChecker::wait_for_exit(proc, 15).await;
                    if !exited {
                        tracing::warn!("[Updater] {} did not exit within timeout, attempting update anyway", proc);
                    }
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            let process_names: Vec<&str> = match binary_name {
                n if n.contains("daemon") || n.contains("core") => vec!["saba-core"],
                n if n.contains("cli") => vec!["saba-chan-cli"],
                n if n.contains("gui") => vec!["saba-chan-gui"],
                _ => vec![],
            };
            for proc in &process_names {
                if ProcessChecker::is_running(proc) {
                    tracing::info!("[Updater] Waiting for {} to exit before applying update...", proc);
                    let exited = ProcessChecker::wait_for_exit(proc, 15).await;
                    if !exited {
                        tracing::warn!("[Updater] {} did not exit within timeout, attempting update anyway", proc);
                    }
                }
            }
        }

        tracing::info!("[Updater] Applying binary update: {} in {}", binary_name, exe_dir.display());
        let mut files = Vec::new();

        if staged.extension().map(|e| e == "zip").unwrap_or(false) {
            files = Self::extract_zip_or_restore(staged, &exe_dir, binary_name)?;
        } else if Self::is_tar_gz(staged) {
            Self::extract_tar_gz(staged, &exe_dir)?;
        } else {
            anyhow::bail!("Unsupported archive format: {}", staged.display());
        }

        std::fs::remove_file(staged).ok();
        tracing::info!("[Updater] Binary '{}' updated", binary_name);
        // 공유 모듈(i18n.py, daemon_rcon.py)은 새 데몬이 자기 안에 든 판으로 맞춘다 (plugin::ensure_shared_modules)

        Ok(files)
    }

    /// zip을 풀며 덮어쓸 파일을 비켜 두고(실행 파일은 `.old`, 나머지는 `.update-bak`), 풀다가 실패하면
    /// 새로 생긴 파일을 지우고 비켜 둔 파일을 모두 되돌린다 (C13). 성공하면 `.update-bak`만 지운다
    /// (`.old`는 아직 떠 있는 프로세스가 쥐고 있을 수 있어 `cleanup_old_files`가 지운다)
    fn extract_zip_or_restore(staged: &Path, dir: &Path, what: &str) -> Result<Vec<PathBuf>> {
        // (원래 경로, 백업, 성공하면 백업을 지울지)
        let mut moved: Vec<(PathBuf, PathBuf, bool)> = Vec::new();
        let mut created: Vec<PathBuf> = Vec::new();
        let result = extract_zip(staged, dir, |out_path| {
            // 같은 경로가 zip에 두 번 있으면 두 번째는 덮어쓰기만 한다. 다시 비켜 두면
            // 원래 백업이 새 파일로 바뀌어 되돌릴 수 없게 된다
            if moved.iter().any(|(p, _, _)| same_path(p, out_path)) || created.iter().any(|p| same_path(p, out_path)) {
                return Ok(());
            }
            if let Some(backup) = Self::move_aside(out_path)? {
                moved.push((out_path.to_path_buf(), backup, false));
            } else if out_path.is_file() {
                let mut name = out_path.file_name().unwrap_or_default().to_os_string();
                name.push(".update-bak");
                let backup = out_path.with_file_name(name);
                let _ = std::fs::remove_file(&backup);
                // 다른 프로세스가 지우기 공유 없이 잡고 있으면 이름을 못 바꾼다. 예전처럼 덮어쓰기만 한다
                match std::fs::rename(out_path, &backup) {
                    Ok(()) => moved.push((out_path.to_path_buf(), backup, true)),
                    Err(e) => tracing::warn!("[Updater] Cannot back up {}, overwriting in place: {}", out_path.display(), e),
                }
            } else {
                created.push(out_path.to_path_buf());
            }
            Ok(())
        });
        match &result {
            Ok(_) => {
                for (_, backup, _) in moved.iter().filter(|m| m.2) {
                    let _ = std::fs::remove_file(backup);
                }
            }
            Err(e) => {
                tracing::error!("[Updater] Applying {} failed, restoring previous files: {}", what, e);
                for path in &created {
                    let _ = std::fs::remove_file(path);
                }
                for (original, backup, _) in moved.iter().rev() {
                    let _ = std::fs::remove_file(original);
                    if let Err(re) = std::fs::rename(backup, original) {
                        tracing::error!("[Updater] Restore of {} failed: {}", original.display(), re);
                    }
                }
            }
        }
        result
    }

    /// GUI 업데이트 적용
    pub(crate) async fn apply_gui_update(&self, staged_path: &str) -> Result<Vec<PathBuf>> {
        let staged = Path::new(staged_path);

        // 설치 폴더 바로 안에 GUI exe가 있으면 (portable exe, 또는 0.1.1의 Electron 폴더) 그 자리에 푼다
        if let Some(portable_exe) = find_gui_exe(&self.install_root) {
            tracing::info!("[Updater] GUI portable exe detected at {}", portable_exe.display());
            let mut files = Vec::new();
            if staged.extension().map(|e| e == "zip").unwrap_or(false) {
                files = Self::extract_zip_or_restore(staged, &self.install_root, "GUI")?;
            } else if Self::is_tar_gz(staged) {
                Self::extract_tar_gz(staged, &self.install_root)?;
            } else {
                anyhow::bail!("Unsupported archive format: {}", staged.display());
            }
            std::fs::remove_file(staged).ok();
            self.retire_other_gui_exes(&files);
            tracing::info!("[Updater] GUI (portable exe) updated");
            return Ok(files);
        }

        // Directory mode fallback (unpacked Electron / dev)
        let gui_dir = self.find_gui_directory()?;
        let extract_dir = {
            let build_dir = gui_dir.join("build");
            if build_dir.exists() {
                build_dir
            } else {
                let res_build = gui_dir.join("resources").join("app").join("build");
                if res_build.exists() {
                    res_build
                } else {
                    if gui_dir.join("src").exists() && gui_dir.join("package.json").exists() {
                        anyhow::bail!(
                            "GUI directory appears to be a source tree ({}). Refusing to overwrite.",
                            gui_dir.display()
                        );
                    }
                    gui_dir.clone()
                }
            }
        };
        tracing::info!("[Updater] Applying GUI update to dir: {}", extract_dir.display());
        if !staged.extension().map(|e| e == "zip").unwrap_or(false) {
            anyhow::bail!("Unsupported archive format: {}", staged.display());
        }
        let files = extract_zip(staged, &extract_dir, |_| Ok(()))?;
        std::fs::remove_file(staged).ok();
        tracing::info!("[Updater] GUI updated successfully");
        Ok(files)
    }

    /// 새 GUI exe 이름이 예전과 다르면(0.1.1의 `Saba-chan.exe` → `saba-chan-gui.exe`) 예전 exe를
    /// `.exe.old`로 비켜 두고(`cleanup_old_files`가 나중에 지운다) 바로가기를 새 exe로 돌린다.
    /// 그대로 두면 바로가기가 예전 GUI를 띄운다
    pub(crate) fn retire_other_gui_exes(&self, written: &[PathBuf]) {
        for (old, new_exe) in Self::retire_other_exes(&self.install_root, written, GUI_EXE_NAMES) {
            retarget_shortcuts(&old, &new_exe);
        }
    }

    /// `names` 중 이번에 쓰지 않은 exe를 비켜 두고 (예전 exe, 새 exe) 쌍을 돌려준다
    pub(crate) fn retire_other_exes(root: &Path, written: &[PathBuf], names: &[&str]) -> Vec<(PathBuf, PathBuf)> {
        let is_named = |f: &Path| names.iter().any(|n| f.file_name().is_some_and(|f| f.eq_ignore_ascii_case(n)));
        let Some(new_exe) = written.iter().find(|f| f.parent().is_some_and(|d| same_path(d, root)) && is_named(f)) else {
            return Vec::new();
        };
        let mut retired = Vec::new();
        for name in names {
            let old = root.join(name);
            if same_path(&old, new_exe) || !old.is_file() {
                continue;
            }
            match Self::move_aside_binary(&old) {
                Ok(()) => {
                    tracing::info!("[Updater] Retired old GUI exe {}", old.display());
                    retired.push((old, new_exe.clone()));
                }
                Err(e) => tracing::warn!("[Updater] Could not retire old GUI exe {}: {}", old.display(), e),
            }
        }
        retired
    }

    // ─────── 유틸리티 ────────────────────────────────────────────────────────────────────────

    pub(crate) fn find_gui_directory(&self) -> Result<PathBuf> {
        // 1) 설치 폴더 바로 안 (배포 환경 기본)
        if find_gui_exe(&self.install_root).is_some() {
            tracing::info!("[Updater] GUI exe found at install_root");
            return Ok(self.install_root.clone());
        }

        // 2) exe 기준 (exe가 install_root와 다른 위치일 때)
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                if find_gui_exe(dir).is_some() {
                    return Ok(dir.to_path_buf());
                }
            }
        }

        // 3) CWD 기준 (개발 환경 — Electron/소스 구조)
        for p in &["saba-chan-gui", "../saba-chan-gui"] {
            let path = PathBuf::from(p);
            if path.exists() {
                return Ok(path);
            }
        }

        anyhow::bail!("GUI directory not found (checked: install_root={}, exe_dir, cwd)", self.install_root.display())
    }

    pub(crate) fn clean_module_dir(&self, dir: &Path) -> Result<()> {
        for entry in std::fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // __pycache__, .git 등은 건드리지 않음
            if name_str == "__pycache__"
                || name_str.starts_with('.')
            {
                continue;
            }

            if path.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        }
        Ok(())
    }

    pub(crate) fn copy_dir_recursive(&self, src: &Path, dst: &Path) -> Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)?.flatten() {
            let path = entry.path();
            let dest = dst.join(entry.file_name());
            if path.is_dir() {
                self.copy_dir_recursive(&path, &dest)?;
            } else {
                std::fs::copy(&path, &dest)?;
            }
        }
        Ok(())
    }

    // ══════════════════════════════════════════════════════
    // 초기 설치 관련 메서드
    // ══════════════════════════════════════════════════════

    /// 컴포넌트가 설치되어 있는지 확인
    pub fn is_component_installed(&self, component: &Component) -> bool {
        match component {
            Component::CoreDaemon => {
                // 코어 데몬은 무조건 설치된 것으로 판단
                true
            }
            Component::Cli => {
                let exe_name = if cfg!(windows) { "saba-chan-cli.exe" } else { "saba-chan-cli" };
                self.install_root.join(exe_name).exists()
                    || PathBuf::from("saba-chan-cli").join("target").exists()
            }
            Component::Gui => {
                // Portable exe OR directory
                if find_gui_exe(&self.install_root).is_some() {
                    return true;
                }
                self.find_gui_directory().ok().map(|d| d.exists()).unwrap_or(false)
            }
            Component::Module(name) => {
                let module_dir = self.modules_dir.join(name);
                module_dir.join("module.toml").exists()
            }
            Component::Extension(name) => {
                let ext_dir = self.resolve_ext_dir(name);
                ext_dir.join("manifest.json").exists() || ext_dir.join("__init__.py").exists()
            }
            Component::Updater => {
                // 업데이터 exe 존재 확인
                let exe_name = if cfg!(windows) { "saba-chan-updater.exe" } else { "saba-chan-updater" };
                self.install_root.join(exe_name).exists()
            }
            Component::Locales => {
                // locales/ 디렉터리에 en/ 존재 확인
                self.install_root.join("locales").join("en").exists()
            }
        }
    }

    // ─────── 설치 위치 유틸리티 ────────────────────────────────────────────────────────────────────────

    /// 컴포넌트의 설치 디렉터리를 결정
    pub(crate) fn resolve_install_dir(&self, component: &Component, manifest_dir: Option<&str>) -> PathBuf {
        // manifest의 install_dir가 지정되면 install_root 하위로 결합
        if let Some(dir) = manifest_dir {
            // Extension인 경우 디렉토리명의 하이픈을 언더스코어로 변환 (Python import 호환)
            if matches!(component, Component::Extension(_)) {
                let safe = dir.replace('-', "_");
                return self.install_root.join(safe);
            }
            return self.install_root.join(dir);
        }

        // 기본 매핑
        match component {
            Component::CoreDaemon => self.install_root.clone(),
            Component::Cli => self.install_root.clone(),
            Component::Gui => self.install_root.clone(),
            Component::Module(name) => self.modules_dir.join(name),
            Component::Extension(name) => self.resolve_ext_dir(name),
            Component::Updater => self.install_root.clone(),
            Component::Locales => self.install_root.join("locales"),
        }
    }

    /// 덮어쓸 실행 파일을 `.old`로 옮긴다. Windows는 실행 중인 exe를 지울 수
    /// 없지만 이름은 바꿀 수 있다.
    pub(crate) fn move_aside_binary(out_path: &Path) -> Result<()> {
        Self::move_aside(out_path).map(|_| ())
    }

    /// `move_aside_binary`와 같고, 옮긴 백업 경로를 돌려준다 (옮기지 않았으면 None)
    pub(crate) fn move_aside(out_path: &Path) -> Result<Option<PathBuf>> {
        if !out_path.exists() {
            return Ok(None);
        }
        if out_path.extension().map(|e| e == "exe").unwrap_or(false) {
            let backup = Self::free_backup_path(out_path);
            if let Err(e) = Self::rename_with_retry(out_path, &backup, 5) {
                tracing::error!("[Updater] Cannot replace {}: {}", out_path.display(), e);
                anyhow::bail!("Cannot replace {}: {}. Is the process still running?", out_path.display(), e);
            }
            return Ok(Some(backup));
        } else if cfg!(unix) && Self::is_known_binary(out_path) {
            let backup = out_path.with_extension("old");
            if std::fs::rename(out_path, &backup).is_ok() {
                return Ok(Some(backup));
            }
        }
        Ok(None)
    }

    /// `<exe>.old`가 아직 떠 있는 프로세스에 잡혀 지울 수 없으면(예: 지난 업데이트 뒤로
    /// 계속 도는 서버의 saba-runner) `<exe>.old.1`, `.old.2` … 중 비울 수 있는 이름을 쓴다.
    fn free_backup_path(out_path: &Path) -> PathBuf {
        let usable = |p: &Path| !p.exists() || std::fs::remove_file(p).is_ok();
        let first = out_path.with_extension("exe.old");
        if usable(&first) {
            return first;
        }
        (1..100)
            .map(|n| out_path.with_extension(format!("exe.old.{n}")))
            .find(|p| usable(p))
            .unwrap_or(first)
    }

    /// zip/tar.gz(또는 단일 파일)을 대상 디렉터리에 압축 해제
    pub(crate) async fn extract_to_directory(&self, staged: &Path, target_dir: &Path) -> Result<Vec<PathBuf>> {
        std::fs::create_dir_all(target_dir)?;

        if staged.extension().map(|e| e == "zip").unwrap_or(false) {
            return extract_zip(staged, target_dir, |_| Ok(()));
        } else if Self::is_tar_gz(staged) {
            Self::extract_tar_gz(staged, target_dir)?;
        } else {
            // 단일 파일인 경우 target_dir 내부에 복사
            let file_name = staged.file_name().unwrap_or_default();
            let dest = target_dir.join(file_name);
            std::fs::copy(staged, &dest)?;
            return Ok(vec![dest]);
        }

        Ok(Vec::new())
    }

    /// ファイル名が .tar.gz で終わるか判定
    pub(crate) fn is_tar_gz(path: &Path) -> bool {
        path.file_name()
            .map(|n| n.to_string_lossy().ends_with(".tar.gz"))
            .unwrap_or(false)
    }

    /// tar.gz アーカイブを展開
    pub(crate) fn extract_tar_gz(archive_path: &Path, target_dir: &Path) -> Result<()> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        let file = std::fs::File::open(archive_path)?;
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);

        for entry in archive.entries()? {
            let mut entry = entry?;
            let entry_path = entry.path()?.into_owned();

            // 경로 탐색 공격 방지
            if entry_path.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            }) {
                tracing::warn!("[Updater] Skipping potentially unsafe path: {}", entry_path.display());
                continue;
            }

            let kind = entry.header().entry_type();
            // 링크 엔트리는 만들지 않는다 (뒤 엔트리가 그 링크를 따라 밖에 쓸 수 있다)
            if !(kind.is_dir() || kind.is_file()) {
                tracing::warn!("[Updater] Skipping non-file entry: {}", entry_path.display());
                continue;
            }
            reject_links_below(target_dir, &entry_path)?;
            let out_path = target_dir.join(&entry_path);

            if kind.is_dir() {
                std::fs::create_dir_all(&out_path)?;
            } else {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                // 기존 바이너리를 .old로 백업
                if out_path.exists() {
                    if out_path.extension().map(|e| e == "exe").unwrap_or(false) {
                        let backup = out_path.with_extension("exe.old");
                        let _ = Self::rename_with_retry(&out_path, &backup, 5);
                    } else if cfg!(unix) && Self::is_known_binary(&out_path) {
                        let backup = out_path.with_extension("old");
                        let _ = std::fs::rename(&out_path, &backup);
                    }
                }

                entry.unpack(&out_path)?;

                // Linux: 실행 파일에 실행 권한 부여
                #[cfg(unix)]
                if Self::is_known_binary(&out_path) {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = std::fs::metadata(&out_path)?.permissions();
                    perms.set_mode(perms.mode() | 0o755);
                    std::fs::set_permissions(&out_path, perms)?;
                }
            }
        }

        Ok(())
    }

    /// 경로가 saba-chan 바이너리인지 판별 (확장자 없는 비-바이너리 파일 오인 방지)
    pub(crate) fn is_known_binary(path: &Path) -> bool {
        const KNOWN_BINARIES: &[&str] = &[
            "saba-core",
            "saba-chan-cli",
            "saba-chan-gui",
            "saba-chan-updater",
            "saba-chan-installer",
        ];
        path.file_name()
            .and_then(|n| n.to_str())
            .map(|name| KNOWN_BINARIES.contains(&name))
            .unwrap_or(false)
    }
}

/// 바탕화면·시작 메뉴에서 `old`를 가리키는 바로가기를 `new`로 돌린다 (Windows만, 실패해도 적용은 성공)
fn retarget_shortcuts(old: &Path, new: &Path) {
    #[cfg(windows)]
    {
        // 경로는 환경 변수로 넘긴다 (스크립트 문자열에 넣지 않는다)
        const SCRIPT: &str = r#"$sh = New-Object -ComObject WScript.Shell
foreach ($f in 'Desktop','Programs','CommonDesktopDirectory','CommonPrograms') {
  $dir = [Environment]::GetFolderPath($f)
  if (-not $dir) { continue }
  Get-ChildItem -LiteralPath $dir -Filter *.lnk -Recurse -ErrorAction SilentlyContinue | ForEach-Object {
    try {
      $l = $sh.CreateShortcut($_.FullName)
      if ($l.TargetPath -ieq $env:SABA_OLD_EXE) { $l.TargetPath = $env:SABA_NEW_EXE; $l.Save() }
    } catch {}
  }
}"#;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
            .env("SABA_OLD_EXE", old)
            .env("SABA_NEW_EXE", new)
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if let Err(e) = out {
            tracing::warn!("[Updater] Could not update shortcuts to {}: {}", new.display(), e);
        }
    }
    #[cfg(not(windows))]
    let _ = (old, new);
}
