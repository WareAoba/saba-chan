//! `completions` — 셸 자동 완성 스크립트를 찍거나(`completions <셸>`) 이 사용자의 셸이 스스로 읽는 자리에 둔다(`--install`)
//!
//! 두 이름(`saba-chan`, `saba-chan-cli`) 모두에 붙인다. PATH나 셸 설정 파일(rc·프로필)은 고치지 않는다:
//! 설정 파일을 고쳐야만 읽는 셸(PowerShell·elvish)은 파일을 두지 않고 넣을 한 줄만 알린다.

use crate::ctx::Ctx;
use crate::error::{failed, usage, Result};
use clap_complete::Shell;
use serde_json::json;
use std::path::{Path, PathBuf};

/// 완성을 붙이는 명령 이름: 짧은 이름(실행기·링크)과 CLI 파일 이름
const NAMES: [&str; 2] = ["saba-chan", "saba-chan-cli"];

pub fn run(ctx: &Ctx, shell: Option<Shell>, install: bool) -> Result<()> {
    if !install {
        let shell = shell.ok_or_else(|| usage(t!("completions.which_shell")))?;
        std::io::Write::write_all(&mut std::io::stdout(), &printed(shell)).map_err(|e| failed(e.to_string()))?;
        return Ok(());
    }
    let shell = shell.or_else(Shell::from_env).ok_or_else(|| usage(t!("completions.which_shell")))?;
    let places = Places::from_env().ok_or_else(|| failed(t!("completions.no_home")))?;
    match install_for(shell, &places).map_err(|e| failed(e.to_string()))? {
        Installed::Files(files) => {
            let shown = places.tilde(&files[0]);
            ctx.out.done(&t!("completions.installed", shell = shell, path = shown), &json!({ "shell": shell.to_string(), "files": files }));
            match shell {
                Shell::Zsh if !places.zfunc_in_fpath() => ctx.out.hint(&t!("completions.zsh_fpath", rc = places.tilde(&places.zshrc()))),
                // fish는 완성을 쓸 때 그 자리에서 읽는다
                Shell::Fish => {}
                _ => ctx.out.hint(&t!("completions.new_terminal")),
            }
        }
        Installed::ByHand { profile, line } => {
            let msg = t!("completions.by_hand", shell = shell, profile = profile, line = line);
            ctx.out.data(&json!({ "shell": shell.to_string(), "files": [], "profile": profile, "line": line }), |_| say!("{}", msg));
        }
    }
    Ok(())
}

/// 완성 파일을 둘 자리를 정하는 이 사용자의 폴더들
#[derive(Debug, Clone)]
struct Places {
    home: PathBuf,
    data: PathBuf,
    config: PathBuf,
    /// zsh 설정 폴더 (`ZDOTDIR`, 없으면 홈)
    zdot: PathBuf,
    /// zsh가 내보낸 함수 경로 (`FPATH`, 보통은 내보내지 않는다)
    fpath: Option<String>,
}

impl Places {
    fn from_env() -> Option<Self> {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let home = var("HOME").or_else(|| var("USERPROFILE"))?;
        Some(Self {
            data: var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")),
            config: var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
            zdot: var("ZDOTDIR").unwrap_or_else(|| home.clone()),
            fpath: std::env::var("FPATH").ok(),
            home,
        })
    }

    fn zfunc(&self) -> PathBuf {
        self.home.join(".zfunc")
    }

    fn zshrc(&self) -> PathBuf {
        self.zdot.join(".zshrc")
    }

    /// `~/.zfunc`가 zsh 함수 경로에 드는가: 내보낸 `FPATH`에 있거나 `.zshrc`가 그 폴더를 적었다 (읽기만 한다)
    fn zfunc_in_fpath(&self) -> bool {
        let dir = self.zfunc();
        self.fpath.as_deref().is_some_and(|f| std::env::split_paths(f).any(|p| p == dir))
            || std::fs::read_to_string(self.zshrc()).is_ok_and(|rc| rc.lines().any(|l| !l.trim_start().starts_with('#') && l.contains(".zfunc")))
    }

    /// 홈 아래 경로는 `~/…`로 보인다 (셸이 쓰는 `/`로)
    fn tilde(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.display().to_string().replace(std::path::MAIN_SEPARATOR, "/")),
            Err(_) => path.display().to_string(),
        }
    }
}

