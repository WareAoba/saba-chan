//! `server run`·`send`·`rcon` 등 게임 명령과 결과 보이기

use super::*;

pub async fn run_command(ctx: &Ctx, a: RunArgs) -> Result<()> {
    let sv = resolve::server(ctx, a.server.as_deref()).await?;
    let cat = catalog_for_server(ctx, &sv).await?;
    let name = match a.command {
        Some(c) => c,
        None => {
            if cat.commands.is_empty() || !ctx.ask.enabled {
                return print_commands(ctx, &sv.module, &cat);
            }
            let items: Vec<String> = cat
                .commands
                .iter()
                .map(|c| format!("{}  {}", c.label, output::dim(&c.usage())))
                .collect();
            match ctx.ask.select(&t!("run.pick", name = sv.name), &items, 0)? {
                Some(i) => cat.commands[i].name.clone(),
                None => return Ok(()),
            }
        }
    };
    exec(ctx, &sv, &cat, &name, &a.args).await
}

/// 명령 하나 실행 (콘솔 입력에서도 쓴다)
/// 콘솔에 친 한 줄. 정의된 명령이면 인자를 나눠 묶고, 아니면 친 그대로 보낸다
/// (따옴표·JSON이 든 게임 명령을 다시 이어 붙이면 모양이 바뀐다)
pub async fn exec_line(ctx: &Ctx, sv: &Server, cat: &Catalog, text: &str) -> Result<()> {
    let words = shlex::split(text).unwrap_or_else(|| text.split_whitespace().map(str::to_string).collect());
    let name = words.first().map(String::as_str).unwrap_or_default();
    if cat.find(name).is_none() && cat.accepts_raw() {
        let r = ctx.api.command(&sv.id, text, Value::Null).await?;
        print_result(ctx, &r);
        return Ok(());
    }
    let (name, rest) = words.split_first().map(|(n, r)| (n.clone(), r.to_vec())).unwrap_or_default();
    exec(ctx, sv, cat, &name, &rest).await
}

pub async fn exec(ctx: &Ctx, sv: &Server, cat: &Catalog, name: &str, words: &[String]) -> Result<()> {
    let Some(def) = cat.find(name) else {
        if !cat.accepts_raw() {
            let found = resolve::find(&cat.names(), name, |n| vec![n.clone()], |n| n.clone());
            let suggestions = match found {
                resolve::Found::None(s) => s,
                resolve::Found::One(n) => vec![n],
                resolve::Found::Many(v) => v,
            };
            return Err(CliError::NotFound {
                what: t!("noun.command"),
                query: name.into(),
                suggestions,
            }
            .into());
        }
        // 정의에 없는 명령은 한 줄 그대로 (데몬이 콘솔로 보낸다)
        let line = std::iter::once(name.to_string())
            .chain(words.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        let r = ctx.api.command(&sv.id, &line, Value::Null).await?;
        print_result(ctx, &r);
        return Ok(());
    };
    let (mut args, missing) = def.bind(words).map_err(usage)?;
    for input in missing {
        let v = ask_input(&ctx.ask, def, &input)?;
        args.insert(input.name.clone(), input.coerce(v).map_err(usage)?);
    }
    if def.risk == Risk::Danger {
        ctx.ask
            .ensure(&t!("run.confirm_danger", label = def.label, name = sv.name))?;
    }
    let r = ctx.api.command(&sv.id, &def.name, Value::Object(args)).await?;
    print_result(ctx, &r);
    Ok(())
}

/// 시작·재시작 답의 `port_moved: [{field, from, to}]` → 알릴 줄
pub(super) fn port_moved_lines(body: &Value) -> Vec<String> {
    body.get("port_moved")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| Some(t!("start.port_moved", from = m.get("from")?, to = m.get("to")?)))
        .collect()
}

/// 시작 답의 `module_update: {module, latest}` → 모듈 업데이트 안내 줄
pub(super) fn module_update_line(body: &Value) -> Option<String> {
    let update = body.get("module_update")?;
    Some(t!("start.module_update", module = update.get("module")?.as_str()?, version = update.get("latest")?.as_str()?))
}

/// 예약 범위를 피해 옮긴 포트는 시작이 실패해도 저장돼 있으므로, 답이 성공이든 오류든 알린다.
/// 켠 서버의 모듈에 새 버전이 있으면 그것도 알린다
pub(super) fn tell_moved_ports(ctx: &Ctx, res: &Result<Value>) {
    let body = match res {
        Ok(v) => Some(v),
        Err(e) => match e.downcast_ref::<CliError>() {
            Some(CliError::Api { body, .. }) => Some(body),
            _ => None,
        },
    };
    for line in body.map(port_moved_lines).unwrap_or_default() {
        ctx.out.note(&line);
    }
    if let Some(line) = body.and_then(module_update_line) {
        ctx.out.note(&line);
    }
}

