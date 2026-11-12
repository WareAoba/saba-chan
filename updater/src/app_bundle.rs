//! 맥 앱 묶음(`Saba-chan.app`). 실행 파일은 `Contents/MacOS`(설치 폴더)에 두고, 묶음 정보(Info.plist)와 아이콘은
//! GUI zip에 함께 실어(package-linux.py) 설치기·업데이터가 `Contents` 자리로 옮긴다. 제거기는 묶음째 지운다

use std::path::{Path, PathBuf};

/// 앱 묶음 이름 (.dmg와 설치기가 같은 이름으로 만든다)
pub const BUNDLE_NAME: &str = "Saba-chan.app";

/// GUI zip에 든 묶음 정보와 아이콘 (설치 폴더에 풀린다). 아이콘은 macOS 26의 Assets.car(라이트·다크·틴트)와
/// 옛 macOS용 icns 두 파일이다 (package-macos.py가 Icon Composer 원본에서 컴파일한다)
pub const PLIST_FILE: &str = "Info.plist";
pub const ICON_FILE: &str = "saba-chan.icns";
pub const ASSETS_FILE: &str = "Assets.car";
/// `Contents/Resources`에 놓는 파일
const RESOURCE_FILES: [&str; 2] = [ICON_FILE, ASSETS_FILE];

/// 설치 폴더가 앱 묶음의 실행 파일 폴더(`<이름>.app/Contents/MacOS`)면 그 묶음
pub fn bundle_of(install_dir: &Path) -> Option<PathBuf> {
    let contents = install_dir.parent().filter(|_| install_dir.file_name().is_some_and(|n| n == "MacOS"))?;
    let bundle = contents.parent().filter(|_| contents.file_name().is_some_and(|n| n == "Contents"))?;
    bundle.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")).then(|| bundle.to_path_buf())
}

/// 설치 폴더에 풀린 묶음 정보와 아이콘을 `Contents/Info.plist`·`Contents/Resources`로 옮긴다
/// (`Contents/MacOS`에는 실행 파일만 둔다: 서명이 다른 파일을 거부한다). 있던 것은 새것으로 바뀐다.
/// 묶음 안 설치가 아니거나 GUI가 없는 구성(파일이 없음)이면 아무것도 하지 않는다. 옮긴 (원래 자리, 새 자리)를 돌려준다
pub fn place_files(install_dir: &Path) -> std::io::Result<Vec<(PathBuf, PathBuf)>> {
    let mut moved = Vec::new();
    let Some(contents) = bundle_of(install_dir).map(|b| b.join("Contents")) else { return Ok(moved) };
    let plist = install_dir.join(PLIST_FILE);
    if !plist.is_file() {
        return Ok(moved);
    }
    let placed = contents.join(PLIST_FILE);
    std::fs::rename(&plist, &placed)?;
    moved.push((plist, placed));
    let resources = contents.join("Resources");
    for name in RESOURCE_FILES {
        let file = install_dir.join(name);
        if file.is_file() {
            std::fs::create_dir_all(&resources)?;
            let placed = resources.join(name);
            std::fs::rename(&file, &placed)?;
            moved.push((file, placed));
        }
    }
    Ok(moved)
}

/// 실행 파일 폴더를 지운 뒤 남은 묶음 껍데기를 지운다: 묶음 정보·아이콘, .dmg 묶음의 임시 서명(`_CodeSignature`),
/// 빈 `Contents`와 묶음 폴더. 그 밖의 파일이 남아 있으면 폴더는 둔다
pub fn remove_shell(install_dir: &Path) {
    let Some(bundle) = bundle_of(install_dir) else { return };
    let contents = bundle.join("Contents");
    let _ = std::fs::remove_file(contents.join("_CodeSignature").join("CodeResources"));
    let _ = std::fs::remove_dir(contents.join("_CodeSignature"));
    let _ = std::fs::remove_file(contents.join(PLIST_FILE));
    for name in RESOURCE_FILES {
        let _ = std::fs::remove_file(contents.join("Resources").join(name));
    }
    let _ = std::fs::remove_dir(contents.join("Resources"));
    let _ = std::fs::remove_dir(&contents);
    let _ = std::fs::remove_dir(&bundle);
}

/// 묶음 앱을 켤 때 GUI가 창보다 먼저 정하는 일 (처음 켠 것인지)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// 준비가 끝난 자리: 그대로 연다
    Ready,
    /// 이 자리는 처음이지만 모듈이 있다 (예전 판에서 올라왔거나 앱을 옮겼다): 창 없이 명령어 링크만 고치고 연다
    Relink,
    /// 진짜 처음: 설치기 준비 화면을 띄운다
    Setup,
    /// 디스크 이미지나 격리된 임시 자리에서 열었다: 응용 프로그램 폴더로 옮기라고 알린다
    MoveFirst,
}

