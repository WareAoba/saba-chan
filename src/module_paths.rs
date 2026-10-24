use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::instance::ServerInstance;
use crate::supervisor::module_loader::ModuleMetadata;

/// 없는 대상은 가장 가까운 기존 조상까지 검사한다. 허용 루트 안을 가리키는 링크도 거부한다
pub(crate) fn checked_path(path: &Path) -> Option<PathBuf> {
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    let path = std::path::absolute(path).ok()?;
    if !crate::sandbox::plain_local_path(&path) {
        return None;
    }
    let mut current = PathBuf::new();
    let mut missing = Vec::new();
    let mut ancestor = None;
    for component in path.components() {
        current.push(component.as_os_str());
        if !current.has_root() {
            continue;
        }
        if !missing.is_empty() {
            missing.push(component.as_os_str().to_os_string());
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return None;
                }
                #[cfg(windows)]
                crate::sandbox::appcontainer::check_existing_path(&current).ok()?;
                ancestor = Some(current.clone());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => missing.push(component.as_os_str().to_os_string()),
            Err(_) => return None,
        }
    }
    let mut real = std::fs::canonicalize(ancestor?).ok()?;
    for component in missing {
        real.push(component);
    }
    Some(real)
}

fn relative_to(path: &Path, root: &Path) -> Option<PathBuf> {
    let mut parts = path.components();
    for expected in root.components() {
        let actual = parts.next()?;
        if actual != expected {
            return None;
        }
    }
    Some(parts.collect())
}

fn is_daemon_data_path(path: &Path, data_dir: &Path) -> bool {
    let Some(relative) = relative_to(path, data_dir) else { return false };
    if relative.as_os_str().is_empty() { return true; }
    relative.components().next().is_some_and(|part| {
        ["config.json", "settings.json", ".ipc_token"].iter().any(|name| part.as_os_str().to_string_lossy().eq_ignore_ascii_case(name))
    })
}

pub fn is_inside(path: &Path, roots: &[PathBuf]) -> bool {
    let Some(path) = checked_path(path) else { return false };
    roots
        .iter()
        .filter_map(|root| checked_path(root))
        .any(|root| relative_to(&path, &root).is_some())
}

pub(crate) fn data_root_parts(pattern: &str) -> Option<(&str, PathBuf)> {
    let rest = pattern.strip_prefix('{')?;
    let (key, suffix) = rest.split_once('}')?;
    if key.is_empty() || key.contains(['{', '/', '\\', ':']) || suffix.contains(['{', '}', ':']) {
        return None;
    }
    if !suffix.is_empty() && !suffix.starts_with(['/', '\\']) {
        return None;
    }
    let suffix = suffix.trim_start_matches(['/', '\\']);
    if suffix.split(['/', '\\']).any(|part| part == "..") {
        return None;
    }
    let suffix = PathBuf::from(suffix);
    suffix
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        .then_some((key, suffix))
}

pub(crate) fn server_dir(instance: &ServerInstance, instance_dir: &Path, metadata: Option<&ModuleMetadata>) -> PathBuf {
    instance
        .working_dir
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let subdir = metadata
                .and_then(|m| m.install.as_ref())
                .map(|i| i.install_subdir.as_deref().unwrap_or("server"))
                .unwrap_or("");
            instance_dir.join(subdir)
        })
}

#[derive(Clone, Debug)]
pub struct Boundary {
    base: PathBuf,
    roots: Vec<PathBuf>,
    instance_dir: Option<PathBuf>,
}