/// 가져오기 "옮기기"를 데몬이 거절했으면(다른 드라이브, 켜진 서버 등) 데몬의 영어 문구 대신 현지화된 까닭과 할 일로 바꾼다
pub(super) fn move_refusal(e: anyhow::Error) -> anyhow::Error {
    match api_code(&e) {
        Some(code @ ("move_cross_volume" | "move_in_use" | "move_failed" | "source_missing" | "destination_exists")) => {
            let body = json!({});
            match crate::error::api_hint(code, &body) {
                Some(text) => CliError::Failed(text).into(),
                None => e,
            }
        }
        _ => e,
    }
}

/// API 오류의 `error_code`
pub(super) fn api_code(e: &anyhow::Error) -> Option<&str> {
    match e.downcast_ref::<CliError>() {
        Some(CliError::Api { body, .. }) => body.get("error_code").and_then(Value::as_str),
        _ => None,
    }
}

pub(super) fn ask_input(ask: &crate::prompt::Prompt, def: &Command, input: &Input) -> Result<Value> {
    let missing = || usage(t!("run.missing_arg", label = input.label, usage = def.usage()));
    if !input.options.is_empty() {
        return match ask.select(&input.label, &input.options, 0)? {
            Some(i) => Ok(Value::String(input.options[i].clone())),
            None => Err(missing()),
        };
    }
    let prompt = match &input.placeholder {
        Some(p) => format!("{} {}", input.label, output::dim(&format!("({})", p))),
        None => input.label.clone(),
    };
    // 비밀번호 칸은 화면에 보이지 않게 묻는다
    let answer = if input.kind == "password" {
        ask.secret(&prompt)?.filter(|v| !v.is_empty())
    } else {
        ask.text(&prompt, None, false)?
    };
    match answer {
        Some(v) => Ok(Value::String(v)),
        None => Err(missing()),
    }
}

/// `add --set` 저장 응답이 `synced: false`면 끝에 알릴 문구. 설정은 저장했으므로 "저장하지 못했다"가 아니다 (L63)
pub(super) fn add_settings_failure(name: &str, r: &Value) -> Option<String> {
    (r.get("synced").and_then(Value::as_bool) == Some(false))
        .then(|| t!("add.created_settings_unsynced", name = name, error = s(r, "sync_error")))
}

/// 설정은 저장했지만 게임 설정 파일에 쓰지 못했으면(`synced: false`) 그 이유
pub(super) fn sync_failure(r: &Value) -> Option<String> {
    (r.get("synced").and_then(Value::as_bool) == Some(false)).then(|| t!("set.sync_failed", error = s(r, "sync_error")))
}

/// 명령 응답을 사람이 읽게: 문자열 → 그대로, 객체 배열 → 표, 객체 → 이름: 값
pub fn print_result(ctx: &Ctx, r: &Value) {
    // 콘솔에 붙어 있으면 stdin 답은 콘솔 줄로 이미 보인다
    if ctx.attached && !ctx.out.json && s(r, "protocol") == "stdin" {
        return;
    }
    ctx.out.data(r, |r| {
        // 데몬이 정리한 결과(값으로 답한 조회): 앱·디스코드와 같은 내용
        if let Some(output) = r.get("output").filter(|o| o.is_object()) {
            print_output(output);
            return;
        }
        let data = r.get("data").filter(|d| !d.is_null());
        let text = data
            .and_then(|d| d.get("response").and_then(Value::as_str).or_else(|| d.as_str()))
            .or_else(|| r.get("response").and_then(Value::as_str))
            .or_else(|| r.get("message").and_then(Value::as_str))
            .or_else(|| r.get("output").and_then(Value::as_str));
        if let Some(t) = text.filter(|t| !t.trim().is_empty()) {
            say!("{}", t.trim_end());
            return;
        }
        // 빈 답(`{response: "", executed}`)은 보낸 것만 알린다
        let empty_reply = data.is_some_and(|d| {
            d.get("response")
                .is_some_and(|x| x.as_str().is_some_and(|s| s.trim().is_empty()))
        });
        match data.filter(|_| !empty_reply) {
            Some(d) => print_value(d),
            None => say!("{} {}", console::style("✓").green().bold(), t!("run.sent")),
        }
    });
}

