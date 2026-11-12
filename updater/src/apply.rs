//! 받은 파일을 설치 폴더에 적용: 모듈·익스텐션 폴더 교체, 바이너리 교체, GUI 교체, 압축 해제

use super::*;

impl UpdateManager {
    /// 모듈 업데이트 적용 — 기존 zip 파일을 압축 해제하여 디렉터리에 배치
    pub(crate) async fn apply_module_update(&self, module_name: &str, staged_path: &str) -> Result<Vec<PathBuf>> {
        let target_dir = self.modules_dir.join(module_name);
        tracing::info!("[Updater] Applying module update: {} → {}", module_name, target_dir.display());
        let files = replace_dir(&target_dir, Path::new(staged_path), &self.staging_dir, &format!("{}_backup", module_name))?;
        tracing::info!("[Updater] Module '{}' updated successfully", module_name);
        Ok(files)
    }

    /// 익스텐션 업데이트 적용 — zip 압축 해제하여 extensions/ 디렉터리에 배치
    pub(crate) async fn apply_extension_update(&self, ext_name: &str, staged_path: &str) -> Result<Vec<PathBuf>> {
        let target_dir = self.resolve_ext_dir(ext_name);
        tracing::info!("[Updater] Applying extension update: {} → {}", ext_name, target_dir.display());
        let files = replace_dir(&target_dir, Path::new(staged_path), &self.staging_dir, &format!("{}_ext_backup", ext_name))?;
        tracing::info!("[Updater] Extension '{}' updated successfully", ext_name);
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

        // 대상 프로세스가 실행 중이라면 종료를 대기
        let exe = |name: &str| format!("{name}{}", std::env::consts::EXE_SUFFIX);
        let process_names: Vec<String> = match binary_name {
            n if n.contains("daemon") || n.contains("core") => vec![exe("saba-core")],
            n if n.contains("cli") => vec![exe("saba-chan-cli")],
            n if n.contains("gui") => GUI_EXE_NAMES.iter().map(|n| n.to_string()).collect(),
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

        tracing::info!("[Updater] Applying binary update: {} in {}", binary_name, exe_dir.display());

        let files = Self::extract_or_restore(staged, &exe_dir, binary_name)?;

        std::fs::remove_file(staged).ok();
        tracing::info!("[Updater] Binary '{}' updated", binary_name);
        // 공유 모듈(i18n.py, daemon_rcon.py)은 새 데몬이 자기 안에 든 판으로 맞춘다 (plugin::ensure_shared_modules)

        Ok(files)
    }

    /// 설치 폴더의 설치기 사본을 바꾼다. 릴리스의 설치기는 zip이 아닌 exe 그대로다.
    /// 복사가 실패하면 비켜 둔 원래 사본을 되돌린다 (제거기가 사라지면 안 된다)
    /// zip이 아닌 설치기(실행 파일 그대로)를 `target`에 둔다. 복사가 실패하면 비켜 둔 이전 사본을 되돌린다
    pub(crate) fn replace_bare_installer(staged: &Path, target: &Path) -> Result<()> {
        let backup = Self::move_aside(target)?;
        if let Err(e) = std::fs::copy(staged, target) {
            tracing::error!("[Updater] Replacing {} failed, restoring previous copy: {}", target.display(), e);
            // 비켜 두지 못했으면(Unix의 덮어쓰기) 지우지 않는다: 원래 사본이 그대로 남아 있을 수 있다
            if let Some(backup) = &backup {
                let _ = std::fs::remove_file(target);
                if let Err(re) = std::fs::rename(backup, target) {
                    tracing::error!("[Updater] Restore of {} failed: {}", target.display(), re);
                }
            }
            anyhow::bail!("Cannot replace {}: {}", target.display(), e);
        }
        Ok(())
    }

    pub(crate) fn apply_installer_update(&self, staged_path: &str) -> Result<Vec<PathBuf>> {
        let staged = Path::new(staged_path);
        if staged.extension().is_some_and(|e| e == "zip") {
            let files = Self::extract_zip_or_restore(staged, &self.install_root, "installer")?;
            std::fs::remove_file(staged).ok();
            return Ok(files);
        }
        let target = self.install_root.join(INSTALLER_EXE);
        Self::replace_bare_installer(staged, &target)?;
        std::fs::remove_file(staged).ok();
        tracing::info!("[Updater] Installer updated");
        Ok(vec![target])
    }

    /// zip을 풀며 덮어쓸 파일을 비켜 두고(실행 파일은 `.old`, 나머지는 `.update-bak`), 풀다가 실패하면
    /// 새로 생긴 파일을 지우고 비켜 둔 파일을 모두 되돌린다 (C13). 성공하면 `.update-bak`만 지운다
    /// (`.old`는 아직 떠 있는 프로세스가 쥐고 있을 수 있어 `cleanup_old_files`가 지운다)
    /// tar.gz도 같은 방식으로 되돌린다 (L3 b). 그 밖의 형식은 거절한다
    fn extract_or_restore(staged: &Path, dir: &Path, what: &str) -> Result<Vec<PathBuf>> {
        if staged.extension().is_some_and(|e| e == "zip") || Self::is_tar_gz(staged) {
            Self::extract_zip_or_restore(staged, dir, what)
        } else {
            anyhow::bail!("Unsupported archive format: {}", staged.display())
        }
    }

    fn extract_zip_or_restore(staged: &Path, dir: &Path, what: &str) -> Result<Vec<PathBuf>> {
        let mut replacing = Replacing::default();
        let result = replacing.extract(staged, dir);
        match &result {
            Ok(_) => replacing.keep(),
            Err(e) => {
                tracing::error!("[Updater] Applying {} failed, restoring previous files: {}", what, e);
                replacing.restore();
            }
        }
        result
    }

    /// GUI 업데이트 적용
    pub(crate) async fn apply_gui_update(&self, staged_path: &str) -> Result<Vec<PathBuf>> {
        let staged = Path::new(staged_path);

        // 설치 폴더 바로 안에 GUI exe가 있어야 그 자리에 푼다
        if let Some(portable_exe) = find_gui_exe(&self.install_root) {
            tracing::info!("[Updater] GUI portable exe detected at {}", portable_exe.display());
            let mut files = Self::extract_or_restore(staged, &self.install_root, "GUI")?;
            std::fs::remove_file(staged).ok();
            self.retire_other_gui_exes(&files);
            // 맥 앱 묶음: 새 묶음 정보·아이콘을 Contents 자리로 옮기고 기록할 경로도 옮긴 자리로 바꾼다
            match crate::app_bundle::place_files(&self.install_root) {
                Ok(moved) => {
                    for (from, to) in moved {
                        files.iter_mut().filter(|f| same_path(f, &from)).for_each(|f| *f = to.clone());
                    }
                }
                Err(e) => tracing::warn!("[Updater] App bundle files were not placed: {}", e),
            }
            tracing::info!("[Updater] GUI (portable exe) updated");
            return Ok(files);
        }

        // 설치 폴더에 GUI가 없다: 다른 폴더를 찾아 풀지 않는다
        anyhow::bail!("No GUI executable in the install folder {}; the GUI update was not applied", self.install_root.display())
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
            Component::Gui => find_gui_exe(&self.install_root).is_some(),
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
            Component::Installer => self.install_root.join(INSTALLER_EXE).exists(),
            Component::Locales => {
                // locales/ 디렉터리에 en/ 존재 확인
                self.install_root.join("locales").join("en").exists()
            }
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
            // 비켜 두지 못하면 덮어쓰지 않는다: 백업 없이 덮으면 새 데몬이 건강하지 않을 때 되돌릴 수 없다
            if let Err(e) = std::fs::rename(out_path, &backup) {
                tracing::error!("[Updater] Cannot move {} aside: {}", out_path.display(), e);
                anyhow::bail!("Cannot replace {}: {}", out_path.display(), e);
            }
            return Ok(Some(backup));
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

    /// ファイル名が .tar.gz で終わるか判定
    pub(crate) fn is_tar_gz(path: &Path) -> bool {
        path.file_name()
            .map(|n| n.to_string_lossy().ends_with(".tar.gz"))
            .unwrap_or(false)
    }

    /// tar.gz를 풀며 파일을 쓰기 직전에 `before_write`를 부른다 (`extract_zip`과 같은 꼴). 쓴 파일을 돌려준다
    fn extract_tar_gz_with(archive_path: &Path, target_dir: &Path, mut before_write: impl FnMut(&Path) -> Result<()>) -> Result<Vec<PathBuf>> {
        use flate2::read::GzDecoder;
        use tar::Archive;

        let file = std::fs::File::open(archive_path)?;
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);
        let mut written = Vec::new();

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

                before_write(&out_path)?;
                entry.unpack(&out_path)?;
                written.push(out_path.clone());

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

        Ok(written)
    }

    /// 경로가 saba-chan 바이너리인지 판별 (확장자 없는 비-바이너리 파일 오인 방지)
    pub(crate) fn is_known_binary(path: &Path) -> bool {
        const KNOWN_BINARIES: &[&str] = &[
            "saba-core",
            // saba-core zip에 같이 든다: 코어를 되돌릴 때 함께 되돌린다
            "saba-runner",
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

/// 덮어쓰며 비켜 둔 파일의 기록. 압축 파일 여러 개를 한 묶음으로 풀고(설치기의 덮어쓰기 설치),
/// 끝나면 `keep`, 도중에 실패하면 `restore`로 앞서 푼 것까지 되돌린다
#[derive(Default)]
pub struct Replacing {
    // (원래 경로, 백업, 성공하면 백업을 지울지)
    moved: Vec<(PathBuf, PathBuf, bool)>,
    created: Vec<PathBuf>,
}

impl Replacing {
    /// zip·tar.gz를 `dir`에 푼다. 실패해도 되돌리지 않는다 (부른 쪽이 `restore`)
    pub fn extract(&mut self, staged: &Path, dir: &Path) -> Result<Vec<PathBuf>> {
        let mut before_write = |out_path: &Path| self.before_write(out_path);
        if UpdateManager::is_tar_gz(staged) {
            UpdateManager::extract_tar_gz_with(staged, dir, &mut before_write)
        } else {
            extract_zip(staged, dir, &mut before_write)
        }
    }

    /// 압축하지 않은 파일 하나(Windows 릴리스의 설치기)를 `target`에 둔다
    pub fn place_file(&mut self, staged: &Path, target: &Path) -> Result<()> {
        self.before_write(target)?;
        std::fs::copy(staged, target).map(|_| ()).map_err(|e| anyhow::anyhow!("Cannot replace {}: {}", target.display(), e))
    }

    fn before_write(&mut self, out_path: &Path) -> Result<()> {
        // 같은 경로가 zip에 두 번 있으면 두 번째는 덮어쓰기만 한다. 다시 비켜 두면
        // 원래 백업이 새 파일로 바뀌어 되돌릴 수 없게 된다
        if self.moved.iter().any(|(p, _, _)| same_path(p, out_path)) || self.created.iter().any(|p| same_path(p, out_path)) {
            return Ok(());
        }
        if let Some(backup) = UpdateManager::move_aside(out_path)? {
            self.moved.push((out_path.to_path_buf(), backup, false));
        } else if out_path.is_file() {
            let mut name = out_path.file_name().unwrap_or_default().to_os_string();
            name.push(".update-bak");
            let backup = out_path.with_file_name(name);
            let _ = std::fs::remove_file(&backup);
            // 다른 프로세스가 지우기 공유 없이 잡고 있으면 이름을 못 바꾼다. 예전처럼 덮어쓰기만 한다
            match std::fs::rename(out_path, &backup) {
                Ok(()) => self.moved.push((out_path.to_path_buf(), backup, true)),
                Err(e) => tracing::warn!("[Updater] Cannot back up {}, overwriting in place: {}", out_path.display(), e),
            }
        } else {
            self.created.push(out_path.to_path_buf());
        }
        Ok(())
    }

    /// 새 파일을 그대로 둔다: `.update-bak`만 지운다 (`.old`는 `cleanup_old_files`가 지운다)
    pub fn keep(self) {
        for (_, backup, _) in self.moved.iter().filter(|m| m.2) {
            let _ = std::fs::remove_file(backup);
        }
    }

    /// 새로 생긴 파일을 지우고 비켜 둔 파일을 모두 되돌린다
    pub fn restore(self) {
        for path in &self.created {
            let _ = std::fs::remove_file(path);
        }
        for (original, backup, _) in self.moved.iter().rev() {
            let _ = std::fs::remove_file(original);
            if let Err(re) = std::fs::rename(backup, original) {
                tracing::error!("[Updater] Restore of {} failed: {}", original.display(), re);
            }
        }
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
        let out = std::process::Command::new(crate::process::system_tool("powershell"))
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

/// `target_dir`을 받은 zip(`staged`) 내용으로 바꾸고 푼 파일을 돌려준다. 도중에 실패하면 백업으로 되돌린다 (C13).
/// 백업은 `staging_dir/backup_name`에 둔다. `__pycache__`와 점(.)으로 시작하는 항목은 남긴다.
/// 업데이터(모듈·익스텐션·locales)와 설치기(모듈·익스텐션)가 함께 쓴다
pub fn replace_dir(target_dir: &Path, staged: &Path, staging_dir: &Path, backup_name: &str) -> Result<Vec<PathBuf>> {
    let backup_dir = staging_dir.join(backup_name);
    // 백업은 다 복사한 뒤 이름을 바꿔 둔다: `backup_dir`이 있으면 언제나 온전한 원래 내용이다
    let partial = staging_dir.join(format!("{}.partial", backup_name));
    if partial.exists() {
        std::fs::remove_dir_all(&partial)?;
    }
    if backup_dir.exists() {
        // 성공하면 지우는 백업이 남았다: 지난 교체가 도중에 꺼졌다. 폴더를 원래 내용으로 되돌리고 그 백업을 그대로 쓴다
        tracing::warn!(
            "[Updater] {} was left half-replaced by an interrupted update; restoring it first",
            target_dir.display()
        );
        if target_dir.exists() {
            clean_dir(target_dir)?;
        }
        copy_tree(&backup_dir, target_dir)?;
    } else if target_dir.exists() {
        copy_tree(target_dir, &partial)?;
        std::fs::rename(&partial, &backup_dir)?;
    }
    let had_target = backup_dir.exists();
    let result = (|| -> Result<Vec<PathBuf>> {
        if staged.extension().is_some_and(|e| e == "zip") {
            if had_target {
                clean_dir(target_dir)?;
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
                let _ = clean_dir(target_dir);
                match copy_tree(&backup_dir, target_dir) {
                    // 다 되돌렸으면 백업을 지운다: 남겨 두면 다음 교체가 "도중에 꺼진 교체"로 보고 그 사이 깔린 판을 덮는다
                    Ok(_) => {
                        let _ = std::fs::remove_dir_all(&backup_dir);
                    }
                    // 되돌리지 못했으면 백업을 남긴다: 다음 교체가 먼저 원래 내용으로 되돌린다
                    Err(re) => tracing::error!("[Updater] Restore of {} failed: {}", target_dir.display(), re),
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

/// 번역 묶음을 옆 폴더(`.locales-new`)에 다 푼 뒤 `locales/version`에 판 번호를 적고, 이름 바꾸기로 `dir`과 맞바꾼다.
/// 읽는 쪽은 옛 묶음이나 새 묶음 하나만 본다 (두 이름 바꾸기 사이 아주 잠깐 폴더가 없을 수 있다: 데몬·GUI·CLI는 내장본으로 읽는다).
/// 풀다 실패하면 `dir`은 건드리지 않는다. 쓴 파일(`dir` 안 경로)을 돌려준다
pub(crate) fn replace_locales(dir: &Path, staged: &Path, version: Option<&str>) -> Result<Vec<PathBuf>> {
    if !staged.extension().is_some_and(|e| e == "zip") {
        anyhow::bail!("Unsupported archive format: {}", staged.display());
    }
    settle_locales(dir);
    let (fresh, old) = locales_siblings(dir);
    if fresh.exists() {
        std::fs::remove_dir_all(&fresh)?;
    }
    std::fs::create_dir_all(&fresh)?;
    let written = extract_zip(staged, &fresh, |_| Ok(())).and_then(|files| {
        if let Some(v) = version {
            std::fs::write(fresh.join("version"), v)?;
        }
        Ok(files)
    });
    let written = match written {
        Ok(files) => files,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&fresh);
            return Err(e);
        }
    };
    if dir.exists() {
        std::fs::rename(dir, &old)?;
    }
    if let Err(e) = std::fs::rename(&fresh, dir) {
        if old.exists() {
            let _ = std::fs::rename(&old, dir);
        }
        let _ = std::fs::remove_dir_all(&fresh);
        anyhow::bail!("Cannot put the new locales in place: {}", e);
    }
    let _ = std::fs::remove_dir_all(&old);
    std::fs::remove_file(staged).ok();
    let mut files: Vec<PathBuf> = written.iter().filter_map(|f| f.strip_prefix(&fresh).ok().map(|rel| dir.join(rel))).collect();
    if version.is_some() {
        files.push(dir.join("version"));
    }
    Ok(files)
}

/// 맞바꾸다 끊긴 번역 묶음을 정리한다: `dir`이 없으면 옛 묶음(`.locales-old`)을 되돌리고, 남은 옆 폴더는 지운다
pub(crate) fn settle_locales(dir: &Path) {
    let (fresh, old) = locales_siblings(dir);
    if !dir.exists() && old.exists() {
        match std::fs::rename(&old, dir) {
            Ok(()) => tracing::warn!("[Updater] Restored {} (an earlier locales swap was interrupted)", dir.display()),
            Err(e) => tracing::warn!("[Updater] Cannot restore {}: {}", dir.display(), e),
        }
    }
    for leftover in [fresh, old] {
        if leftover.exists() {
            let _ = std::fs::remove_dir_all(&leftover);
        }
    }
}

fn locales_siblings(dir: &Path) -> (PathBuf, PathBuf) {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "locales".into());
    (dir.with_file_name(format!(".{name}-new")), dir.with_file_name(format!(".{name}-old")))
}

/// 묻지 않는 교체가 옮긴 원래 폴더에 남기는 지문 파일 (검사한 순간의 내용)
pub const SWAP_FINGERPRINT: &str = ".saba-swap-fingerprint.json";

/// 묻지 않는 모듈·익스텐션 교체. 원래 폴더를 비우지도, 교체 직후 지우지도 않는다.
///
/// 1. 새 판을 옆 폴더(`.<이름>.saba-new`)에 푼다
/// 2. 원래 폴더를 이름만 바꿔 `.<이름>.saba-old`로 옮긴다 (이 순간부터 원래 경로에 새로 쓰는 것은 옮긴 폴더에 섞이지 않는다)
/// 3. 옮긴 폴더의 지문을 한 번 떠서 `untouched`로 검사한다. 사용자가 넣거나 고친 것이 있으면 원래대로 되돌리고 멈춘다
/// 4. 새 판을 원래 이름으로 옮긴 뒤 그 지문을 옮긴 폴더에 적고(끝난 표시), 맨 위 점 항목(.git 등)을 새 폴더로 옮겨 준다
///
/// 옮긴 폴더는 남긴다: 교체 전에 열어 둔 파일에 나중에 저장하면 그 내용은 옮긴 폴더에 써진다(유닉스).
/// 다음 교체가 옮긴 폴더의 지문이 적어 둔 것과 같을 때만 지우고, 다르면 지우지 않고 그 위치를 오류에 적고 멈춘다.
/// 어느 단계가 실패해도 지우는 것은 새로 푼 판뿐이다. 도중에 꺼졌으면 다음 호출이 먼저 원래 폴더를 돌려놓는다.
/// 교체를 마친 뒤(지문이 있음) 제자리가 비었으면 사용자가 지운 것이다: 되살리지도 새로 깔지도 않는다
pub fn swap_dir_untouched(
    target_dir: &Path,
    staged: &Path,
    mut untouched: impl FnMut(&std::collections::BTreeMap<String, String>) -> bool,
) -> Result<Vec<PathBuf>> {
    let parent = target_dir.parent().ok_or_else(|| anyhow::anyhow!("{} has no parent folder", target_dir.display()))?;
    let name = target_dir.file_name().ok_or_else(|| anyhow::anyhow!("{} has no name", target_dir.display()))?.to_string_lossy().to_string();
    let aside = parent.join(format!(".{name}.saba-old"));
    let fresh = parent.join(format!(".{name}.saba-new"));
    let kept = |why: &str| anyhow::anyhow!("{}; the original folder is kept at {}", why, aside.display());
    let put_back = || -> Result<()> {
        std::fs::rename(&aside, target_dir).map_err(|e| kept(&format!("could not put the folder back ({e})")))?;
        let _ = std::fs::remove_file(target_dir.join(SWAP_FINGERPRINT));
        Ok(())
    };

    // 지난 교체가 남긴 것부터. 지문은 새 판을 넣은 뒤에 적으므로 지문이 있으면 그 교체는 끝났다:
    // 제자리가 비었어도(사용자가 지움) 되살리지 않는다. 제자리가 있으면 새 판이 들어간 것이니 남은 점 항목을 먼저 옮긴다
    if aside.exists() {
        let finished = aside.join(SWAP_FINGERPRINT).exists();
        if !target_dir.exists() && !finished {
            put_back()?;
        } else {
            if target_dir.exists() {
                carry_swap_dot_entries(&aside, target_dir).map_err(|e| kept(&e.to_string()))?;
            }
            drop_old_folder_if_unchanged(&aside).map_err(|e| kept(&e.to_string()))?;
        }
    }
    if fresh.exists() {
        std::fs::remove_dir_all(&fresh)?;
    }
    // 바꿔 끼우기만 한다: 그 사이 사용자가 지운 것을 새로 깔지 않는다
    if !target_dir.exists() {
        anyhow::bail!("{} is not installed; nothing to replace", target_dir.display());
    }

    let extracted = match extract_zip(staged, &fresh, |_| Ok(())) {
        Ok(files) => files,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&fresh);
            return Err(e);
        }
    };
    let files: Vec<PathBuf> = extracted.iter().filter_map(|f| f.strip_prefix(&fresh).ok().map(|rel| target_dir.join(rel))).collect();
    let drop_fresh = || {
        let _ = std::fs::remove_dir_all(&fresh);
    };

    if let Err(e) = std::fs::rename(target_dir, &aside) {
        drop_fresh();
        return Err(anyhow::anyhow!("could not move {} aside: {}", target_dir.display(), e));
    }
    // 지문은 한 번만 뜬다: 검사한 내용과 적어 두는 내용이 같다
    let checked = crate::integrity::dir_fingerprint(&aside).map_err(|e| anyhow::anyhow!(e)).and_then(|fp| {
        if !untouched(&fp) {
            anyhow::bail!("{} has files that were added or changed after install; left as it is", target_dir.display());
        }
        Ok(fp)
    });
    let fingerprint = match checked {
        Ok(fp) => fp,
        Err(e) => {
            drop_fresh();
            put_back()?;
            return Err(e);
        }
    };
    if let Err(e) = std::fs::rename(&fresh, target_dir) {
        // 되돌리지 못하면 새 판도 남긴다: 옮긴 폴더에 지문이 없으니 기동 정리가 다시 되돌린다
        put_back().map_err(|re| kept(&format!("could not put the new version in ({e}) nor the folder back ({re})")))?;
        drop_fresh();
        return Err(anyhow::anyhow!("could not put the new version of {} in: {}", target_dir.display(), e));
    }
    // 새 판이 들어갔다: 끝난 표시로 지문을 적는다. 적지 못하면 다음 교체가 옮긴 폴더를 지우지 않고 알린다
    let marked = serde_json::to_string(&fingerprint).map_err(anyhow::Error::from)
        .and_then(|text| crate::write_atomic(&aside.join(SWAP_FINGERPRINT), text.as_bytes()).map_err(Into::into));
    if let Err(e) = marked {
        tracing::warn!("[Updater] Could not record the fingerprint of {}: {}", aside.display(), e);
    }
    // 점 항목을 옮기지 못하면 옮긴 폴더에 남는다 (다음 교체가 먼저 다시 옮기고, 안 되면 지우지 않고 알린다)
    if let Err(e) = carry_dot_entries(&aside, target_dir) {
        tracing::warn!("[Updater] Left some dot entries in {}: {}", aside.display(), e);
    }
    Ok(files)
}

/// 새 판이 들어간(제자리가 있는) 교체가 옮기지 못한 점 항목을 새 폴더로 옮긴다. 기동 정리와 다음 교체가 부른다.
/// 새 판을 넣은 직후 지문을 적기 전에 꺼졌으면 지금 내용으로 지문을 적어 끝난 교체로 만든다(다음 교체가 정리한다)
pub fn carry_swap_dot_entries(aside: &Path, target_dir: &Path) -> Result<()> {
    if !target_dir.is_dir() || !aside.is_dir() {
        return Ok(());
    }
    if !aside.join(SWAP_FINGERPRINT).exists() {
        let fingerprint = crate::integrity::dir_fingerprint(aside).map_err(|e| anyhow::anyhow!(e))?;
        crate::write_atomic(&aside.join(SWAP_FINGERPRINT), serde_json::to_string(&fingerprint)?.as_bytes())?;
    }
    carry_dot_entries(aside, target_dir)
}

/// 옮긴 원래 폴더의 맨 위 점 항목(.git 등, 교체가 남기는 것)을 새 폴더로 옮긴다. 지문 파일은 남긴다
fn carry_dot_entries(aside: &Path, target_dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(aside)? {
        let entry = entry?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with('.') || name == SWAP_FINGERPRINT {
            continue;
        }
        let to = target_dir.join(&name);
        if to.exists() {
            anyhow::bail!("{} is in both the new version and the old folder", name.to_string_lossy());
        }
        std::fs::rename(entry.path(), &to)?;
    }
    Ok(())
}

/// 지난 교체가 남긴 원래 폴더를 지운다: 적어 둔 지문과 지금 내용이 같고, 옮기지 못한 점 항목이 없을 때만.
/// 그 사이 열린 파일로 저장했거나 무엇이든 바뀌었으면 지우지 않는다
fn drop_old_folder_if_unchanged(aside: &Path) -> Result<()> {
    let recorded: std::collections::BTreeMap<String, String> = crate::read_json(&aside.join(SWAP_FINGERPRINT))
        .map_err(|e| anyhow::anyhow!(e))?
        .ok_or_else(|| anyhow::anyhow!("a previous swap left a folder without its fingerprint"))?;
    let now = crate::integrity::dir_fingerprint(aside).map_err(|e| anyhow::anyhow!(e))?;
    if now != recorded {
        anyhow::bail!("a previous swap left a folder that changed after the swap");
    }
    for entry in std::fs::read_dir(aside)? {
        let name = entry?.file_name();
        if name.to_string_lossy().starts_with('.') && name != SWAP_FINGERPRINT {
            anyhow::bail!("a previous swap left {} in the old folder", name.to_string_lossy());
        }
    }
    std::fs::remove_dir_all(aside)?;
    Ok(())
}

/// `dir` 안을 비운다. `__pycache__`와 점(.)으로 시작하는 항목(.git 등)은 건드리지 않는다
fn clean_dir(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "__pycache__" || name_str.starts_with('.') {
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

/// `src` 폴더를 `dst`에 그대로 복사한다 (디렉터리 링크·junction은 따라가 내용을 복사한다). 실패한 경로를 오류에 적는다
pub(crate) fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    let fail = |p: &Path, e: std::io::Error| std::io::Error::new(e.kind(), format!("{}: {}", p.display(), e));
    std::fs::create_dir_all(dst).map_err(|e| fail(dst, e))?;
    for entry in std::fs::read_dir(src).map_err(|e| fail(src, e))? {
        let entry = entry.map_err(|e| fail(src, e))?;
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| fail(&from, e))?;
        }
    }
    Ok(())
}
