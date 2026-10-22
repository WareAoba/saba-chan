//! `install` — 창 인스톨러 없이 이 PC에 사바쨩을 설치한다 (SSH·Server Core. 리눅스는 그 플랫폼 릴리스가 게시된 뒤부터)
//!
//! 데몬이 아직 없으므로 데몬 API 대신 창 인스톨러와 같은 설치 라이브러리(`saba-chan-installer-core`)를 부른다.
//! 받기·서명 확인·압축 풀기는 그 라이브러리가 하고, 여기서는 값을 모으고 진행을 보인다.

use crate::cli::InstallArgs;
use crate::ctx::Ctx;
use crate::error::{failed, usage, Result};
use saba_chan_installer_core::{self as core, release, InstallHooks, InstallOptions, InstallProgress, Profile};
use serde_json::json;
use std::path::{Path, PathBuf};

/// 설치 판정에 쓰는 운영체제. Server Core는 GUI 셸이 없는 Windows다
#[derive(Clone, Copy, PartialEq, Debug)]
enum Os {
    Unix,
    Windows,
    ServerCore,
}

impl Os {
    fn current() -> Os {
        if !cfg!(windows) {
            Os::Unix
        } else if is_server_core() {
            Os::ServerCore
        } else {
            Os::Windows
        }
    }
}

/// 레지스트리의 설치 유형이 "Server Core"인지 (읽지 못하면 아니다)
#[cfg(windows)]
fn is_server_core() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("InstallationType"))
        .is_ok_and(|t| t.eq_ignore_ascii_case("Server Core"))
}

#[cfg(not(windows))]
fn is_server_core() -> bool {
    false
}

/// 화면이 없는 환경인지: 리눅스는 DISPLAY·WAYLAND_DISPLAY가 모두 없을 때, Windows는 Server Core이거나 SSH로
/// 들어왔거나 대화형 세션 이름(SESSIONNAME)이 없을 때(서비스). `--gui`·`--no-gui`가 감지보다 먼저다
fn detect_profile(gui: bool, no_gui: bool, os: Os, env: impl Fn(&str) -> Option<String>) -> Profile {
    if gui {
        return Profile::Desktop;
    }
    if no_gui {
        return Profile::Headless;
    }
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    let headless = match os {
        Os::ServerCore => true,
        Os::Windows => set("SSH_CONNECTION") || !set("SESSIONNAME"),
        Os::Unix => !set("DISPLAY") && !set("WAYLAND_DISPLAY"),
    };
    if headless {
        Profile::Headless
    } else {
        Profile::Desktop
    }
}

/// `--modules a,b`를 릴리스에 있는 모듈 id로 확인한다. 모르는 이름은 고를 수 있는 목록과 함께 거절한다
fn pick_modules(asked: &[String], available: &[core::ModuleInfo]) -> std::result::Result<Vec<String>, String> {
    let ids: Vec<&str> = available.iter().map(|m| m.id.as_str()).collect();
    let mut picked: Vec<String> = Vec::new();
    for name in asked
        .iter()
        .flat_map(|a| a.split(','))
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        let Some(id) = ids.iter().find(|id| id.eq_ignore_ascii_case(name)) else {
            return Err(name.to_string());
        };
        if !picked.iter().any(|p| p == id) {
            picked.push(id.to_string());
        }
    }
    Ok(picked)
}

/// 라이브러리의 진행을 터미널 막대로 (`--json-stream`이면 한 줄씩)
struct Hooks {
    bar: crate::progress::Progress,
}

impl InstallHooks for Hooks {
    fn progress(&self, p: &InstallProgress) {
        if p.error.is_some() {
            return;
        }
        self.bar.set_position(p.percent.clamp(0, 100) as u64);
        let msg = match (&p.download_file, p.download_received, p.download_total) {
            (Some(file), Some(got), Some(total)) if total > 0 => format!("{} ({}%)", file, got * 100 / total),
            _ => p.message.clone(),
        };
        self.bar.set_message(msg);
    }
}

