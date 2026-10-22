//! `uninstall` — 이 PC에서 사바쨩을 지운다 (명령창 제거)
//!
//! 창 제거기와 같은 제거 라이브러리(`saba-chan-installer-core`)를 부른다. 데몬과 서버를 끄고 프로그램 폴더·런타임·
//! 임시 파일·바로가기·레지스트리를 지운다. 설정·인스턴스는 `--remove-*`가 없으면 남긴다

use crate::cli::UninstallArgs;
use crate::ctx::Ctx;
use crate::error::{failed, Result};
use saba_chan_installer_core::uninstall::{self, Keep};
use serde_json::json;

fn keep_of(args: &UninstallArgs) -> Keep {
    Keep { settings: !args.remove_settings, instances: !args.remove_instances }
}

pub async fn run(ctx: &Ctx, args: UninstallArgs) -> Result<()> {
    let Some(dir) = uninstall::target_dir() else {
        return Err(failed(t!("uninstall.not_installed")));
    };
    let keep = keep_of(&args);
    if !ctx.ask.confirm(&t!("uninstall.confirm", dir = dir.display(), kept = kept_text(keep)), false)? {
        return Err(crate::error::CliError::Cancelled.into());
    }
    let left = uninstall::do_uninstall(keep, |p| {
        if !p.complete {
            ctx.out.note(&p.message);
        }
    })
    .await;
    if !left.is_empty() {
        let paths: Vec<String> = left.iter().map(|p| p.display().to_string()).collect();
        return Err(failed(t!("uninstall.left", paths = paths.join("\n"))));
    }
    ctx.out.done(
        &t!("uninstall.done", kept = kept_text(keep)),
        &json!({ "ok": true, "dir": dir, "kept_settings": keep.settings, "kept_instances": keep.instances }),
    );
    Ok(())
}

/// 남기는 것을 한 줄로
fn kept_text(keep: Keep) -> String {
    match (keep.settings, keep.instances) {
        (true, true) => t!("uninstall.kept_both"),
        (true, false) => t!("uninstall.kept_settings"),
        (false, true) => t!("uninstall.kept_instances"),
        (false, false) => t!("uninstall.kept_nothing"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn uninstall_keeps_both_unless_each_is_removed() {
        let parse = |a: &[&str]| match crate::cli::Cli::try_parse_from(a).unwrap().cmd {
            Some(crate::cli::Cmd::Uninstall(u)) => keep_of(&u),
            other => panic!("{other:?}"),
        };
        assert_eq!(parse(&["saba-chan-cli", "uninstall"]), Keep::ALL);
        assert_eq!(parse(&["saba-chan-cli", "uninstall", "--remove-settings"]), Keep { settings: false, instances: true });
        assert_eq!(parse(&["saba-chan-cli", "uninstall", "--remove-instances", "--remove-settings"]), Keep::NOTHING);
    }
}
