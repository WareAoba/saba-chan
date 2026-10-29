//! 맥 앱 묶음(`Saba-chan.app`). 실행 파일은 `Contents/MacOS`(설치 폴더)에 두고, 묶음 정보(Info.plist)와 아이콘은
//! GUI zip에 함께 실어(package-linux.py) 설치기·업데이터가 `Contents` 자리로 옮긴다. 제거기는 묶음째 지운다

use std::path::{Path, PathBuf};

/// 앱 묶음 이름 (.dmg와 설치기가 같은 이름으로 만든다)
pub const BUNDLE_NAME: &str = "Saba-chan.app";

/// GUI zip에 든 묶음 정보와 아이콘 (설치 폴더에 풀린다)
pub const PLIST_FILE: &str = "Info.plist";
pub const ICON_FILE: &str = "saba-chan.icns";

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
    let icon = install_dir.join(ICON_FILE);
    if icon.is_file() {
        let resources = contents.join("Resources");
        std::fs::create_dir_all(&resources)?;
        let placed = resources.join(ICON_FILE);
        std::fs::rename(&icon, &placed)?;
        moved.push((icon, placed));
    }
    Ok(moved)
}

/// 실행 파일 폴더를 지운 뒤 남은 묶음 껍데기를 지운다: 묶음 정보·아이콘, 빈 `Contents`와 묶음 폴더.
/// 그 밖의 파일이 남아 있으면 폴더는 둔다
pub fn remove_shell(install_dir: &Path) {
    let Some(bundle) = bundle_of(install_dir) else { return };
    let contents = bundle.join("Contents");
    let _ = std::fs::remove_file(contents.join(PLIST_FILE));
    let _ = std::fs::remove_file(contents.join("Resources").join(ICON_FILE));
    let _ = std::fs::remove_dir(contents.join("Resources"));
    let _ = std::fs::remove_dir(&contents);
    let _ = std::fs::remove_dir(&bundle);
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
        assert_eq!(place_files(&macos).unwrap().len(), 2);
        assert_eq!(std::fs::read(bundle.join("Contents").join(PLIST_FILE)).unwrap(), b"plist");
        assert_eq!(std::fs::read(bundle.join("Contents/Resources").join(ICON_FILE)).unwrap(), b"icns");
        assert!(!macos.join(PLIST_FILE).exists() && !macos.join(ICON_FILE).exists(), "MacOS holds executables only");

        std::fs::remove_dir_all(&macos).unwrap();
        remove_shell(&macos);
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
}