pub async fn run(ctx: &Ctx, args: InstallArgs) -> Result<()> {
    let profile = detect_profile(args.gui, args.no_gui, Os::current(), |k| std::env::var(k).ok());

    let spin = crate::progress::spinner(&t!("install.fetching"));
    let manifest = release::fetch_latest().await;
    spin.finish_and_clear();
    let manifest = manifest.map_err(|e| failed(t!("install.fetch_failed", reason = e)))?;
    let available = release::modules(&manifest);

    // 설치 폴더: 고르지 않으면 기본 위치. 비어 있지 않은 폴더면 그 아래 Saba-chan 폴더에 푼다(제거기가 폴더째 지운다)
    let default_dir = core::dedicated_install_dir(&PathBuf::from(core::default_install_path()));
    let chosen = match args.dir {
        Some(d) => Some(d),
        None => ctx
            .ask
            .text(&t!("install.ask_dir"), Some(&default_dir.display().to_string()), false)?
            .map(|d| PathBuf::from(d.trim())),
    };
    let dir = match chosen {
        Some(d) => install_target(&d).ok_or_else(|| usage(t!("install.empty_dir")))?,
        None => default_dir,
    };
    if core::check_install_dir(&dir, &saba_chan_updater_lib::component_dirs::Layout::current()).is_err() {
        return Err(usage(t!("install.invalid_dir", dir = dir.display())));
    }

    // 한 PC에는 설치본 하나만 둔다: 다른 폴더에 있으면 무엇도 받기 전에 그 설치본을 안내한다
    if let Some(other) = core::other_install(&dir) {
        return Err(usage(t!("install.installed_elsewhere", dir = other.display())));
    }
    // 설치는 그 폴더의 CLI를 모두 끄고 덮어쓴다: 거기서 도는 자신도 꺼지므로 다른 곳의 CLI로 하게 한다
    if std::env::current_exe().is_ok_and(|exe| runs_inside(&dir, &exe)) {
        return Err(usage(t!("install.running_inside", dir = dir.display())));
    }

    let modules = if !args.modules.is_empty() {
        pick_modules(&args.modules, &available).map_err(|name| {
            let ids: Vec<&str> = available.iter().map(|m| m.id.as_str()).collect();
            usage(t!("install.unknown_module", name = name, list = ids.join(", ")))
        })?
    } else {
        ask_modules(ctx, &available)?
    };

    let summary = t!(
        "install.confirm",
        dir = dir.display(),
        profile = if profile == Profile::Desktop {
            t!("install.profile_desktop")
        } else {
            t!("install.profile_headless")
        },
        modules = if modules.is_empty() {
            t!("install.no_modules")
        } else {
            modules.join(", ")
        }
    );
    if let Some(dir) = &args.components_dir {
        let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.clone());
        if let Err(e) = core::check_components_dir(&dir, &saba_chan_updater_lib::component_dirs::Layout::current()) {
            let hint = crate::i18n::try_t(&format!("err.hint.paths_{}", e.code)).unwrap_or(e.detail);
            return Err(usage(t!(
                "install.components_dir_invalid",
                dir = dir.display(),
                reason = hint
            )));
        }
        ctx.out.hint(&t!("install.components_dir", dir = dir.display()));
    }
    ctx.ask.ensure(&summary)?;

    let hooks = Hooks {
        bar: crate::progress::percent_bar(&t!("install.installing")),
    };
    let options = InstallOptions {
        install_path: dir.clone(),
        language: crate::i18n::lang().to_string(),
        create_desktop_shortcut: false,
        // 데스크톱이면 시작 메뉴에서 앱을 찾을 수 있게 한다 (헤드리스·리눅스는 라이브러리가 건너뛴다)
        create_start_menu_shortcut: profile == Profile::Desktop,
        profile,
        selected_modules: modules.clone(),
        module_metadata: available,
        release: Some(manifest),
        // 창 제거기를 두지 않는다: Windows 앱 목록에도 나오지 않고, 지울 때는 데몬을 끄고 설치 폴더를 지운다
        uninstaller_exe: None,
        components_dir: args
            .components_dir
            .as_ref()
            .map(|d| std::path::absolute(d).unwrap_or_else(|_| d.clone())),
    };
    // 한 번만 도는 명령이라 전역으로 둔다: 설치 라이브러리는 띄운 작업 안에서도 기록한다
    let logged =
        install_log(&core::log_file_path()).is_some_and(|log| tracing::subscriber::set_global_default(log).is_ok());
    let last = core::install(options, &hooks).await;
    hooks.bar.finish_and_clear();

    if let Some(err) = last.error.as_deref().filter(|_| !last.complete) {
        if last.error_code.as_deref() == Some("elevation_failed") {
            return Err(failed(t!("install.not_writable", dir = dir.display())));
        }
        if last.error_code.as_deref() == Some("daemon_running") {
            return Err(failed(t!("install.daemon_running")));
        }
        return Err(failed(match last.log_path.as_deref().filter(|_| logged) {
            Some(log) => t!("install.failed", reason = err, log = log),
            None => t!("install.failed_no_log", reason = err),
        }));
    }
    for w in &last.warnings {
        ctx.out.warn(w);
    }
    ctx.out.done(
        &t!("install.done", dir = dir.display()),
        &json!({ "ok": true, "dir": dir, "modules": modules, "components": last.installed_components, "warnings": last.warnings }),
    );
    ctx.out
        .hint(&t!("install.next", command = next_command(&dir, cfg!(windows))));
    Ok(())
}

