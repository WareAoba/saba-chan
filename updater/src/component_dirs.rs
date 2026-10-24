//! 모듈·익스텐션 폴더 — 어디에 두는지 정하고, 고른 곳으로 옮긴다
//!
//! 경로는 이 순서로 정한다: 환경 변수(개발·테스트용) → `config.json`의 `paths` 칸 → 기본값(`<data_dir>/<이름>`).
//! 데몬·CLI·업데이터·설치기·제거기 모두 [`resolve`]를 거친다 (`constants::resolve_modules_dir` 등).
//!
//! 사용자는 폴더 `X`를 고르고, 실제 폴더는 `X/modules`·`X/extensions`다 ([`target_for`]).
//! 익스텐션은 Python에서 `import extensions.<id>`로 불러 폴더 이름이 `extensions`여야 하고,
//! 제거기가 그 폴더째 지워도 고른 폴더의 다른 파일이 다치지 않는다.

use std::path::{Component as PathComponent, Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

/// `config.json`에서 고른 폴더를 적는 칸
pub const PATHS_SECTION: &str = "paths";

/// 사바쨩이 만든 폴더라는 표시. 기본 위치 밖 폴더는 이 표시가 있을 때만 제거기가 지운다
pub const OWNED_MARKER: &str = ".saba-chan-dir";
/// 복사 중 폴더(`staging_dir`)에 처음 넣는 표시. 사본 검증에서는 빼고, 이름을 바꾸기 전에 지운다
const STAGING_MARKER: &str = ".saba-chan-moving";

/// 옮길 수 있는 폴더 종류
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Modules,
    Extensions,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Modules, Kind::Extensions];

    /// 폴더 이름이자 `paths` 칸의 키
    pub fn dir_name(self) -> &'static str {
        match self {
            Kind::Modules => crate::constants::MODULES_DIR_NAME,
            Kind::Extensions => crate::constants::EXTENSIONS_DIR_NAME,
        }
    }

    /// 개발·테스트용으로 경로를 고정하는 환경 변수
    pub fn env_var(self) -> &'static str {
        match self {
            Kind::Modules => "SABA_MODULES_PATH",
            Kind::Extensions => "SABA_EXTENSIONS_DIR",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.dir_name() == s)
    }

    fn other(self) -> Kind {
        match self {
            Kind::Modules => Kind::Extensions,
            Kind::Extensions => Kind::Modules,
        }
    }
}

/// 지금 경로가 어디서 왔는가
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Env,
    Config,
    Default,
}

/// 기본 폴더: `<data_dir>/<이름>`
pub fn default_dir(kind: Kind, data_dir: &Path) -> PathBuf {
    data_dir.join(kind.dir_name())
}

/// 지금 쓰는 폴더와 그 출처
pub fn resolve(kind: Kind) -> (PathBuf, Source) {
    let data_dir = crate::constants::resolve_data_dir();
    let env = std::env::var(kind.env_var()).ok();
    let paths = crate::config_file::ConfigFile::at(&data_dir).peek_section(PATHS_SECTION);
    resolve_from(kind, env.as_deref(), &paths, &data_dir)
}

/// [`resolve`]의 판단 부분. 설정 값이 쓸 수 없는 경로(상대 경로, `..`, 익스텐션인데 이름이 `extensions`가 아님)면 기본값으로 둔다
pub fn resolve_from(kind: Kind, env: Option<&str>, paths: &Value, data_dir: &Path) -> (PathBuf, Source) {
    if let Some(p) = env.filter(|p| !p.is_empty()) {
        return (PathBuf::from(p), Source::Env);
    }
    if let Some(p) = paths
        .get(kind.dir_name())
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
    {
        let path = PathBuf::from(p);
        if usable_config_path(kind, &path) {
            return (path, Source::Config);
        }
        tracing::warn!(
            "[Paths] Ignoring {} path {:?} in config.json; using the default",
            kind.dir_name(),
            p
        );
    }
    (default_dir(kind, data_dir), Source::Default)
}

fn usable_config_path(kind: Kind, path: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|c| matches!(c, PathComponent::ParentDir))
        && path.file_name().is_some_and(|n| n == kind.dir_name())
}

/// 고른 폴더를 쓸 수 없는 이유. `code`는 GUI·CLI가 문구를 고르는 키다
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathError {
    pub code: &'static str,
    pub detail: String,
}