#[derive(Debug, PartialEq)]
enum Installed {
    /// 쓴 파일 (첫째가 짧은 이름 것)
    Files(Vec<PathBuf>),
    /// 설정 파일을 고쳐야만 읽는 셸: 넣을 줄
    ByHand { profile: &'static str, line: String },
}

/// 한 이름의 완성 스크립트
fn script(shell: Shell, name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    clap_complete::generate(shell, &mut crate::cli::localized(), name, &mut out);
    out
}

/// zsh는 `#compdef`에 적은 이름 모두에 한 파일을 쓴다 (둘을 이어 붙이면 파일로 저장해 쓰는 꼴이 깨진다)
fn zsh_script() -> Vec<u8> {
    let body = String::from_utf8_lossy(&script(Shell::Zsh, NAMES[0])).replacen(&format!("#compdef {}", NAMES[0]), &format!("#compdef {}", NAMES.join(" ")), 1);
    body.into_bytes()
}

/// `completions <셸>`이 찍는 두 이름의 스크립트. zsh 말고는 이름마다 이어 붙인다
fn printed(shell: Shell) -> Vec<u8> {
    match shell {
        Shell::Zsh => zsh_script(),
        _ => NAMES.iter().flat_map(|n| script(shell, n)).collect(),
    }
}

/// 셸이 스스로 읽는 이 사용자의 자리에 두 이름의 완성 스크립트를 쓴다
fn install_for(shell: Shell, places: &Places) -> std::io::Result<Installed> {
    let files: Vec<(PathBuf, Vec<u8>)> = match shell {
        // bash-completion은 명령 이름의 파일을 그 명령을 처음 완성할 때 읽는다
        Shell::Bash => NAMES.iter().map(|n| (places.data.join("bash-completion/completions").join(n), script(shell, n))).collect(),
        Shell::Fish => NAMES.iter().map(|n| (places.config.join("fish/completions").join(format!("{n}.fish")), script(shell, n))).collect(),
        Shell::Zsh => vec![(places.zfunc().join(format!("_{}", NAMES[0])), zsh_script())],
        Shell::PowerShell => {
            return Ok(Installed::ByHand { profile: "$PROFILE", line: format!("{} completions powershell | Out-String | Invoke-Expression", NAMES[0]) });
        }
        _ => return Ok(Installed::ByHand { profile: "~/.config/elvish/rc.elv", line: format!("eval ({} completions elvish | slurp)", NAMES[0]) }),
    };
    for (path, body) in &files {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, body)?;
    }
    Ok(Installed::Files(files.into_iter().map(|(p, _)| p).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(root: &Path) -> Places {
        Places { home: root.into(), data: root.join(".local/share"), config: root.join(".config"), zdot: root.into(), fpath: None }
    }

    /// 두 이름 모두에 완성이 붙는다: bash·fish는 이름마다 파일, zsh는 두 이름을 적은 파일 하나
    #[test]
    fn both_command_names_get_completions_where_the_shell_looks() {
        crate::i18n::init("en");
        let tmp = tempfile::tempdir().unwrap();
        let p = places(tmp.path());

        let Installed::Files(bash) = install_for(Shell::Bash, &p).unwrap() else { panic!("bash writes files") };
        assert_eq!(bash[0], tmp.path().join(".local/share/bash-completion/completions/saba-chan"));
        for (file, name) in bash.iter().zip(NAMES) {
            assert!(std::fs::read_to_string(file).unwrap().contains(&format!(" {name}\n")), "{name}");
        }

        let Installed::Files(zsh) = install_for(Shell::Zsh, &p).unwrap() else { panic!("zsh writes a file") };
        assert_eq!(zsh, vec![tmp.path().join(".zfunc/_saba-chan")]);
        assert!(std::fs::read_to_string(&zsh[0]).unwrap().starts_with("#compdef saba-chan saba-chan-cli\n"));

        let Installed::Files(fish) = install_for(Shell::Fish, &p).unwrap() else { panic!("fish writes files") };
        assert_eq!(fish, vec![tmp.path().join(".config/fish/completions/saba-chan.fish"), tmp.path().join(".config/fish/completions/saba-chan-cli.fish")]);
        assert!(std::fs::read_to_string(&fish[1]).unwrap().contains("complete -c saba-chan-cli"));
        assert_eq!(p.tilde(&zsh[0]), "~/.zfunc/_saba-chan");
    }

    /// 찍는 꼴(`completions <셸>`, 파일로 저장하거나 rc에서 eval)도 두 이름에 붙는다. zsh는 설치하는 파일과 같은 본문이다
    #[test]
    fn the_printed_script_covers_both_command_names() {
        crate::i18n::init("en");
        for shell in [Shell::Bash, Shell::Fish] {
            let out = String::from_utf8(printed(shell)).unwrap();
            for name in NAMES {
                let registered = if shell == Shell::Fish { out.contains(&format!("complete -c {name} ")) } else { out.contains(&format!(" {name}\n")) };
                assert!(registered, "{shell} {name}");
            }
        }
        let tmp = tempfile::tempdir().unwrap();
        let Installed::Files(zsh) = install_for(Shell::Zsh, &places(tmp.path())).unwrap() else { panic!("zsh writes a file") };
        assert_eq!(printed(Shell::Zsh), std::fs::read(&zsh[0]).unwrap());
    }

    /// 설정 파일을 고쳐야만 읽는 셸은 아무 파일도 쓰지 않고 넣을 한 줄을 준다
    #[test]
    fn a_shell_that_needs_a_profile_edit_gets_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let got = install_for(Shell::PowerShell, &places(tmp.path())).unwrap();
        assert!(matches!(&got, Installed::ByHand { line, .. } if line.contains("completions powershell")), "{got:?}");
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0, "nothing is written");
    }

    /// zsh 함수 경로는 고치지 않는다: `.zshrc`가 `~/.zfunc`를 적었는지 읽기만 한다
    #[test]
    fn the_zsh_hint_follows_the_zshrc() {
        let tmp = tempfile::tempdir().unwrap();
        let p = places(tmp.path());
        assert!(!p.zfunc_in_fpath());
        std::fs::write(tmp.path().join(".zshrc"), "# fpath=(~/.zfunc $fpath)\n").unwrap();
        assert!(!p.zfunc_in_fpath(), "a comment does not count");
        std::fs::write(tmp.path().join(".zshrc"), "fpath=(~/.zfunc $fpath)\nautoload -U compinit && compinit\n").unwrap();
        assert!(p.zfunc_in_fpath());
        let other = places(&tmp.path().join("other"));
        let listed = std::env::join_paths([PathBuf::from("/usr/share/zsh/functions"), other.zfunc()]).unwrap();
        let exported = Places { fpath: Some(listed.to_string_lossy().into_owned()), ..other };
        assert!(exported.zfunc_in_fpath());
    }
}