/// 모듈은 하나씩 예/아니오로 묻는다. 물을 수 없으면 모듈 없이 설치한다(나중에 `module install`)
fn ask_modules(ctx: &Ctx, available: &[core::ModuleInfo]) -> Result<Vec<String>> {
    if !ctx.ask.enabled || ctx.ask.yes || available.is_empty() {
        return Ok(Vec::new());
    }
    let mut picked = Vec::new();
    for m in available {
        if ctx.ask.confirm(&t!("install.ask_module", name = m.name), false)? {
            picked.push(m.id.clone());
        }
    }
    Ok(picked)
}

/// 고른 설치 폴더를 절대 경로로 (빈 경로는 `None`: 지금 폴더에 풀지 않는다). 비어 있지 않으면 그 아래 Saba-chan 폴더
fn install_target(chosen: &Path) -> Option<PathBuf> {
    if chosen.as_os_str().to_string_lossy().trim().is_empty() {
        return None;
    }
    std::path::absolute(chosen)
        .ok()
        .map(|d| core::dedicated_install_dir(&d))
}

/// 설치 라이브러리의 기록을 창 인스톨러와 같은 파일에 덧붙인다. 열지 못하면 `None`(실패 때 경로를 안내하지 않는다)
fn install_log(path: &Path) -> Option<impl tracing::Subscriber + Send + Sync> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
    writeln!(file, "saba-chan-cli install {} started", env!("CARGO_PKG_VERSION")).ok()?;
    let log = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .with_target(false)
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .finish();
    Some(log)
}

/// 이 실행 파일이 설치 폴더 안에 있는지. 두 경로 모두 실제 경로로 비교한다 (폴더가 아직 없으면 밖이다)
fn runs_inside(dir: &Path, exe: &Path) -> bool {
    match (std::fs::canonicalize(dir), std::fs::canonicalize(exe)) {
        (Ok(dir), Ok(exe)) => exe.starts_with(dir),
        _ => false,
    }
}