impl Boundary {
    pub(crate) fn for_instance(
        instance: &ServerInstance,
        instance_dir: &Path,
        metadata: Option<&ModuleMetadata>,
    ) -> Self {
        let base = server_dir(instance, instance_dir, metadata);
        let mut roots = vec![base.clone(), instance_dir.to_path_buf()];
        roots.extend(
            crate::supervisor::Supervisor::instance_data_dirs(instance, instance_dir)
                .into_iter()
                .map(PathBuf::from),
        );
        for pattern in metadata.into_iter().flat_map(|m| &m.data_roots) {
            let Some((key, suffix)) = data_root_parts(pattern) else {
                continue;
            };
            let verified = metadata.is_some_and(|m| {
                let entry = crate::plugin::resolve_modules_dir().join(&instance.module_name).join(&m.entry);
                crate::sandbox::trust_of_entry(&entry) == saba_chan_updater_lib::integrity::Trust::Verified
            });
            let root = if key == "home" && verified {
                std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
            } else if key == "home" {
                None
            } else if instance.user_settings.contains(key) || instance.imported_settings.contains(key) {
                instance
                    .module_settings
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|v| !v.trim().is_empty())
                    .map(PathBuf::from)
            } else {
                None
            };
            if let Some(root) = root.filter(|r| r.is_absolute()) {
                roots.push(root.join(suffix));
            }
        }
        Self {
            base,
            roots,
            instance_dir: Some(instance_dir.to_path_buf()),
        }
    }

    pub(crate) fn for_install(base: &Path, instance: Option<Self>) -> Self {
        let mut boundary = instance.unwrap_or_else(|| Self {
            base: base.to_path_buf(),
            roots: vec![],
            instance_dir: None,
        });
        boundary.base = base.to_path_buf();
        boundary.roots.push(base.to_path_buf());
        boundary
    }

    pub(crate) fn resolve(&self, path: &Path) -> Option<PathBuf> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base.join(path)
        };
        let path = std::path::absolute(path).ok()?;
        let real = checked_path(&path)?;
        let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
        let data_dir = checked_path(&data_dir)?;
        if is_daemon_data_path(&real, &data_dir) {
            return None;
        }
        if let Some(instance_dir) = &self.instance_dir {
            let instance = checked_path(instance_dir)?;
            if let Some(relative) = relative_to(&real, &instance) {
                let first = relative.components().next()?;
                if crate::supervisor::is_daemon_entry(&first.as_os_str().to_string_lossy()) {
                    return None;
                }
            }
        }
        self.roots
            .iter()
            .filter_map(|root| checked_path(root))
            .any(|root| relative_to(&real, &root).is_some())
            .then_some(path)
    }

    pub(crate) fn returned_paths(&self, result: &mut Value, module: &str, function: &str) -> anyhow::Result<()> {
        for field in ["install_path", "executable_path"] {
            let Some(path) = result.get(field).and_then(Value::as_str) else {
                continue;
            };
            if path.trim().is_empty() {
                result[field] = Value::Null;
                continue;
            }
            let resolved = self
                .resolve(Path::new(path))
                .ok_or_else(|| outside(module, function, path))?;
            result[field] = serde_json::json!(resolved);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn daemon_data_files_are_never_in_a_component_boundary() {
        let data = Path::new("C:/Users/test/AppData/Roaming/saba-chan");
        assert!(is_daemon_data_path(data, data));
        for name in ["config.json", "settings.json", ".ipc_token"] {
            assert!(is_daemon_data_path(&data.join(name), data));
        }
        assert!(!is_daemon_data_path(&data.join("instances/id/server/world"), data));
    }

    #[cfg(windows)]
    #[test]
    fn contract_paths_windows_does_not_merge_unicode_sibling_names() {
        let tmp = tempfile::tempdir().unwrap();
        let allowed = tmp.path().join("straße");
        let sibling = tmp.path().join("strasse");
        std::fs::create_dir_all(&allowed).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let secret = sibling.join("secret.txt");
        std::fs::write(&secret, "outside").unwrap();
        assert!(!is_inside(&secret, std::slice::from_ref(&allowed)));
        std::fs::create_dir_all(tmp.path().join("AsciiRoot/world")).unwrap();
        assert!(is_inside(&tmp.path().join("ASCIIROOT/WORLD"), &[tmp.path().join("asciiroot")]));
    }

    #[test]
    fn contract_paths_external_roots_need_a_user_choice_or_migration_import() {
        let tmp = tempfile::tempdir().unwrap();
        let instance_dir = tmp.path().join("instance");
        let outside = tmp.path().join("chosen");
        std::fs::create_dir_all(&instance_dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let meta = crate::supervisor::module_loader::parse_module_toml("[module]\nschema_version = 1\nname = 'g'\nversion = '1'\nentry = 'lifecycle.py'\n\n[[settings.fields]]\nname = 'data'\ntype = 'file'\n\n[install]\nmethod = 'manual'\n\n[extensions.saba-core]\ndata_roots = ['{data}', '{home}/Worlds']\n").unwrap();
        let mut instance = ServerInstance::new("g", "g");
        instance.module_settings.insert("data".into(), json!(&outside));
        let target = outside.join("deleted/world");
        assert!(Boundary::for_instance(&instance, &instance_dir, Some(&meta))
            .resolve(&target)
            .is_none());
        instance.user_settings.insert("data".into());
        assert!(Boundary::for_instance(&instance, &instance_dir, Some(&meta))
            .resolve(&target)
            .is_some());
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap();
        assert!(Boundary::for_instance(&instance, &instance_dir, Some(&meta))
            .resolve(&home.join("Worlds/secret"))
            .is_none());
        instance.user_settings.clear();
        instance.imported_settings.insert("data".into());
        let boundary = Boundary::for_instance(&instance, &instance_dir, Some(&meta));
        assert!(boundary.resolve(&target).is_some());
        assert_eq!(
            boundary.resolve(Path::new("world")).unwrap(),
            instance_dir.join("server/world")
        );
        for daemon in [
            "instance.json",
            "run/x",
            "RUN/x",
            "settings.json.",
            "SETTIN~1.JSO",
            "run:x",
        ] {
            assert!(boundary.resolve(&instance_dir.join(daemon)).is_none(), "{daemon}");
        }
        assert!(boundary.resolve(&instance_dir).is_none());
        let encoded = serde_json::to_value(&instance).unwrap();
        let restored: ServerInstance = serde_json::from_value(encoded).unwrap();
        assert_eq!(restored.imported_settings, instance.imported_settings);
    }
}

pub(crate) fn outside(module: &str, function: &str, path: impl std::fmt::Display) -> anyhow::Error {
    crate::plugin::ModuleCallError::from_response(serde_json::json!({
        "success": false, "error_code": "module_path_outside", "module": module,
        "function": function, "phase": "path", "stderr_tail": [],
        "error": format!("{} returned a path outside this instance's allowed roots: {}", function, path),
    }))
    .into()
}