/// 디스크 이미지나 macOS가 격리해 연 임시 자리(App Translocation)에서 열렸는가.
/// 이런 자리는 다음에 열면 없거나 바뀌어 명령어 링크가 끊긴다. `/Volumes`에는 외장 디스크도 붙으므로
/// 읽기 전용으로 붙은 볼륨(디스크 이미지)만 임시 자리로 본다
pub fn in_temporary_place(install_dir: &Path) -> bool {
    temporary_place(install_dir, || read_only_mount(install_dir))
}

fn temporary_place(install_dir: &Path, read_only: impl FnOnce() -> bool) -> bool {
    install_dir.components().any(|c| c.as_os_str() == "AppTranslocation") || (install_dir.starts_with("/Volumes") && read_only())
}

/// 그 폴더가 읽기 전용으로 붙은 볼륨에 있는가 (`.dmg`는 읽기 전용으로 붙는다)
#[cfg(target_os = "macos")]
fn read_only_mount(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else { return false };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: path는 NUL로 끝나는 C 문자열이고 stat은 쓸 수 있는 statvfs다
    (unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0) && (stat.f_flag & libc::ST_RDONLY) != 0
}

#[cfg(not(target_os = "macos"))]
fn read_only_mount(_dir: &Path) -> bool {
    false
}