/// 설치 뒤 데몬을 켜는 명령. 공백 든 경로도 그대로 붙여 실행할 수 있게: Windows는 cmd·PowerShell 모두 되는
/// `pushd "폴더"`(cmd에서도 드라이브까지 옮긴다) + `.\saba-chan-cli`, 그 밖은 작은따옴표로 싼 경로
fn next_command(dir: &Path, windows: bool) -> String {
    if windows {
        format!("pushd \"{}\"\n.\\saba-chan-cli daemon start", dir.display())
    } else {
        // 유닉스 셸 꼴이라 구분자와 파일 이름도 유닉스 것으로 (`dir.join`은 Windows에서 `\`를 넣는다)
        let cli = format!("{}/saba-chan-cli", dir.display().to_string().trim_end_matches('/'));
        format!("'{}' daemon start", cli.replace('\'', r"'\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli_file() -> &'static str {
        if cfg!(windows) {
            "saba-chan-cli.exe"
        } else {
            "saba-chan-cli"
        }
    }

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    /// 화면이 없는 서버는 GUI 없이 깐다. `--gui`·`--no-gui`가 감지보다 먼저다 (무인 설치 설계 2단계)
    #[test]
    fn a_machine_without_a_display_installs_without_the_gui() {
        assert_eq!(detect_profile(false, false, Os::Unix, env(&[])), Profile::Headless);
        assert_eq!(
            detect_profile(false, false, Os::Unix, env(&[("DISPLAY", ":0")])),
            Profile::Desktop
        );
        assert_eq!(
            detect_profile(false, false, Os::Unix, env(&[("WAYLAND_DISPLAY", "wayland-0")])),
            Profile::Desktop
        );
        assert_eq!(
            detect_profile(false, false, Os::Windows, env(&[("SESSIONNAME", "Console")])),
            Profile::Desktop
        );
        assert_eq!(
            detect_profile(
                false,
                false,
                Os::Windows,
                env(&[("SESSIONNAME", "Console"), ("SSH_CONNECTION", "1 2 3 4")])
            ),
            Profile::Headless
        );
        assert_eq!(detect_profile(false, false, Os::Windows, env(&[])), Profile::Headless);
        assert_eq!(detect_profile(true, false, Os::Unix, env(&[])), Profile::Desktop);
        assert_eq!(
            detect_profile(false, true, Os::Unix, env(&[("DISPLAY", ":0")])),
            Profile::Headless
        );
    }

    /// 모듈 이름은 릴리스에 있는 것만, 대소문자 무시, 겹치면 한 번
    #[test]
    fn only_modules_in_the_release_can_be_picked() {
        let m = |id: &str| core::ModuleInfo {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            icon: String::new(),
            required_extensions: Vec::new(),
        };
        let available = vec![m("minecraft"), m("palworld")];
        assert_eq!(
            pick_modules(&["Minecraft, palworld".into(), "minecraft".into()], &available).unwrap(),
            vec!["minecraft".to_string(), "palworld".to_string()]
        );
        assert_eq!(pick_modules(&["zomboid".into()], &available).unwrap_err(), "zomboid");
        assert!(pick_modules(&[" , ".into()], &available).unwrap().is_empty());
    }

    /// 설치 폴더 안에서 도는 CLI로 그 폴더에 다시 깔면, 설치 라이브러리가 그 폴더의 CLI를 모두 끄므로 자신도 꺼진다.
    /// 그래서 시작하기 전에 거절한다 (#516 리뷰)
    #[test]
    fn a_cli_running_from_the_target_folder_refuses_to_install_there() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("Saba-chan");
        std::fs::create_dir_all(target.join("bin")).unwrap();
        std::fs::create_dir_all(root.path().join("x")).unwrap();
        let inside = target.join(cli_file());
        let nested = target.join("bin").join(cli_file());
        std::fs::write(&inside, b"").unwrap();
        std::fs::write(&nested, b"").unwrap();
        assert!(runs_inside(&target, &inside));
        assert!(runs_inside(&target, &nested));
        // 표기가 달라도 같은 폴더면 안이다
        assert!(runs_inside(
            &root.path().join("x").join("..").join("Saba-chan"),
            &inside
        ));
        let outside = root.path().join(cli_file());
        std::fs::write(&outside, b"").unwrap();
        assert!(!runs_inside(&target, &outside));
        // 이름만 겹치는 옆 폴더는 밖이다
        assert!(!runs_inside(&root.path().join("Saba"), &inside));
        // 아직 없는 폴더에 새로 깔 때
        assert!(!runs_inside(&root.path().join("new"), &outside));
    }

    /// 실패 때 안내하는 기록 파일에는 이번 CLI 설치의 기록이 실제로 남는다. 열지 못하면 경로를 안내하지 않는다 (#516 리뷰)
    #[test]
    fn the_install_log_it_points_to_is_written_by_this_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saba-chan-installer.log");
        let log = install_log(&path).expect("log opens");
        tracing::subscriber::with_default(log, || tracing::info!("[Install] step from core"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("saba-chan-cli install"), "{text}");
        assert!(text.contains("step from core"), "{text}");
        assert!(install_log(&dir.path().join("missing").join("x.log")).is_none());
    }

    /// 빈 경로(`--dir "$INSTALL_DIR"`의 빈 변수)는 거절하고, 상대 경로는 절대 경로로 바꿔 쓴다 (#516 리뷰)
    #[test]
    fn an_empty_install_dir_is_refused_and_a_relative_one_is_made_absolute() {
        assert_eq!(install_target(Path::new("")), None);
        assert_eq!(install_target(Path::new("  ")), None);
        let rel = install_target(Path::new("saba-test-relative")).unwrap();
        assert!(rel.is_absolute(), "{}", rel.display());
        assert!(rel.starts_with(std::env::current_dir().unwrap()), "{}", rel.display());
    }

    /// Server Core는 로컬 콘솔(SESSIONNAME=Console)에서도 GUI 셸이 없다: 헤드리스로 깐다 (#516 리뷰)
    #[test]
    fn server_core_installs_without_the_gui_even_at_its_console() {
        assert_eq!(
            detect_profile(false, false, Os::ServerCore, env(&[("SESSIONNAME", "Console")])),
            Profile::Headless
        );
        assert_eq!(
            detect_profile(true, false, Os::ServerCore, env(&[])),
            Profile::Desktop,
            "--gui still wins"
        );
    }

    /// 다음 단계 안내는 기본 경로(`C:\Program Files\…`)처럼 공백이 있어도 복사해 실행할 수 있다 (#516 리뷰)
    #[test]
    fn the_next_step_can_be_pasted_even_with_spaces_in_the_path() {
        let win = next_command(Path::new(r"C:\Program Files\Saba-chan"), true);
        assert_eq!(
            win,
            "pushd \"C:\\Program Files\\Saba-chan\"\n.\\saba-chan-cli daemon start"
        );
        let unix = next_command(Path::new("/home/me/my apps/saba-chan"), false);
        assert_eq!(unix, "'/home/me/my apps/saba-chan/saba-chan-cli' daemon start");
    }
}
