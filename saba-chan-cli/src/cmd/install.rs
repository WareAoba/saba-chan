//! `install` — 창 인스톨러 없이 이 PC에 사바쨩을 설치한다 (SSH·Server Core·GUI 없는 리눅스)
//!
//! 데몬이 아직 없으므로 데몬 API 대신 창 인스톨러와 같은 설치 라이브러리(`saba-chan-installer-core`)를 부른다.
//! 받기·서명 확인·압축 풀기는 그 라이브러리가 하고, 여기서는 값을 모으고 진행을 보인다.

use crate::cli::InstallArgs;
use crate::ctx::Ctx;
use crate::error::{failed, usage, Result};
use saba_chan_installer_core::{self as core, release, InstallHooks, InstallOptions, InstallProgress, Profile};
use serde_json::json;
use std::path::PathBuf;

/// 화면이 없는 환경인지: 리눅스는 DISPLAY·WAYLAND_DISPLAY가 모두 없을 때, Windows는 SSH로 들어왔거나
/// 대화형 세션 이름(SESSIONNAME)이 없을 때(서비스). `--gui`·`--no-gui`가 감지보다 먼저다
fn detect_profile(gui: bool, no_gui: bool, windows: bool, env: impl Fn(&str) -> Option<String>) -> Profile {
    if gui {
        return Profile::Desktop;
    }
    if no_gui {
        return Profile::Headless;
    }
    let set = |k: &str| env(k).is_some_and(|v| !v.is_empty());
    let headless = if windows {
        set("SSH_CONNECTION") || !set("SESSIONNAME")
    } else {
        !set("DISPLAY") && !set("WAYLAND_DISPLAY")
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
    let profile = detect_profile(args.gui, args.no_gui, cfg!(windows), |k| std::env::var(k).ok());

    let spin = crate::progress::spinner(&t!("install.fetching"));
    let manifest = release::fetch_latest().await;
    spin.finish_and_clear();
    let manifest = manifest.map_err(|e| failed(t!("install.fetch_failed", reason = e)))?;
    let available = release::modules(&manifest);

    // 설치 폴더: 고르지 않으면 기본 위치. 비어 있지 않은 폴더면 그 아래 Saba-chan 폴더에 푼다(제거기가 폴더째 지운다)
    let default_dir = core::dedicated_install_dir(&PathBuf::from(core::default_install_path()));
    let dir = match args.dir {
        Some(d) => core::dedicated_install_dir(&d),
        None => match ctx
            .ask
            .text(&t!("install.ask_dir"), Some(&default_dir.display().to_string()), false)?
        {
            Some(d) => core::dedicated_install_dir(&PathBuf::from(d.trim())),
            None => default_dir,
        },
    };

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
    };
    let last = core::install(options, &hooks).await;
    hooks.bar.finish_and_clear();

    if let Some(err) = last.error.as_deref().filter(|_| !last.complete) {
        if last.error_code.as_deref() == Some("elevation_failed") {
            return Err(failed(t!("install.not_writable", dir = dir.display())));
        }
        let log = last.log_path.clone().unwrap_or_default();
        return Err(failed(t!("install.failed", reason = err, log = log)));
    }
    for w in &last.warnings {
        ctx.out.warn(w);
    }
    ctx.out.done(
        &t!("install.done", dir = dir.display()),
        &json!({ "ok": true, "dir": dir, "modules": modules, "components": last.installed_components, "warnings": last.warnings }),
    );
    ctx.out.hint(&t!("install.next", cli = dir.join(cli_file()).display()));
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

fn cli_file() -> &'static str {
    if cfg!(windows) {
        "saba-chan-cli.exe"
    } else {
        "saba-chan-cli"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    /// 화면이 없는 서버는 GUI 없이 깐다. `--gui`·`--no-gui`가 감지보다 먼저다 (무인 설치 설계 2단계)
    #[test]
    fn a_machine_without_a_display_installs_without_the_gui() {
        assert_eq!(detect_profile(false, false, false, env(&[])), Profile::Headless);
        assert_eq!(
            detect_profile(false, false, false, env(&[("DISPLAY", ":0")])),
            Profile::Desktop
        );
        assert_eq!(
            detect_profile(false, false, false, env(&[("WAYLAND_DISPLAY", "wayland-0")])),
            Profile::Desktop
        );
        assert_eq!(
            detect_profile(false, false, true, env(&[("SESSIONNAME", "Console")])),
            Profile::Desktop
        );
        assert_eq!(
            detect_profile(
                false,
                false,
                true,
                env(&[("SESSIONNAME", "Console"), ("SSH_CONNECTION", "1 2 3 4")])
            ),
            Profile::Headless
        );
        assert_eq!(detect_profile(false, false, true, env(&[])), Profile::Headless);
        assert_eq!(detect_profile(true, false, false, env(&[])), Profile::Desktop);
        assert_eq!(
            detect_profile(false, true, false, env(&[("DISPLAY", ":0")])),
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
}