impl PathError {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Self {
        PathError {
            code,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for PathError {}

/// [`target_for`]가 보는 지금 상태
#[derive(Debug, Clone)]
pub struct Layout {
    pub data_dir: PathBuf,
    pub home: Option<PathBuf>,
    pub modules: PathBuf,
    pub extensions: PathBuf,
}

impl Layout {
    pub fn current() -> Layout {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .filter(|h| !h.is_empty())
            .map(PathBuf::from);
        Layout {
            data_dir: crate::constants::resolve_data_dir(),
            home,
            modules: resolve(Kind::Modules).0,
            extensions: resolve(Kind::Extensions).0,
        }
    }

    pub fn dir(&self, kind: Kind) -> &Path {
        match kind {
            Kind::Modules => &self.modules,
            Kind::Extensions => &self.extensions,
        }
    }
}

/// 사용자가 고른 폴더 `chosen`에서 실제로 쓸 폴더를 정한다 (이미 `<이름>`으로 끝나면 그대로, 아니면 그 아래 `<이름>`).
/// 파일 시스템은 보지 않는다 (비었는지·쓸 수 있는지는 [`relocate`]가 본다)
pub fn target_for(kind: Kind, chosen: &Path, layout: &Layout) -> Result<PathBuf, PathError> {
    let shown = chosen.display().to_string();
    if chosen.as_os_str().is_empty() || !chosen.is_absolute() {
        return Err(PathError::new("not_absolute", shown));
    }
    if chosen.components().any(|c| matches!(c, PathComponent::ParentDir)) {
        return Err(PathError::new("not_absolute", shown));
    }
    let chosen: PathBuf = chosen
        .components()
        .filter(|c| !matches!(c, PathComponent::CurDir))
        .collect();
    let target = if chosen.file_name().is_some_and(|n| n == kind.dir_name()) {
        chosen
    } else {
        chosen.join(kind.dir_name())
    };
    let parent = target.parent().unwrap_or(&target);
    // 격리 실행이 권한을 줄 수 없는 위치: 드라이브 루트 바로 아래, 사용자 폴더 자신과 그 위 (익스텐션은 부모 폴더 목록 권한이 필요하다)
    let root_or_home = |p: &Path| normal_parts(p) == 0 || layout.home.as_deref().is_some_and(|h| is_within(h, p));
    if root_or_home(parent) {
        return Err(PathError::new("root_or_home", target.display().to_string()));
    }
    if is_within(&layout.data_dir, &target) {
        return Err(PathError::new("contains_data_dir", target.display().to_string()));
    }
    let current = layout.dir(kind);
    if crate::same_path(current, &target) {
        return Err(PathError::new("same_as_current", target.display().to_string()));
    }
    if crate::paths_overlap(current, &target) {
        return Err(PathError::new("overlaps_current", target.display().to_string()));
    }
    if crate::paths_overlap(layout.dir(kind.other()), &target) {
        return Err(PathError::new("overlaps_other", target.display().to_string()));
    }
    Ok(target)
}

/// `inner`가 `outer`와 같거나 그 안인가 (`same_path`와 같은 비교)
fn is_within(inner: &Path, outer: &Path) -> bool {
    crate::paths_overlap(inner, outer) && inner.components().count() >= outer.components().count()
}

fn normal_parts(p: &Path) -> usize {
    p.components().filter(|c| matches!(c, PathComponent::Normal(_))).count()
}

/// 고른 폴더를 `config.json`의 `paths.<종류>`에 적는다. 기본 위치면 칸을 지운다 (기본값이 바뀌면 따라가게)
pub fn save_choice(
    config: &crate::config_file::ConfigFile,
    kind: Kind,
    dir: &Path,
    data_dir: &Path,
) -> std::io::Result<()> {
    let is_default = crate::same_path(dir, &default_dir(kind, data_dir));
    config
        .update(|doc| {
            let paths = doc
                .entry(PATHS_SECTION.to_string())
                .or_insert_with(|| Value::Object(Default::default()));
            if !paths.is_object() {
                *paths = Value::Object(Default::default());
            }
            let paths = paths.as_object_mut().expect("just made an object");
            if is_default {
                paths.remove(kind.dir_name());
            } else {
                paths.insert(
                    kind.dir_name().to_string(),
                    Value::from(dir.to_string_lossy().into_owned()),
                );
            }
        })
        .map(|_| ())
}

/// 옮기기를 확정한다: 설치 기록(`installed-files.json`, 절대 경로)을 새 폴더로 바꾸고 `config.json`에 적는다.
/// 설정을 쓰지 못하면 기록도 되돌린다 ([`relocate`]의 `commit`으로 부른다)
pub fn commit_move(
    config: &crate::config_file::ConfigFile,
    kind: Kind,
    src: &Path,
    dst: &Path,
    data_dir: &Path,
) -> std::io::Result<()> {
    let records = data_dir.join("installed-files.json");
    crate::integrity::rebase_at(&records, src, dst).map_err(std::io::Error::other)?;
    save_choice(config, kind, dst, data_dir).map_err(|e| match crate::integrity::rebase_at(&records, dst, src) {
        Ok(()) => e,
        // 기록이 새 폴더를 가리킨 채 남는다: 숨기지 않고 함께 알린다
        Err(back) => std::io::Error::other(format!("{e}; the install record could not be put back: {back}")),
    })
}

/// 고른 폴더로 옮기고 설정에 적는다. 이미 그 폴더면 아무것도 하지 않는다 (`Ok(None)`).
/// 아무것도 그 폴더를 쓰지 않을 때만 부른다 (설치기는 데몬을 끈 뒤, 데몬은 [`relocate`]를 잠금 안에서 직접 부른다)
pub fn move_to(
    kind: Kind,
    chosen: &Path,
    layout: &Layout,
    config: &crate::config_file::ConfigFile,
) -> Result<Option<(PathBuf, Relocated)>, PathError> {
    let target = match target_for(kind, chosen, layout) {
        Err(e) if e.code == "same_as_current" => return Ok(None),
        other => other?,
    };
    let moved = relocate(layout.dir(kind), &target, || {
        commit_move(config, kind, layout.dir(kind), &target, &layout.data_dir)
    })?;
    Ok(Some((target, moved)))
}

/// 옮긴 결과
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Relocated {
    /// 새 폴더로 옮긴 뒤 원래 자리에서 지우지 못한 것 (내용은 이미 새 폴더에 있다)
    pub left_behind: Vec<PathBuf>,
}

/// `src`를 `dst`로 옮긴다. `dst`는 없거나 빈 폴더여야 한다.
///
/// 같은 드라이브면 이름 바꾸기 한 번, 아니면 `dst` 옆 임시 폴더에 복사하고 파일마다 크기·SHA-256을 맞춘 뒤 이름을 바꾼다.
/// 새 폴더가 다 갖춰진 뒤에야 `commit`(설정 저장)을 부른다. 그 전에 실패하거나 `commit`이 실패하면 `src`는 그대로이고
/// 만든 것은 지운다. `commit`이 끝난 뒤에만 `src`를 지운다
pub fn relocate(src: &Path, dst: &Path, commit: impl FnOnce() -> std::io::Result<()>) -> Result<Relocated, PathError> {
    relocate_inner(src, dst, commit, true)
}

fn relocate_inner(
    src: &Path,
    dst: &Path,
    commit: impl FnOnce() -> std::io::Result<()>,
    try_rename: bool,
) -> Result<Relocated, PathError> {
    let shown = dst.display().to_string();
    // 링크·정션을 푼 실제 경로로도 본다: 대상 위 폴더가 원본을 가리키면 원본 안에 복사해 끝없이 커진다
    if crate::paths_overlap(src, dst) || crate::paths_overlap(&real_path(src), &real_path(dst)) {
        return Err(PathError::new("overlaps_current", shown));
    }
    match std::fs::symlink_metadata(dst) {
        Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {
            let empty = std::fs::read_dir(dst)
                .map(|mut it| it.next().is_none())
                .unwrap_or(false);
            if !empty {
                return Err(PathError::new("not_empty", shown));
            }
            std::fs::remove_dir(dst).map_err(|e| PathError::new("not_writable", format!("{}: {}", shown, e)))?;
        }
        Ok(_) => return Err(PathError::new("not_empty", shown)),
        Err(_) => {}
    }
    let parent = dst
        .parent()
        .ok_or_else(|| PathError::new("not_absolute", shown.clone()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| PathError::new("not_writable", format!("{}: {}", parent.display(), e)))?;

    if !src.exists() {
        std::fs::create_dir(dst).map_err(|e| PathError::new("not_writable", format!("{}: {}", shown, e)))?;
        let _ = mark_owned(dst);
        if let Err(e) = commit() {
            let _ = std::fs::remove_dir_all(dst);
            return Err(PathError::new("commit_failed", e.to_string()));
        }
        return Ok(Relocated::default());
    }
    if let Some(link) = first_link(src) {
        return Err(PathError::new("has_link", link.display().to_string()));
    }

    // 같은 드라이브: 원자적 이름 바꾸기. 잠긴 파일이 있거나 드라이브가 다르면 실패하므로 복사로 간다
    if try_rename && std::fs::rename(src, dst).is_ok() {
        let _ = mark_owned(dst);
        if let Err(e) = commit() {
            if let Err(back) = std::fs::rename(dst, src) {
                tracing::error!("[Paths] Could not move {} back to {}: {}", shown, src.display(), back);
            }
            return Err(PathError::new("commit_failed", e.to_string()));
        }
        return Ok(Relocated::default());
    }

    let staging = staging_dir(dst);
    // 끊긴 옮기기가 남긴 복사 중 폴더만 지운다 (표시가 없는 같은 이름 폴더는 사용자 것일 수 있다)
    if std::fs::symlink_metadata(&staging).is_ok() {
        if !is_staging(&staging) {
            return Err(PathError::new("not_empty", staging.display().to_string()));
        }
        let _ = std::fs::remove_dir_all(&staging);
    }
    let copied = std::fs::create_dir_all(&staging)
        .and_then(|_| std::fs::write(staging.join(STAGING_MARKER), b""))
        .map_err(|e| PathError::new("not_writable", format!("{}: {}", staging.display(), e)))
        .and_then(|_| copy_tree(src, &staging))
        .and_then(|_| verify_copy(src, &staging))
        .and_then(|_| {
            std::fs::remove_file(staging.join(STAGING_MARKER))
                .map_err(|e| PathError::new("not_writable", format!("{}: {}", staging.display(), e)))
        });
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&staging, dst) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(PathError::new("not_writable", format!("{}: {}", shown, e)));
    }
    let _ = mark_owned(dst);
    if let Err(e) = commit() {
        let _ = std::fs::remove_dir_all(dst);
        return Err(PathError::new("commit_failed", e.to_string()));
    }
    let _ = std::fs::remove_dir_all(src);
    Ok(Relocated {
        left_behind: remaining(src),
    })
}

/// 복사 중인 폴더: `dst` 옆에 숨겨 둔다 (같은 드라이브라 마지막 이름 바꾸기가 원자적이다)
pub fn staging_dir(dst: &Path) -> PathBuf {
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    dst.with_file_name(format!(".{}.saba-moving", name))
}

/// 있는 가장 가까운 위 폴더까지 링크를 풀고 나머지를 붙인 경로 (없는 경로도 비교할 수 있게)
pub fn real_path(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut cur = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(cur) {
            return rest.iter().rev().fold(real, |p, part| p.join(part));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                cur = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// 복사 중 폴더 표시:사바쨩이 만든 복사 중 폴더인가 (끊긴 옮기기의 남은 것만 지우게)
pub fn is_staging(dir: &Path) -> bool {
    dir.join(STAGING_MARKER).is_file()
}

/// 사바쨩이 만든 폴더로 표시한다
pub fn mark_owned(dir: &Path) -> std::io::Result<()> {
    std::fs::write(dir.join(OWNED_MARKER), b"")
}

/// 사바쨩이 만든 폴더인가 (제거기가 폴더째 지워도 되는가)
pub fn is_owned(dir: &Path) -> bool {
    dir.join(OWNED_MARKER).is_file()
}

pub fn first_link(root: &Path) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() || is_junction(&meta) {
                return Some(path);
            }
            if meta.is_dir() {
                stack.push(path);
            }
        }
    }
    None
}

#[cfg(windows)]
fn is_junction(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_junction(_meta: &std::fs::Metadata) -> bool {
    false
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), PathError> {
    let fail = |p: &Path, e: std::io::Error| PathError::new("copy_failed", format!("{}: {}", p.display(), e));
    std::fs::create_dir_all(dst).map_err(|e| fail(dst, e))?;
    for entry in std::fs::read_dir(src).map_err(|e| fail(src, e))? {
        let entry = entry.map_err(|e| fail(src, e))?;
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        if entry.file_type().map_err(|e| fail(&from, e))?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| fail(&from, e))?;
        }
    }
    Ok(())
}