/// 데몬 `output` (`{text}` | `{fields}` | `{table}`, 데몬 `commands::output`)
fn print_output(output: &Value) {
    if let Some(text) = output.get("text").and_then(Value::as_str) {
        say!("{}", text);
    } else if let Some(fields) = output.get("fields").and_then(Value::as_array) {
        field_lines(fields).iter().for_each(|line| say!("{}", line));
    } else if let Some(table) = output.get("table") {
        let rows = table["rows"].as_array().cloned().unwrap_or_default();
        if let Some(label) = table.get("label").and_then(Value::as_str) {
            say!("{} ({})", console::style(label).bold(), rows.len());
        }
        if rows.is_empty() {
            return;
        }
        let cols: Vec<String> = table["columns"].as_array().into_iter().flatten().map(|c| s(c, "label").to_string()).collect();
        let mut t = output::table(&cols);
        for row in rows {
            t.add_row(row.as_array().into_iter().flatten().map(|v| v.as_str().unwrap_or_default().to_string()).collect::<Vec<_>>());
        }
        output::print_table(t);
    }
}

/// 이름: 값 목록을 이름 폭(화면 칸)에 맞춘 줄로
pub(super) fn field_lines(fields: &[Value]) -> Vec<String> {
    let width = fields.iter().map(|f| console::measure_text_width(s(f, "label"))).max().unwrap_or(0);
    fields
        .iter()
        .map(|f| {
            let label = s(f, "label");
            let pad = width - console::measure_text_width(label);
            format!("{}{}  {}", label, " ".repeat(pad), s(f, "value"))
        })
        .collect()
}

/// 모양을 모르는 JSON을 읽기 좋게
pub fn print_value(v: &Value) {
    match v {
        Value::Array(items) if items.iter().all(Value::is_object) && !items.is_empty() => {
            let mut cols: Vec<String> = Vec::new();
            for it in items {
                for k in it.as_object().unwrap().keys() {
                    if !cols.contains(k) {
                        cols.push(k.clone());
                    }
                }
            }
            let mut t = output::table(&cols);
            for it in items {
                t.add_row(
                    cols.iter()
                        .map(|c| it.get(c).map(show).unwrap_or_default())
                        .collect::<Vec<_>>(),
                );
            }
            output::print_table(t);
        }
        Value::Object(map) => {
            // 객체 안에 목록 하나뿐이면 그 목록을 표로 (예: {"players": [...]})
            if map.len() == 1 {
                if let Some(inner @ Value::Array(_)) = map.values().next() {
                    return print_value(inner);
                }
            }
            let mut flat = Vec::new();
            crate::cmd::flatten(v, "", &mut flat);
            output::print_kv(&flat.into_iter().map(|(k, v)| (k, show(&v))).collect::<Vec<_>>());
        }
        Value::Array(items) => {
            for it in items {
                say!("{}", show(it));
            }
        }
        other => say!("{}", show(other)),
    }
}

pub(super) async fn commands(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let cat = catalog_for_server(ctx, &sv).await?;
    print_commands(ctx, &sv.module, &cat)
}

