//! `notice`: 데몬이 보관하는 알림함 (`GET /api/notices`). 앱 알림 창과 같은 목록·같은 읽음 상태다.
//! 보기만 해서는 읽음으로 바꾸지 않는다 (`notice read`)

use crate::cli::NoticeCmd;
use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s};
use serde_json::Value;

pub async fn run(ctx: &Ctx, action: Option<NoticeCmd>) -> Result<()> {
    match action {
        None => list(ctx).await,
        Some(NoticeCmd::Read) => {
            let r = ctx.api.notices_read().await?;
            ctx.out.done(&t!("notice.read_done"), &r);
            Ok(())
        }
        Some(NoticeCmd::Remove { id }) => {
            let r = ctx.api.notice_remove(id).await?;
            ctx.out.done(&t!("notice.removed", id = id), &r);
            Ok(())
        }
        Some(NoticeCmd::Clear) => {
            let r = ctx.api.notices_clear().await?;
            let count = r.get("removed").and_then(Value::as_u64).unwrap_or(0);
            ctx.out.done(&t!("notice.cleared", count = count), &r);
            Ok(())
        }
    }
}

async fn list(ctx: &Ctx) -> Result<()> {
    let v = ctx.api.notices().await?;
    ctx.out.data(&v, |v| {
        let rows = rows(v);
        if rows.is_empty() {
            say!("{}", output::dim(&t!("notice.none")));
            return;
        }
        let mut table = output::table(&[t!("col.id"), t!("col.when"), t!("audit.from"), t!("notice.col")]);
        for r in rows {
            table.add_row(r.to_vec());
        }
        output::print_table(table);
        let unread = v.get("unread").and_then(Value::as_u64).unwrap_or(0);
        if unread > 0 {
            say!();
            ctx.out.note(&t!("notice.unread", count = unread));
            ctx.out.hint("saba-chan-cli notice read");
        }
    });
    Ok(())
}

/// 알림 → 표 한 줄: 번호(읽지 않은 것은 ●), 시각, 묶음(서버 이름 또는 사바쨩), 문장과 이유
fn rows(v: &Value) -> Vec<[String; 4]> {
    v.get("notices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|n| {
            let unread = n.get("read").and_then(Value::as_bool) != Some(true);
            let mark = if unread { console::style("●").red().to_string() } else { output::dim("○") };
            let id = n.get("id").and_then(Value::as_u64).unwrap_or(0);
            let at = n.get("at").and_then(Value::as_i64).unwrap_or(0);
            let mut text = s(n, "message").to_string();
            if !s(n, "detail").is_empty() {
                text.push_str(&format!("  {}", output::dim(s(n, "detail"))));
            }
            [format!("{mark} {id}"), local_time(at), s(n, "source").to_string(), text]
        })
        .collect()
}

/// 유닉스 초 → 이 PC 시간 "11-15 19:02"
fn local_time(secs: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(secs, 0).single().map(|t| t.format("%m-%d %H:%M").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 번호로 지울 수 있게 번호를 보이고, 이유는 문장 뒤에 붙인다
    #[test]
    fn each_notice_shows_its_number_where_and_why() {
        console::set_colors_enabled(false);
        let v = json!({ "notices": [
            { "id": 12, "at": 0, "level": "error", "message": "크래시로 종료됨", "detail": "종료 코드 1", "source": "survival", "read": false },
            { "id": 3, "at": 0, "level": "success", "message": "사바쨩 0.1.14로 업데이트됨", "source": "사바쨩", "read": true },
        ], "unread": 1 });
        let r = rows(&v);
        assert_eq!(r[0][0], "● 12");
        assert_eq!(r[0][2], "survival");
        assert_eq!(r[0][3], "크래시로 종료됨  종료 코드 1");
        assert_eq!(r[1][0], "○ 3");
        assert_eq!(r[1][3], "사바쨩 0.1.14로 업데이트됨");
    }
}