/// 원본의 모든 파일이 사본에 같은 크기·SHA-256으로 있고, 사본에 다른 파일이 없는가
fn verify_copy(src: &Path, copy: &Path) -> Result<(), PathError> {
    let a = files_under(src).map_err(|e| PathError::new("verify_failed", e.to_string()))?;
    let mut b = files_under(copy).map_err(|e| PathError::new("verify_failed", e.to_string()))?;
    b.retain(|rel| rel != Path::new(STAGING_MARKER));
    if a.len() != b.len() {
        return Err(PathError::new(
            "verify_failed",
            format!("{} files copied of {}", b.len(), a.len()),
        ));
    }
    for rel in &a {
        let (x, y) = (src.join(rel), copy.join(rel));
        match (digest(&x), digest(&y)) {
            (Ok(a), Ok(b)) if a == b => {}
            _ => return Err(PathError::new("verify_failed", rel.display().to_string())),
        }
    }
    Ok(())
}

fn files_under(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for entry in std::fs::read_dir(root.join(&rel))? {
            let entry = entry?;
            let path = rel.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn digest(path: &Path) -> std::io::Result<(u64, Vec<u8>)> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let len = std::io::copy(&mut file, &mut hasher)?;
    Ok((len, hasher.finalize().to_vec()))
}

fn remaining(src: &Path) -> Vec<PathBuf> {
    if !src.exists() {
        return Vec::new();
    }
    let mut left: Vec<PathBuf> = files_under(src)
        .unwrap_or_default()
        .into_iter()
        .map(|r| src.join(r))
        .collect();
    if left.is_empty() {
        left.push(src.to_path_buf());
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn abs(p: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!("C:\\{}", p.replace('/', "\\")))
        } else {
            PathBuf::from(format!("/{}", p))
        }
    }

    fn layout() -> Layout {
        Layout {
            data_dir: abs("home/me/.config/saba-chan"),
            home: Some(abs("home/me")),
            modules: abs("home/me/.config/saba-chan/modules"),
            extensions: abs("home/me/.config/saba-chan/extensions"),
        }
    }

    #[test]
    fn env_wins_then_config_then_default() {
        let data = abs("data");
        let paths = json!({"modules": abs("games/modules").to_str().unwrap()});
        assert_eq!(
            resolve_from(Kind::Modules, Some("/dev/mods"), &paths, &data),
            (PathBuf::from("/dev/mods"), Source::Env)
        );
        assert_eq!(
            resolve_from(Kind::Modules, None, &paths, &data),
            (abs("games/modules"), Source::Config)
        );
        assert_eq!(
            resolve_from(Kind::Extensions, None, &paths, &data),
            (data.join("extensions"), Source::Default)
        );
        assert_eq!(
            resolve_from(Kind::Modules, Some(""), &Value::Null, &data),
            (data.join("modules"), Source::Default)
        );
    }

    #[test]
    fn unusable_config_values_fall_back_to_the_default() {
        let data = abs("data");
        for bad in [
            json!("relative/modules"),
            json!(abs("games/other").to_str().unwrap()),
            json!(abs("a/../modules").to_str().unwrap()),
            json!(3),
            json!(""),
        ] {
            let paths = json!({ "modules": bad });
            assert_eq!(
                resolve_from(Kind::Modules, None, &paths, &data).1,
                Source::Default,
                "{:?}",
                paths
            );
        }
    }

    #[test]
    fn chosen_folder_gets_the_fixed_leaf_name() {
        let l = layout();
        assert_eq!(
            target_for(Kind::Modules, &abs("Games/Saba"), &l),
            Ok(abs("Games/Saba/modules"))
        );
        assert_eq!(
            target_for(Kind::Extensions, &abs("Games/Saba"), &l),
            Ok(abs("Games/Saba/extensions"))
        );
        // 이미 이름이 맞으면 한 번 더 붙이지 않는다
        assert_eq!(
            target_for(Kind::Modules, &abs("Games/modules"), &l),
            Ok(abs("Games/modules"))
        );
        // 기본 위치로 되돌리기
        let mut moved = layout();
        moved.modules = abs("Games/Saba/modules");
        assert_eq!(
            target_for(Kind::Modules, &l.data_dir, &moved),
            Ok(l.data_dir.join("modules"))
        );
    }

    #[test]
    fn unsafe_choices_are_refused_with_a_code() {
        let l = layout();
        let code = |kind, p: PathBuf| target_for(kind, &p, &l).unwrap_err().code;
        assert_eq!(code(Kind::Modules, PathBuf::from("relative")), "not_absolute");
        assert_eq!(code(Kind::Modules, abs("a/../b")), "not_absolute");
        // 드라이브 루트 바로 아래(`/modules`)와 사용자 폴더 바로 아래는 격리 실행 권한을 줄 수 없다
        assert_eq!(code(Kind::Modules, abs("")), "root_or_home");
        assert_eq!(code(Kind::Extensions, abs("home/me")), "root_or_home");
        assert_eq!(code(Kind::Modules, abs("home")), "root_or_home");
        assert_eq!(code(Kind::Modules, l.modules.clone()), "same_as_current");
        assert_eq!(code(Kind::Modules, l.modules.join("sub")), "overlaps_current");
        assert_eq!(code(Kind::Modules, l.extensions.clone()), "overlaps_other");
        assert_eq!(code(Kind::Modules, l.extensions.join("x")), "overlaps_other");
        // 사용자 폴더 안 깊은 곳은 된다
        assert_eq!(
            target_for(Kind::Modules, &abs("home/me/Games"), &l),
            Ok(abs("home/me/Games/modules"))
        );
    }

    #[test]
    fn target_containing_the_data_dir_is_refused() {
        let mut l = layout();
        l.home = None;
        l.data_dir = abs("srv/modules/data");
        assert_eq!(
            target_for(Kind::Modules, &abs("srv"), &l).unwrap_err().code,
            "contains_data_dir"
        );
    }

    fn tree(root: &Path) {
        std::fs::create_dir_all(root.join("minecraft/locales")).unwrap();
        std::fs::write(root.join("minecraft/module.toml"), "name = 'mc'").unwrap();
        std::fs::write(root.join("minecraft/locales/ko.json"), "{}").unwrap();
        std::fs::write(root.join("i18n.py"), "x = 1\n").unwrap();
    }

    #[test]
    fn saved_choice_is_read_back_and_the_default_clears_it() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let config = crate::config_file::ConfigFile::at(&data);
        config.set_section("gui", json!({"language": "ko"})).unwrap();
        let chosen = tmp.path().join("games").join("modules");
        save_choice(&config, Kind::Modules, &chosen, &data).unwrap();
        let paths = config.peek_section(PATHS_SECTION);
        assert_eq!(
            resolve_from(Kind::Modules, None, &paths, &data),
            (chosen, Source::Config)
        );
        save_choice(&config, Kind::Modules, &data.join("modules"), &data).unwrap();
        assert_eq!(
            resolve_from(Kind::Modules, None, &config.peek_section(PATHS_SECTION), &data).1,
            Source::Default
        );
        assert_eq!(
            config.section("gui"),
            json!({"language": "ko"}),
            "other settings are kept"
        );
    }

    #[test]
    fn move_to_moves_both_kinds_under_one_folder_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let config = crate::config_file::ConfigFile::at(&data);
        let mut l = Layout {
            data_dir: data.clone(),
            home: None,
            modules: data.join("modules"),
            extensions: data.join("extensions"),
        };
        tree(&l.modules);
        let chosen = tmp.path().join("Games").join("Saba");
        for kind in Kind::ALL {
            let (path, _) = move_to(kind, &chosen, &l, &config).unwrap().unwrap();
            match kind {
                Kind::Modules => l.modules = path,
                Kind::Extensions => l.extensions = path,
            }
        }
        assert!(chosen.join("modules").join("i18n.py").is_file());
        assert!(chosen.join("extensions").is_dir());
        let paths = config.peek_section(PATHS_SECTION);
        assert_eq!(
            resolve_from(Kind::Extensions, None, &paths, &data),
            (chosen.join("extensions"), Source::Config)
        );
        // 같은 폴더를 다시 고르면 그대로 둔다
        assert_eq!(move_to(Kind::Modules, &chosen, &l, &config), Ok(None));
    }

    #[test]
    fn rename_moves_everything_then_commits() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("old/modules"), tmp.path().join("new/modules"));
        tree(&src);
        let mut committed = false;
        let r = relocate(&src, &dst, || {
            committed = true;
            Ok(())
        })
        .unwrap();
        assert!(committed);
        assert!(r.left_behind.is_empty());
        assert!(!src.exists());
        assert_eq!(
            std::fs::read_to_string(dst.join("minecraft/locales/ko.json")).unwrap(),
            "{}"
        );
        assert!(is_owned(&dst));
    }

    #[test]
    fn failed_commit_puts_everything_back() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("old/modules"), tmp.path().join("new/modules"));
        tree(&src);
        let err = relocate(&src, &dst, || Err(std::io::Error::other("disk full"))).unwrap_err();
        assert_eq!(err.code, "commit_failed");
        assert!(src.join("minecraft/module.toml").is_file());
        assert!(!dst.exists());
    }

    #[test]
    fn copy_path_verifies_commits_and_removes_source() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("old/modules"), tmp.path().join("new/modules"));
        tree(&src);
        let staging = staging_dir(&dst);
        std::fs::create_dir_all(&staging).unwrap();
        copy_tree(&src, &staging).unwrap();
        assert_eq!(verify_copy(&src, &staging), Ok(()));
        std::fs::write(staging.join("i18n.py"), "x = 2\n").unwrap();
        assert_eq!(verify_copy(&src, &staging).unwrap_err().code, "verify_failed");
        std::fs::write(staging.join("i18n.py"), "x = 1\n").unwrap();
        std::fs::write(staging.join("extra.txt"), "").unwrap();
        assert_eq!(verify_copy(&src, &staging).unwrap_err().code, "verify_failed");
    }

    #[test]
    fn copy_move_across_drives_commits_then_removes_source() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("old/modules"), tmp.path().join("new/modules"));
        tree(&src);
        let r = relocate_inner(&src, &dst, || Ok(()), false).unwrap();
        assert!(r.left_behind.is_empty());
        assert!(!src.exists() && !staging_dir(&dst).exists());
        assert_eq!(
            std::fs::read_to_string(dst.join("minecraft/module.toml")).unwrap(),
            "name = 'mc'"
        );

        let (src2, dst2) = (tmp.path().join("old2/modules"), tmp.path().join("new2/modules"));
        tree(&src2);
        let err = relocate_inner(&src2, &dst2, || Err(std::io::Error::other("no")), false).unwrap_err();
        assert_eq!(err.code, "commit_failed");
        assert!(src2.join("minecraft/module.toml").is_file());
        assert!(!dst2.exists() && !staging_dir(&dst2).exists());
    }

    // Codex 리뷰 #544 P1: 대상 옆 복사 중 폴더 이름의 기존 폴더를 표시 없이도 통째로 지웠다
    #[test]
    fn a_staging_named_folder_is_removed_only_when_it_is_a_leftover_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("old/modules"), tmp.path().join("new/modules"));
        tree(&src);
        let staging = staging_dir(&dst);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("mine.txt"), "user").unwrap();
        let err = relocate_inner(&src, &dst, || panic!("must not commit"), false).unwrap_err();
        assert_eq!(err.code, "not_empty");
        assert_eq!(std::fs::read_to_string(staging.join("mine.txt")).unwrap(), "user");
        assert!(src.join("i18n.py").is_file());

        // 끊긴 옮기기가 남긴 것(표시 있음)은 지우고 다시 옮긴다
        std::fs::write(staging.join(STAGING_MARKER), "").unwrap();
        relocate_inner(&src, &dst, || Ok(()), false).unwrap();
        assert!(!staging.exists() && !dst.join("mine.txt").exists() && !dst.join(STAGING_MARKER).exists());
        assert!(dst.join("i18n.py").is_file() && !src.exists());
    }

    // Codex 리뷰 #544 P2: 대상 위 정션이 원본을 가리키면 이름 비교로는 겹치지 않아 원본 안에 복사했다
    #[cfg(windows)]
    #[test]
    fn a_target_reached_through_a_junction_into_the_source_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("old").join("modules");
        tree(&src);
        let alias = tmp.path().join("alias");
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&alias).arg(&src).output().unwrap();
        assert!(made.status.success(), "{}{}", String::from_utf8_lossy(&made.stdout), String::from_utf8_lossy(&made.stderr));
        let err = relocate_inner(&src, &alias.join("modules"), || panic!("must not commit"), false).unwrap_err();
        assert_eq!(err.code, "overlaps_current");
        assert!(!src.join("modules").exists() && !src.join(".modules.saba-moving").exists());
        std::fs::remove_dir(&alias).unwrap();
    }

    #[test]
    fn non_empty_target_is_refused_and_nothing_moves() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("old/modules"), tmp.path().join("new/modules"));
        tree(&src);
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("mine.txt"), "user file").unwrap();
        let err = relocate(&src, &dst, || panic!("must not commit")).unwrap_err();
        assert_eq!(err.code, "not_empty");
        assert!(src.join("i18n.py").is_file());
        assert_eq!(std::fs::read_to_string(dst.join("mine.txt")).unwrap(), "user file");
    }

    #[test]
    fn empty_target_folder_and_missing_source_are_fine() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("never/modules"), tmp.path().join("new/modules"));
        std::fs::create_dir_all(&dst).unwrap();
        relocate(&src, &dst, || Ok(())).unwrap();
        assert!(dst.is_dir() && is_owned(&dst));
    }

    #[cfg(unix)]
    #[test]
    fn links_inside_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("old/modules"), tmp.path().join("new/modules"));
        tree(&src);
        std::os::unix::fs::symlink(tmp.path(), src.join("minecraft/escape")).unwrap();
        assert_eq!(relocate(&src, &dst, || Ok(())).unwrap_err().code, "has_link");
        assert!(src.join("i18n.py").is_file());
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        fn segment() -> impl Strategy<Value = String> {
            prop_oneof![
                "[a-zA-Z0-9 _가-힣-]{1,8}",
                Just("modules".to_string()),
                Just("extensions".to_string()),
                Just("me".to_string()),
                Just("saba-chan".to_string()),
            ]
        }

        proptest! {
            /// 받아들인 대상은 늘 절대 경로이고, 이름이 종류의 폴더 이름이며, 지금 두 폴더·데이터 폴더·사용자 폴더와 겹치지 않는다
            #[test]
            fn accepted_targets_are_safe(parts in proptest::collection::vec(segment(), 0..6), ext in any::<bool>()) {
                let kind = if ext { Kind::Extensions } else { Kind::Modules };
                let l = layout();
                let chosen = parts.iter().fold(abs(""), |p, s| p.join(s));
                if let Ok(t) = target_for(kind, &chosen, &l) {
                    prop_assert!(t.is_absolute());
                    prop_assert_eq!(t.file_name().unwrap().to_str().unwrap(), kind.dir_name());
                    prop_assert!(!crate::paths_overlap(&t, &l.modules));
                    prop_assert!(!crate::paths_overlap(&t, &l.extensions));
                    prop_assert!(!crate::paths_overlap(&t, &l.data_dir) || !is_within(&l.data_dir, &t));
                    prop_assert!(normal_parts(t.parent().unwrap()) > 0);
                    prop_assert!(!crate::same_path(t.parent().unwrap(), l.home.as_deref().unwrap()));
                    // 설정에 적으면 그대로 다시 읽힌다
                    let paths = json!({ kind.dir_name(): t.to_str().unwrap() });
                    prop_assert_eq!(resolve_from(kind, None, &paths, &l.data_dir), (t.clone(), Source::Config));
                }
            }

            /// 옮기기는 성공하면 모든 파일이 같은 내용으로 새 폴더에 있고, commit이 실패하면 원래대로다
            #[test]
            fn relocate_keeps_every_file(files in proptest::collection::btree_map("[a-z]{1,6}(/[a-z]{1,6}){0,2}", proptest::collection::vec(any::<u8>(), 0..64), 1..8), fail in any::<bool>(), rename in any::<bool>()) {
                let tmp = tempfile::tempdir().unwrap();
                let (src, dst) = (tmp.path().join("a/modules"), tmp.path().join("b/modules"));
                let mut written = std::collections::BTreeMap::new();
                for (rel, bytes) in &files {
                    let p = src.join(rel);
                    // 같은 이름이 파일·폴더로 이미 쓰였으면 건너뛴다
                    if std::fs::create_dir_all(p.parent().unwrap()).is_ok() && std::fs::write(&p, bytes).is_ok() {
                        written.insert(rel.clone(), bytes.clone());
                    }
                }
                written.retain(|rel, _| src.join(rel).is_file());
                let r = relocate_inner(&src, &dst, || if fail { Err(std::io::Error::other("no")) } else { Ok(()) }, rename);
                let (root, gone) = if fail { (&src, &dst) } else { (&dst, &src) };
                prop_assert_eq!(r.is_ok(), !fail);
                prop_assert!(!gone.exists());
                for (rel, bytes) in &written {
                    prop_assert_eq!(&std::fs::read(root.join(rel)).unwrap(), bytes);
                }
            }
        }
    }
}
