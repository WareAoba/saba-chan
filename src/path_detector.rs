use std::path::PathBuf;
use anyhow::Result;
use glob::glob;

#[allow(dead_code)]
pub struct PathDetector;

#[allow(dead_code)]
impl PathDetector {
    /// ・ｨ・・・､・菩乱 ・菩攪・・common_paths・川・ ・罹ｲ・・､嵂・甯護攵 ・ｾ・ｰ
    pub fn detect_server_path(
        process_name: &str,
        common_paths: &[String],
    ) -> Result<Option<PathBuf>> {
        for pattern in common_paths {
            // glob 甯ｨ奓ｴ 嶹菩棗
            if let Ok(paths) = glob(pattern) {
                for path in paths.flatten() {
                    if path.exists() && path.is_dir() {
                        // ・罷駕・・ｬ ・ｴ・川・ 嵓・｡懍┷・､ ・､嵂・甯護攵 ・ｾ・ｰ
                        let exe_path = path.join(process_name);
                        if exe_path.exists() {
                            tracing::info!("Found server at: {}", exe_path.display());
                            return Ok(Some(exe_path));
                        }
                    } else if path.exists() && path.is_file() {
                        // ・・・甯護攵 ・ｽ・懍攤 ・ｽ・ｰ
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

    /// ・ｰ・ｸ ・護桷 ・罹ｲ・・､・・・ｽ・・・・・(增ｬ・懍侃 嵓誤椨尞ｼ)
    pub fn get_default_game_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        #[cfg(target_os = "windows")]
        {
            // Windows: Steam ・ｰ・ｸ ・ｽ・・
            if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
                paths.push(PathBuf::from(program_files_x86).join("Steam").join("steamapps").join("common"));
            }

            // ・ｬ・ｩ・・・・・Steam ・ｼ・ｴ・誤洳・ｬ ・ｽ・罹豆
            for drive in &["C:", "D:", "E:", "F:"] {
                paths.push(PathBuf::from(format!("{}\\SteamLibrary\\steamapps\\common", drive)));
                paths.push(PathBuf::from(format!("{}\\Games", drive)));
            }

            // ・ｰ・､增ｬ夋・
            if let Some(userprofile) = std::env::var_os("USERPROFILE") {
                paths.push(PathBuf::from(userprofile).join("Desktop"));
            }
        }

        #[cfg(target_os = "linux")]
        {
            // Linux: Steam ・ｰ・ｸ ・ｽ・・
            if let Some(home) = std::env::var_os("HOME") {
                let home = PathBuf::from(home);
                paths.push(home.join(".steam").join("steam").join("steamapps").join("common"));
                paths.push(home.join(".local").join("share").join("Steam").join("steamapps").join("common"));
                
                // Flatpak Steam
                paths.push(home.join(".var").join("app").join("com.valvesoftware.Steam")
                    .join(".steam").join("steam").join("steamapps").join("common"));
            }
        }

        #[cfg(target_os = "macos")]
        {
            // macOS: Steam ・ｰ・ｸ ・ｽ・・
            if let Some(home) = std::env::var_os("HOME") {
                let home = PathBuf::from(home);
                paths.push(home.join("Library").join("Application Support").join("Steam")
                    .join("steamapps").join("common"));
            }
        }

        paths
    }
}