pub fn print_commands(ctx: &Ctx, module: &str, cat: &Catalog) -> Result<()> {
    let v: Vec<Value> = cat
        .commands
        .iter()
        .map(|c| {
            json!({
                "name": c.name, "label": c.label, "description": c.description, "risk": c.risk.key(),
                "method": c.method, "aliases": c.aliases, "usage": c.usage(), "extension": c.extension,
                "inputs": c.inputs.iter().map(|i| json!({
                    "name": i.name, "label": i.label, "type": i.kind, "required": i.required,
                    "default": i.default, "options": i.options, "min": i.min, "max": i.max,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let v = json!({ "module": module, "accepts_raw": cat.accepts_raw(), "commands": v });
    ctx.out.data(&v, |_| {
        if cat.commands.is_empty() {
            say!("{}", output::dim(&t!("run.no_commands")));
        } else {
            let mut t = output::table(&[t!("col.command"), t!("col.what"), t!("col.aliases")]);
            for c in &cat.commands {
                let mut usage = c.usage();
                if c.risk == Risk::Danger {
                    usage.push_str(&console::style(" ⚠").red().to_string());
                }
                let mut what = if c.description.is_empty() || c.description == c.label {
                    c.label.clone()
                } else {
                    format!("{} — {}", c.label, c.description)
                };
                if let Some(ext) = &c.extension {
                    what.push_str(&output::dim(&format!("  [{}]", ext)));
                }
                t.add_row(vec![usage, what, output::dim(&c.aliases.join(", "))]);
            }
            output::print_table(t);
        }
        if cat.accepts_raw() {
            say!();
            say!("{}", output::dim(&t!("run.raw_ok")));
        }
    });
    Ok(())
}

pub(super) async fn send(ctx: &Ctx, query: &str, line: &str) -> Result<()> {
    let sv = resolve::server(ctx, Some(query)).await?;
    let r = ctx.api.stdin(&sv.id, line).await?;
    print_result(ctx, &r);
    Ok(())
}

pub(super) async fn chat(ctx: &Ctx, query: &str, name: &str, text: &str) -> Result<()> {
    let sv = resolve::server(ctx, Some(query)).await?;
    let r = ctx.api.game_chat(&sv.id, name, text).await?;
    print_result(ctx, &r);
    Ok(())
}

pub(super) async fn rcon(ctx: &Ctx, query: &str, line: &str) -> Result<()> {
    let sv = resolve::server(ctx, Some(query)).await?;
    let r = ctx.api.rcon(&sv.id, line).await?;
    print_result(ctx, &r);
    Ok(())
}

pub(super) async fn players(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.players(&sv.id).await?;
    ctx.out.data(&r, |r| {
        let online = r.get("online").and_then(Value::as_u64).unwrap_or(0);
        let count = match r.get("max").and_then(Value::as_u64) {
            Some(max) => format!("{online}/{max}"),
            None => online.to_string(),
        };
        output::heading(&t!("players.title", server = sv.name, count = count));
        let names: Vec<&str> = r.get("players").and_then(Value::as_array).into_iter().flatten().map(|p| s(p, "name")).filter(|n| !n.is_empty()).collect();
        if names.is_empty() {
            say!("{}", output::dim(&t!("players.none")));
        }
        for n in names {
            say!("  {}", n);
        }
    });
    Ok(())
}

/// 플레이어 목록(화이트리스트·관리자·밴 등 모듈이 선언한 것)마다 지금 명단과, 이름을 넣고 빼는 명령
pub(super) async fn lists(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.lists(&sv.id).await?;
    ctx.out.data(&r, |r| {
        let lists: Vec<&Value> = r.get("lists").and_then(Value::as_array).into_iter().flatten().collect();
        if lists.is_empty() {
            say!("{}", output::dim(&t!("lists.unsupported")));
        }
        for list in lists {
            let entries: Vec<&Value> = list.get("entries").and_then(Value::as_array).into_iter().flatten().collect();
            output::heading(&t!("lists.title", list = s(list, "label"), count = entries.len()));
            if entries.is_empty() {
                say!("{}", output::dim(&t!("lists.none")));
            }
            for e in entries {
                let more: Vec<&str> = [s(e, "note"), s(e, "added")].into_iter().filter(|v| !v.is_empty()).collect();
                say!("  {}  {}", s(e, "name"), output::dim(&more.join(" · ")));
            }
            say!("{}", output::dim(&t!("lists.edit", add = format!("server run {} {}", sv.name, s(list, "add")), remove = format!("server run {} {}", sv.name, s(list, "remove")))));
        }
    });
    Ok(())
}

/// `주소:포트` (IPv6는 `[주소]:포트`). 호스트 이름이면 그대로 붙인다
fn host_port(host: &str, port: u16) -> String {
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => std::net::SocketAddr::new(ip, port).to_string(),
        Err(_) => format!("{host}:{port}"),
    }
}

/// 서버 주소: 이 PC의 공개 주소(정한 도메인·IP, 없으면 릴레이가 본 공인 IP) + 밖에 여는 포트마다 `주소:포트`
pub(super) async fn address(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.address(&sv.id).await?;
    ctx.out.data(&r, |r| {
        output::heading(&t!("address.title", server = sv.name));
        let ports: Vec<&Value> = r.get("ports").and_then(Value::as_array).into_iter().flatten().collect();
        let host = r.get("address").and_then(Value::as_str).filter(|a| !a.is_empty());
        match host {
            _ if ports.is_empty() => say!("{}", t!("address.no_ports")),
            None => say!("{}", t!("address.unknown")),
            Some(host) => {
                for p in ports {
                    let port = p.get("port").and_then(Value::as_u64).and_then(|v| u16::try_from(v).ok()).unwrap_or(0);
                    say!("  {}  {}", host_port(host, port), output::dim(s(p, "label")));
                }
            }
        }
        if r.get("running").and_then(Value::as_bool) == Some(false) {
            say!("{}", output::dim(&t!("address.stopped")));
        }
    });
    Ok(())
}

/// 포트포워딩 확인: 판정 한 줄, 공인 주소, 포트마다 상태, 닫혔으면 무엇을 볼지
pub(super) async fn port_check(ctx: &Ctx, query: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    let r = ctx.api.port_check(&sv.id).await?;
    ctx.out.data(&r, |r| {
        output::heading(&t!("port_check.title", server = sv.name));
        let verdict = s(r, "verdict");
        say!("{}", t!(&format!("port_check.verdict.{}", verdict)));
        if !s(r, "host").is_empty() {
            let resolved: Vec<&str> = r.get("resolved").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            say!("{}", output::dim(&t!("port_check.host", host = s(r, "host"), resolved = resolved.join(", "), address = s(r, "address"))));
        } else if !s(r, "address").is_empty() {
            say!("{}", output::dim(&t!("port_check.address", address = s(r, "address"))));
        }
        for p in r.get("ports").and_then(Value::as_array).into_iter().flatten() {
            let port = p.get("port").and_then(Value::as_u64).unwrap_or(0);
            let status = t!(&format!("port_check.status.{}", s(p, "status")));
            say!("  {}/{}  {}", port, s(p, "protocol").to_uppercase(), status);
        }
        if matches!(verdict, "firewall" | "router" | "closed" | "cgnat" | "unknown" | "host_mismatch" | "host_unresolved") {
            // 고칠 곳은 데몬이 OS에 맞춰 고른다
            say!("{}", output::dim(&t!(&format!("port_check.hint.{}", s(r, "hint")))));
        }
    });
    Ok(())
}

pub(super) async fn log(ctx: &Ctx, query: Option<&str>, source: Option<&str>, lines: Option<usize>, runs: bool, all: bool) -> Result<()> {
    let sv = resolve::server(ctx, query).await?;
    if all {
        let r = ctx.api.export_log(&sv.id, source).await?;
        ctx.out.data(&r, |r| {
            say!("{}", s(r, "content").trim_end());
            if r.get("truncated").and_then(Value::as_bool) == Some(true) {
                ctx.out.warn(&t!("log.truncated"));
            }
        });
        return Ok(());
    }
    let r = ctx.api.logs(&sv.id, source, lines, None).await?;
    ctx.out.data(&r, |r| {
        let sources = r.get("sources").and_then(Value::as_array).cloned().unwrap_or_default();
        if sources.is_empty() {
            say!("{}", output::dim(&t!("log.none")));
            return;
        }
        if runs {
            let mut t = output::table(&[t!("log.col.run"), t!("log.col.started")]);
            for src in &sources {
                t.add_row(vec![s(src, "id"), s(src, "label")]);
            }
            output::print_table(t);
            return;
        }
        for l in r.get("lines").and_then(Value::as_array).into_iter().flatten() {
            say!("{}", l.as_str().unwrap_or_default());
        }
    });
    Ok(())
}

pub(super) async fn query(ctx: &Ctx, server: &str, name: Option<&str>, args: &[String]) -> Result<()> {
    let sv = resolve::server(ctx, Some(server)).await?;
    let Some(name) = name else {
        let m = ctx.api.module_metadata(&sv.module).await?;
        let list = m.pointer("/metadata/queries").cloned().unwrap_or(Value::Array(Vec::new()));
        ctx.out.data(&list, |list| {
            let items = list.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                say!("{}", output::dim(&t!("query.none")));
                return;
            }
            let mut t = output::table(&[t!("query.col.name"), t!("query.col.about"), t!("query.col.inputs")]);
            for q in &items {
                let about = q.get("label").or_else(|| q.get("description")).and_then(Value::as_str).unwrap_or("");
                let inputs: Vec<&str> = q.get("inputs").and_then(Value::as_array).into_iter().flatten().map(|i| s(i, "name")).collect();
                t.add_row(vec![s(q, "name").to_string(), about.to_string(), inputs.join(", ")]);
            }
            output::print_table(t);
        });
        return Ok(());
    };
    let input: serde_json::Map<String, Value> = crate::cmd::split_pairs(args)?.into_iter().map(|(k, v)| (k, crate::cmd::guess(&v))).collect();
    let r = ctx.api.query(&sv.id, name, Value::Object(input)).await?;
    ctx.out.data(&r, |r| print_value(r.get("data").unwrap_or(&Value::Null)));
    Ok(())
}

pub(super) async fn rest(ctx: &Ctx, query: &str, method: &str, endpoint: &str, body: Option<&str>) -> Result<()> {
    let sv = resolve::server(ctx, Some(query)).await?;
    let body = body
        .map(|b| serde_json::from_str::<Value>(b).map_err(|e| usage(t!("err.bad_json", error = e))))
        .transpose()?;
    let r = ctx.api.rest(&sv.id, &method.to_uppercase(), endpoint, body).await?;
    print_result(ctx, &r);
    Ok(())
}