/// 준비는 설치기가 마칠 때 설치 목록(`installs.json`)에 이 자리를 적는 것으로 끝난다. 파일이 있는지만 보지 않는 것은
/// 앱 안 업데이트도 설치 기록 파일을 만들기 때문이다. 준비 도중 창을 닫으면 목록에 없어 다음에 다시 준비한다.
/// 설치 목록이 생기기 전 판에서 앱 안 업데이트로 올라온 자리는 업데이터가 이 자리에 GUI를 놓았다고 `records`
/// (`installed-files.json`)에 적어 두므로 처음으로 보지 않는다 (모듈이 하나도 없어도)
pub fn launch_for(install_dir: &Path, installs_file: &Path, modules_dir: &Path, records: &Path) -> Launch {
    if in_temporary_place(install_dir) {
        return Launch::MoveFirst;
    }
    let listed = crate::uninstall_entry::listed_installs(installs_file);
    if listed.iter().any(|d| crate::same_path(Path::new(d), install_dir)) {
        return Launch::Ready;
    }
    let has_modules = std::fs::read_dir(modules_dir)
        .map(|it| it.flatten().any(|e| e.path().join("module.toml").is_file()))
        .unwrap_or(false);
    if has_modules || crate::integrity::placed_under(records, "gui", install_dir) { Launch::Relink } else { Launch::Setup }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_executable_folder_of_an_app_bundle_names_the_bundle() {
        assert_eq!(bundle_of(Path::new("/Users/a/Applications/Saba-chan.app/Contents/MacOS")), Some("/Users/a/Applications/Saba-chan.app".into()));
        assert_eq!(bundle_of(Path::new("/Users/a/.local/share/saba-chan")), None);
        assert_eq!(bundle_of(Path::new("/Users/a/Saba-chan/Contents/MacOS")), None, "not an .app");
        assert_eq!(bundle_of(Path::new("/Users/a/Saba-chan.app/MacOS")), None);
    }

    // 설치는 GUI zip의 묶음 파일을 제자리에 놓고, 제거는 실행 파일 폴더를 지운 뒤 묶음까지 비운다
    #[test]
    fn the_bundle_files_are_placed_and_the_empty_bundle_is_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join(BUNDLE_NAME);
        let macos = bundle.join("Contents").join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(macos.join(PLIST_FILE), b"plist").unwrap();
        std::fs::write(macos.join(ICON_FILE), b"icns").unwrap();
        std::fs::write(macos.join(ASSETS_FILE), b"car").unwrap();
        assert_eq!(place_files(&macos).unwrap().len(), 3);
        assert_eq!(std::fs::read(bundle.join("Contents").join(PLIST_FILE)).unwrap(), b"plist");
        assert_eq!(std::fs::read(bundle.join("Contents/Resources").join(ICON_FILE)).unwrap(), b"icns");
        assert_eq!(std::fs::read(bundle.join("Contents/Resources").join(ASSETS_FILE)).unwrap(), b"car");
        for f in [PLIST_FILE, ICON_FILE, ASSETS_FILE] {
            assert!(!macos.join(f).exists(), "MacOS holds executables only: {f}");
        }

        std::fs::remove_dir_all(&macos).unwrap();
        remove_shell(&macos);
        assert!(!bundle.exists());
    }

    // .dmg 앱 묶음은 임시 서명(`Contents/_CodeSignature`)을 지닌다: 제거가 빈 껍데기 앱을 남기지 않는다
    #[test]
    fn the_signature_of_a_dmg_bundle_goes_with_the_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join(BUNDLE_NAME);
        let signature = bundle.join("Contents").join("_CodeSignature");
        std::fs::create_dir_all(&signature).unwrap();
        std::fs::write(signature.join("CodeResources"), b"seal").unwrap();
        std::fs::write(bundle.join("Contents").join(PLIST_FILE), b"plist").unwrap();
        remove_shell(&bundle.join("Contents").join("MacOS"));
        assert!(!bundle.exists());
    }

    #[test]
    fn a_bundle_with_other_files_is_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join(BUNDLE_NAME);
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        std::fs::write(bundle.join("Contents").join(PLIST_FILE), b"plist").unwrap();
        std::fs::write(bundle.join("Contents").join("notes.txt"), b"mine").unwrap();
        remove_shell(&bundle.join("Contents").join("MacOS"));
        assert!(bundle.join("Contents/notes.txt").is_file());
        assert!(!bundle.join("Contents").join(PLIST_FILE).exists());
    }

    // 헤드리스 설치(GUI zip이 없음)는 묶음 정보를 만들지 않는다
    #[test]
    fn without_the_gui_files_nothing_is_placed() {
        let tmp = tempfile::tempdir().unwrap();
        let macos = tmp.path().join(BUNDLE_NAME).join("Contents").join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        assert!(place_files(&macos).unwrap().is_empty());
        assert!(!macos.parent().unwrap().join(PLIST_FILE).exists());
        assert!(place_files(tmp.path()).is_ok(), "not inside a bundle");
    }

    /// 처음 켤 때만 준비 화면을 띄운다. 준비를 마친 자리는 그대로, 모듈은 있는데 처음 보는 자리(옮긴 앱·예전 판)는 링크만 고친다.
    /// 디스크 이미지에서 연 앱은 옮기라고 먼저 알린다
    #[test]
    fn launch_shows_setup_only_on_a_real_first_run() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Applications/Saba-chan.app/Contents/MacOS");
        let installs = tmp.path().join("installs.json");
        let modules = tmp.path().join("modules");
        std::fs::create_dir_all(&modules).unwrap();

        let records = tmp.path().join("installed-files.json");

        assert_eq!(launch_for(&app, &installs, &modules, &records), Launch::Setup);
        std::fs::create_dir_all(modules.join("minecraft")).unwrap();
        std::fs::write(modules.join("minecraft/module.toml"), "").unwrap();
        assert_eq!(launch_for(&app, &installs, &modules, &records), Launch::Relink);
        crate::uninstall_entry::remember_install(&installs, &app).unwrap();
        assert_eq!(launch_for(&app, &installs, &modules, &records), Launch::Ready);

        // 디스크 이미지(읽기 전용 볼륨)만 임시 자리다. 외장 디스크에 둔 앱은 그대로 쓴다
        let dmg = Path::new("/Volumes/Saba-chan/Saba-chan.app/Contents/MacOS");
        assert!(temporary_place(dmg, || true));
        assert!(!temporary_place(Path::new("/Volumes/External/Applications/Saba-chan.app/Contents/MacOS"), || false));
        let translocated = Path::new("/private/var/folders/x/T/AppTranslocation/ABC/d/Saba-chan.app/Contents/MacOS");
        assert_eq!(launch_for(translocated, &installs, &modules, &records), Launch::MoveFirst);
    }

    /// 설치 목록이 생기기 전 판에서 앱 안 업데이트로 올라온 자리는 모듈이 없어도 처음이 아니다:
    /// 업데이터가 이 자리에 GUI를 놓았다고 적어 두었으면 준비 화면 없이 링크만 고친다
    #[test]
    fn an_app_updated_in_place_is_not_a_first_run() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("Applications/Saba-chan.app/Contents/MacOS");
        std::fs::create_dir_all(&app).unwrap();
        let gui = app.join("saba-chan-gui");
        std::fs::write(&gui, b"gui").unwrap();
        let (installs, modules, records) = (tmp.path().join("installs.json"), tmp.path().join("modules"), tmp.path().join("installed-files.json"));

        // 다른 자리에 놓은 GUI 기록은 이 자리와 상관없다
        let elsewhere = tmp.path().join("Other.app/Contents/MacOS");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("saba-chan-gui"), b"gui").unwrap();
        crate::integrity::record_install_at(&records, "gui", &[elsewhere.join("saba-chan-gui")]).unwrap();
        assert_eq!(launch_for(&app, &installs, &modules, &records), Launch::Setup);

        crate::integrity::record_install_at(&records, "gui", &[gui]).unwrap();
        assert_eq!(launch_for(&app, &installs, &modules, &records), Launch::Relink);
    }
}
