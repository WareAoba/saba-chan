use std::path::{Path, PathBuf};
use anyhow::Result;
use glob::glob;

pub struct PathDetector;

impl PathDetector {
    /// 모듈 설정에 정의된 common_paths에서 서버 실행 파일 찾기
    pub fn detect_server_path(
        process_name: &str,
        common_paths: &[String],
    ) -> Result<Option<PathBuf>> {
        for pattern in common_paths {
            // glob 패턴 확장
            if let Ok(paths) = glob(pattern) {
                for path in paths.flatten() {
                    if path.exists() && path.is_dir() {
                        // 디렉토리 내에서 프로세스 실행 파일 찾기
                        let exe_path = path.join(process_name);
                        if exe_path.exists() {
                            tracing::info!("Found server at: {}", exe_path.display());
                            return Ok(Some(exe_path));
                        }
                    } else if path.exists() && path.is_file() {
                        // 직접 파일 경로인 경우
                        if path.file_name().and_then(|n| n.to_str()) == Some(process_name) {
                            tracing::info!("Found server at: {}", path.display());
                            return Ok(Some(path));
                        }
                    }
                }
            }
        }

        tracing::warn!("Could not find server executable: {}", process_name);
        Ok(None)
    }

    /// 기본 게임 서버 설치 경로 검색
    pub fn get_default_game_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        // Steam 기본 경로
        if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
            paths.push(PathBuf::from(program_files_x86).join("Steam").join("steamapps").join("common"));
        }

        // 사용자 지정 Steam 라이브러리 경로들
        for drive in &["C:", "D:", "E:", "F:"] {
            paths.push(PathBuf::from(format!("{}\\SteamLibrary\\steamapps\\common", drive)));
            paths.push(PathBuf::from(format!("{}\\Games", drive)));
        }

        // 데스크탑
        if let Some(userprofile) = std::env::var_os("USERPROFILE") {
            paths.push(PathBuf::from(userprofile).join("Desktop"));
        }

        paths
    }
}
