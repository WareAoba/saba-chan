//! `server add`·`remove`: 새 서버 만들기(설치·가져오기·약관)와 지우기

use super::*;

pub(super) async fn add(ctx: &Ctx, a: AddArgs) -> Result<()> {
    let all = resolve::modules(ctx).await?;
    if all.is_empty() {
        return Err(failed(t!("add.no_modules")));
    }
    let given = match a.module.as_deref() {
        Some(q) => Some(resolve::module(ctx, Some(q)).await?),
        None => None,
    };

    // 1) 새로 설치 / 기존 폴더 (폴더를 고르면 게임은 폴더를 보고 알아낸다)
    let (from, move_folder) = choose_source(ctx, &a, given.as_ref())?;
    // 2) 게임
    let module = choose_game(ctx, &all, given, from.as_deref()).await?;
    if let Some(why) = platform_refusal(&module) {
        return Err(failed(why));
    }
    let module_name = s(&module, "name").to_string();
    let display = if s(&module, "display_name").is_empty() {
        module_name.clone()
    } else {
        s(&module, "display_name").to_string()
    };
    if from.is_none() && module.get("install").is_none_or(|i| i.is_null()) {
        ctx.out.warn(&t!("add.no_installer", game = display));
    }

    // 3) 이름. 고르자마자 이름·가져올 폴더가 다른 서버와 겹치는지 데몬에게 묻는다 (뒤의 질문에 답하기 전에 알린다)
    let name = choose_name(ctx, a.name.clone(), &module_name).await?;
    let mut body = json!({ "name": name, "module_name": module_name });
    if let Some(dir) = &from {
        body["migration_source"] = json!(dir);
        if move_folder {
            body["migration_mode"] = json!("move");
        }
    }
    check_new_server(ctx, &body, from.as_deref(), false).await?;

    // 4) 필요한 익스텐션
    ensure_extensions(ctx, &module).await?;

    // 5) 익스텐션이 묻는 칸 (AddServer.options). 답은 만들 때 `extension_data`로 보낸다
    let ext_data = extension_options(ctx).await?;

    // 5-1) 만들 때 고르는 모듈 설정 (`install = true`, 예: 마인크래프트 서버 종류). 설치할 파일이
    // 달라지므로 만든 뒤가 아니라 만들 때 보낸다. --set으로 주지 않았으면 새 서버일 때 묻는다
    let fields = setting_fields(&module);
    let (mut module_settings, later_set) = install_settings_from_args(&fields, &a.set, from.is_none())?;
    // 5-1-a) 추천 설정 묶음 (`[[presets]]`). --preset으로 주지 않았으면 새 서버일 때 묻는다.
    // 설치 설정보다 먼저 고른다: 프리셋이 정한 설치 설정은 다시 묻지 않는다
    let presets: Vec<&Value> = module.get("presets").and_then(Value::as_array).map(|p| p.iter().collect()).unwrap_or_default();
    let locales = module_locales(ctx, &module_name).await;
    let preset = choose_preset(ctx, a.preset.clone(), &presets, locales.as_ref(), from.is_none())?;
    let chosen_preset = presets.iter().find(|p| Some(s(p, "id")) == preset.as_deref()).copied();
    if from.is_none() && ctx.ask.enabled {
        ask_install_settings(ctx, &fields, chosen_preset, locales.as_ref(), &mut module_settings)?;
    }

    // 5-2) 버전·파일 위치·실행 방식 (GUI 마법사와 같다). 고르지 않으면 데몬 기본값: 최신, 사바쨩 폴더, 사바쨩이 관리
    let version = choose_version(ctx, &a, &module, from.is_none(), chosen_preset, &module_settings, &display).await?;
    let mut dir = a.dir.clone();
    if dir.is_none() && from.is_none() && ctx.ask.enabled {
        let choices = [t!("add.location_default"), t!("add.location_custom")];
        if ctx.ask.select(&t!("add.ask_location"), &choices, 0)? == Some(1) {
            dir = ctx.ask.text(&t!("add.ask_dir"), None, false)?;
        }
    }
    // 데몬은 전체 경로만 받는다 (--from과 같다)
    let dir = dir.map(|d| crate::cmd::config::daemon_path(ctx, std::path::Path::new(&d))).transpose()?;
    let mut managed = !a.unmanaged;
    if managed && from.is_none() && ctx.ask.enabled {
        let choices = [t!("add.run_managed"), t!("add.run_unmanaged")];
        managed = ctx.ask.select(&t!("add.ask_run"), &choices, 0)? != Some(1);
    }

    // --set 값은 만들기 전에 검사한다 (틀리면 서버도 설치 작업도 남기지 않는다)
    let mut patch = Map::new();
    for (k, v) in later_set {
        let (key, value) = typed_setting(&fields, &Value::Null, &k, &v)?;
        insert_patch(&mut patch, &key, value);
    }
    // 새로 설치하는 동안은 설치가 작업 잠금을 쥐어 설정을 저장할 수 없다: 설치가 끝난 뒤 저장하므로 기다려야 한다
    let installs = from.is_none() && module.get("install").is_some_and(|v| !v.is_null());
    if a.no_wait && installs && !patch.is_empty() {
        return Err(usage(t!("add.set_needs_wait")));
    }

    // 6) 만들기
    if let Some(p) = &preset {
        body["preset"] = json!(p);
    }
    if !module_settings.is_empty() {
        body["module_settings"] = Value::Object(module_settings);
    }
    if let Some(p) = a.game_port {
        body["port"] = json!(p);
    }
    if let Some(v) = &version {
        body["version"] = json!(v);
    }
    if let Some(d) = &dir {
        body["working_dir"] = json!(d);
    }
    if !managed {
        body["managed_start"] = json!(false);
    }
    if !ext_data.is_empty() {
        body["extension_data"] = Value::Object(ext_data);
    }
    // 6-1) 다른 서버와 겹치는지 (GUI 마법사의 확인 단계): 이름·폴더는 막고 포트는 알린다.
    // 이름·가져올 폴더는 위에서 보았고, 여기서는 고른 설치 폴더와 포트까지 본다
    check_new_server(ctx, &body, from.as_deref(), true).await?;
    // 7-0) 약관 (예: 게임 EULA). GUI처럼 만들기 전에 묻고, 묻고서 동의하지 않으면 만들지 않는다.
    // 물을 수 없으면(`--json`, 터미널 아님) `--accept`로 댄 것만 동의하고 만든 뒤 `server agree`를 안내한다
    let (mut accepted, ask) = split_agreements(&module_agreements(&module, locales.as_ref()), &a.accept, ctx.ask.enabled);
    if !ask.is_empty() {
        show_agreements(ctx, &ask);
        let names: Vec<&str> = ask.iter().map(|a| s(a, "label")).collect();
        if !ctx.ask.consent(&t!("agree.confirm", name = name, names = names.join(", "))) {
            return Err(usage(t!("add.agreements_declined")));
        }
        accepted.extend(ask);
    }
    if !accepted.is_empty() {
        body["accept_agreements"] = json!(accepted.iter().map(|a| s(a, "id")).collect::<Vec<_>>());
    }
    let created = ctx.api.create_instance(body).await.map_err(move_refusal)?;
    let id = s(&created, "id").to_string();
    crate::cmd::print_trust_warnings(ctx, &created);
    // 가져온 폴더에서 미리 알릴 것 (데몬 `notices`: 쓸 수 없는 폴더, 밖에서 켜 둔 서버)
    for code in created.get("notices").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        if let Some(text) = crate::i18n::try_t(&format!("add.notice.{code}")) {
            ctx.out.warn(&text);
        }
    }
    // 7-0-a) 만들기 전에 동의하지 않은 약관 안내 (`--accept` 오타, 물을 수 없을 때)
    offer_agreements(ctx, &id, &name, &a.accept).await;

    // 8) 설치 진행
    let provisioning = created.get("provisioning").and_then(Value::as_bool) == Some(true);
    if provisioning {
        // 저장할 설정이 남았으면 설치가 끝날 때까지 기다린다 (위에서 --no-wait와 함께 쓰는 것은 막았다)
        if a.no_wait && patch.is_empty() {
            ctx.out.done(&t!("add.created_installing", name = name), &created);
            ctx.out.hint(&format!("saba-chan-cli server progress {}", name));
            return Ok(());
        }
        let result = follow_provision(ctx, &name, true).await?;
        if let Some(err) = result.get("error").and_then(Value::as_str) {
            let rolled = ctx.api.dismiss_provision(&name).await.ok();
            let removed = rolled.and_then(|r| r.get("rolled_back").and_then(Value::as_bool)) == Some(true);
            return Err(failed(if removed {
                t!("add.install_failed_removed", error = err)
            } else {
                t!("add.install_failed", error = err)
            }));
        }
    }
    // 7) 설정 (--set). 설치가 끝난 뒤 저장한다 (설치 중에는 작업 잠금 때문에 거절된다).
    // 틀린 값은 위에서 이미 걸렀다. 데몬이 거절하면 서버는 남기고 끝에 실패로 알린다
    let mut settings_error = None;
    if !patch.is_empty() {
        match ctx.api.update_instance(&id, Value::Object(patch)).await {
            Err(e) => {
                ctx.out.warn(&t!("add.settings_failed", error = e));
                settings_error = Some(t!("add.created_settings_failed", name = name, error = e));
            }
            Ok(r) => {
                if let Some(e) = sync_failure(&r) {
                    ctx.out.warn(&e);
                    settings_error = add_settings_failure(&name, &r);
                }
            }
        }
    }

    let out = json!({ "success": true, "id": id, "name": name, "module": module_name, "provisioned": provisioning });
    if let Some(error) = settings_error {
        return Err(CliError::Partial {
            message: error,
            data: json!({ "id": id, "name": name, "module": module_name, "provisioned": provisioning }),
        }
        .into());
    }
    ctx.out.done(&t!("add.ready", name = name, game = display), &out);
    if a.start {
        return start(
            ctx,
            StartArgs {
                server: Some(id),
                ..Default::default()
            },
        )
        .await;
    }
    ctx.out.hint(&format!("saba-chan-cli start {}", shell_word(&name)));
    Ok(())
}

/// 새로 설치할지, 있는 서버 폴더를 쓸지와 그 폴더를 옮길지. 폴더는 데몬이 받는 전체 경로로
fn choose_source(ctx: &Ctx, a: &AddArgs, given: Option<&Value>) -> Result<(Option<String>, bool)> {
    let mut from = a.from.clone();
    let mut move_folder = a.move_folder;
    // 새 설치에만 있는 선택(--dir, --game-version)을 줬으면 묻지 않는다
    let fresh_only = a.dir.is_some() || a.game_version.is_some();
    if from.is_none() && ctx.ask.enabled && a.name.is_none() && !fresh_only {
        let installs = given.is_none_or(|m| m.get("install").is_some_and(|i| !i.is_null()));
        let choices = [t!("add.fresh_install"), t!("add.use_existing")];
        if ctx
            .ask
            .select(&t!("add.ask_source"), &choices, if installs { 0 } else { 1 })?
            == Some(1)
        {
            from = ctx.ask.text(&t!("add.ask_folder"), None, false)?;
            // 그 자리에 연결 / 사바쨩 폴더로 옮기기 (GUI 가져오기와 같은 선택)
            let modes = [t!("add.mode_link"), t!("add.mode_move")];
            move_folder = from.is_some() && ctx.ask.select(&t!("add.ask_mode"), &modes, 0)? == Some(1);
        }
    }
    // 데몬은 전체 경로만 받는다: 상대 경로는 데몬이 아닌 이 CLI의 지금 폴더 기준으로 푼다
    let from = from.map(|dir| crate::cmd::config::daemon_path(ctx, std::path::Path::new(&dir))).transpose()?;
    Ok((from, move_folder))
}

/// 게임: 준 모듈(폴더가 있으면 그 게임인지 확인), 아니면 폴더를 보고 알아내거나 고른다
async fn choose_game(ctx: &Ctx, all: &[Value], given: Option<Value>, from: Option<&str>) -> Result<Value> {
    Ok(match (given, from) {
        (Some(m), Some(dir)) => {
            check_folder(ctx, dir, s(&m, "name")).await?;
            m
        }
        (Some(m), None) => m,
        (None, Some(dir)) => {
            let found = detected(ctx, dir).await?;
            let hits: Vec<Value> = all
                .iter()
                .filter(|m| found.iter().any(|f| f == s(m, "name")))
                .cloned()
                .collect();
            match hits.len() {
                1 => {
                    ctx.out
                        .note(&t!("add.folder_is", dir = dir, game = resolve::module_label(&hits[0])));
                    hits.into_iter().next().unwrap()
                }
                0 => {
                    ctx.out.warn(&t!("add.folder_unrecognized", dir = dir));
                    if !ctx.ask.enabled {
                        return Err(usage(t!("add.pick_game")));
                    }
                    resolve::choose(ctx, &t!("noun.game"), all.to_vec(), resolve::module_label)?
                }
                _ => resolve::choose(ctx, &t!("noun.game"), hits, resolve::module_label)?,
            }
        }
        (None, None) => {
            if !ctx.ask.enabled {
                return Err(usage(t!("add.module_required")));
            }
            resolve::choose(ctx, &t!("noun.game"), all.to_vec(), resolve::module_label)?
        }
    })
}

/// 새 서버 이름: 준 이름, 아니면 묻는다 (기본값은 다른 서버와 겹치지 않는 게임 이름).
/// 대소문자만 다른 이름도 겹침으로 막는다 (데몬은 정확히 같은 이름만 막는다).
/// 쓸 수 없는 글자는 데몬의 만들기 전 확인(`check_new_server`)이 본다
async fn choose_name(ctx: &Ctx, given: Option<String>, module_name: &str) -> Result<String> {
    let existing: Vec<String> = resolve::servers(ctx)
        .await?
        .into_iter()
        .map(|x| x.name.to_lowercase())
        .collect();
    let taken = |n: &str| existing.contains(&n.to_lowercase());
    let name = match given {
        Some(n) => n,
        None => {
            let suggested = (1..)
                .map(|i| if i == 1 { module_name.to_string() } else { format!("{}-{}", module_name, i) })
                .find(|n| !taken(n))
                .unwrap();
            ctx.ask.text(&t!("add.ask_name"), Some(&suggested), false)?.unwrap_or(suggested)
        }
    };
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(usage(t!("add.name_empty")));
    }
    if taken(&name) {
        return Err(usage(t!("add.name_taken", name = name)));
    }
    Ok(name)
}

/// 데몬의 만들기 전 확인(`/api/instances/check`, GUI 마법사의 확인 단계): 이름·폴더가 겹치면 막는다.
/// `ports`면 겹치는 포트도 알린다. 확인하지 못하면 그대로 둔다: 만들기가 이름·폴더를 다시 본다.
/// `from`은 가져올 폴더 (가져오기는 데몬이 폴더 경로를 싣지 않는다)
async fn check_new_server(ctx: &Ctx, body: &Value, from: Option<&str>, ports: bool) -> Result<()> {
    let check = match ctx.api.check_instance(body.clone()).await {
        Ok(check) => check,
        // 옮기기로 가져올 폴더를 다른 서버가 쓰면 확인도 만들기처럼 409로 답한다
        Err(e) if api_code(&e) == Some("working_dir_in_use") => {
            let other = match e.downcast_ref::<CliError>() {
                Some(CliError::Api { body, .. }) => s(body, "instance").to_string(),
                _ => String::new(),
            };
            return Err(usage(t!("add.folder_in_use", dir = from.unwrap_or_default(), name = other)));
        }
        Err(_) => return Ok(()),
    };
    let (blockers, warnings) = check_problems(&check, from);
    if ports {
        for w in &warnings {
            ctx.out.warn(w);
        }
    }
    if !blockers.is_empty() {
        return Err(usage(blockers.join("\n")));
    }
    Ok(())
}

/// 익스텐션이 만들 때 묻는 칸 (AddServer.options)의 답
async fn extension_options(ctx: &Ctx) -> Result<Map<String, Value>> {
    let mut ext_data = Map::new();
    for d in &crate::slots::create_options(&resolve::extensions(ctx).await.unwrap_or_default()) {
        let Some(field) = d.get("field").and_then(Value::as_str) else {
            continue;
        };
        let label = d.get("label").and_then(Value::as_str).unwrap_or(field);
        let default = d.get("default").cloned().unwrap_or(Value::Null);
        let value = if s(d, "type") == "boolean" {
            bool_option(&ctx.ask, label, &default)?
        } else {
            match ctx.ask.text(label, default.as_str(), true)? {
                Some(v) if !v.is_empty() => Value::String(v),
                _ => default,
            }
        };
        if !value.is_null() {
            ext_data.insert(field.to_string(), value);
        }
    }
    Ok(ext_data)
}

/// 만들 때 정하는 모듈 설정 칸 (`install = true`)
fn is_install_field(fields: &[Value], key: &str) -> bool {
    fields.iter().any(|f| s(f, "name") == key && f.get("install") == Some(&json!(true)))
}

/// `--set`의 `KEY=VALUE` 하나
type SetPair = (String, String);

/// `--set`을 만들 때 보낼 설치 설정과 만든 뒤 저장할 나머지로 나눈다. `fresh`(새로 설치)면
/// 사바쨩이 설치할 수 있는 값만 받는다 (데몬 `not_installable`을 만들기 전에 알린다)
fn install_settings_from_args(fields: &[Value], set: &[String], fresh: bool) -> Result<(Map<String, Value>, Vec<SetPair>)> {
    let mut module_settings = Map::new();
    let mut later_set = Vec::new();
    for (k, v) in split_pairs(set)? {
        if is_install_field(fields, &k) {
            let (key, value) = typed_setting(fields, &Value::Null, &k, &v)?;
            if let Some(f) = fields.iter().find(|f| s(f, "name") == key).filter(|_| fresh) {
                let choices = install_choices(f);
                if f.get("install_options").is_some_and(Value::is_array) && !value.as_str().is_some_and(|v| choices.iter().any(|c| c == v)) {
                    return Err(usage(t!("add.not_installable", key = key, choices = choices.join(", "))));
                }
            }
            module_settings.insert(key, value);
        } else {
            later_set.push((k, v));
        }
    }
    Ok((module_settings, later_set))
}

/// 추천 설정 묶음: `--preset`, 아니면 새 서버일 때 묻는다
fn choose_preset(ctx: &Ctx, given: Option<String>, presets: &[&Value], locales: Option<&Value>, fresh: bool) -> Result<Option<String>> {
    if given.is_some() || !fresh || !ctx.ask.enabled || presets.is_empty() {
        return Ok(given);
    }
    let label = |p: &Value| {
        let id = s(p, "id");
        let localized = locales.and_then(|l| l.pointer(&format!("/presets/{}/label", id))).and_then(Value::as_str);
        localized.filter(|l| !l.is_empty()).unwrap_or(if s(p, "label").is_empty() { id } else { s(p, "label") }).to_string()
    };
    let mut choices = vec![t!("add.no_preset")];
    choices.extend(presets.iter().map(|p| label(p)));
    Ok(ctx.ask.select(&t!("add.ask_preset"), &choices, 0)?.filter(|i| *i > 0).map(|i| s(presets[i - 1], "id").to_string()))
}

/// 새 서버의 설치 설정을 묻는다. `--set`으로 주었거나 고른 추천 설정(`preset`)이 정한 칸은 묻지 않는다
fn ask_install_settings(ctx: &Ctx, fields: &[Value], preset: Option<&Value>, locales: Option<&Value>, settings: &mut Map<String, Value>) -> Result<()> {
    let preset_sets = |key: &str| preset.is_some_and(|p| p.pointer(&format!("/settings/{}", key)).is_some());
    for f in fields.iter().filter(|f| is_install_field(fields, s(f, "name"))) {
        if settings.contains_key(s(f, "name")) || preset_sets(s(f, "name")) {
            continue;
        }
        let label = setting_label(locales, f);
        let default = f.get("default").cloned().unwrap_or(Value::Null);
        // GUI와 같다: 고르지 않은(기본값 그대로인) 칸은 보내지 않는다. 데몬이 모듈 기본값을 채운다
        let value = match s(f, "field_type") {
            "select" => {
                let options = install_choices(f);
                if options.is_empty() {
                    continue;
                }
                let labels: Vec<String> = options.iter().map(|o| option_label(locales, f, o)).collect();
                let at = options.iter().position(|o| Some(o.as_str()) == default.as_str()).unwrap_or(0);
                ctx.ask.select(&label, &labels, at)?.map(|i| json!(options[i]))
            }
            "boolean" => Some(bool_option(&ctx.ask, &label, &default)?),
            // 목록·여러 줄·비밀번호·파일 칸은 만든 뒤 `server set`으로 (GUI 만들기 화면과 같다)
            "" | "text" | "number" => {
                let shown = default.as_str().map(str::to_string).or_else(|| (!default.is_null()).then(|| default.to_string()));
                match ctx.ask.text(&label, shown.as_deref(), true)? {
                    Some(v) if !v.is_empty() => Some(typed_setting(fields, &Value::Null, s(f, "name"), &v)?.1),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(v) = value.filter(|v| *v != default && !v.is_null()) {
            settings.insert(s(f, "name").to_string(), v);
        }
    }
    Ok(())
}

/// 설치할 서버 버전: `--game-version`, 아니면 받는 게임을 새로 설치할 때 묻는다 (없으면 최신)
async fn choose_version(
    ctx: &Ctx,
    a: &AddArgs,
    module: &Value,
    fresh: bool,
    preset: Option<&Value>,
    module_settings: &Map<String, Value>,
    display: &str,
) -> Result<Option<String>> {
    let downloads = module.pointer("/install/method").and_then(Value::as_str) == Some("download");
    let mut version = a.game_version.clone();
    if version.is_some() && (!fresh || !downloads) {
        return Err(usage(t!("add.version_not_chosen", game = display)));
    }
    if version.is_none() && downloads && fresh && ctx.ask.enabled {
        // 고른 추천 설정 위에 고른 설치 설정(서버 종류 등)으로 받을 수 있는 것만 (데몬 만들기가 합치는 순서와 같다).
        // 목록을 받지 못하면 최신으로 설치한다 (GUI와 같다)
        let mut chosen = preset.and_then(|p| p.get("settings")).and_then(Value::as_object).cloned().unwrap_or_default();
        chosen.extend(module_settings.clone());
        let versions = ctx.api.module_versions(s(module, "name"), false, 1, 50, Some(&Value::Object(chosen))).await.map(|v| release_versions(&v)).unwrap_or_default();
        if !versions.is_empty() {
            let mut choices = vec![t!("add.version_latest")];
            choices.extend(versions.iter().cloned());
            if let Some(i) = ctx.ask.select(&t!("add.ask_version"), &choices, 0)?.filter(|i| *i > 0) {
                version = Some(versions[i - 1].clone());
            }
        }
    }
    Ok(version)
}

/// 서버 이름에 쓸 수 없는 쉼표 (데몬 `chat::words::is_comma`와 같은 글자, 400 `invalid_name`). 띄어쓰기는 된다
pub(super) fn has_comma(name: &str) -> bool {
    name.contains([',', '，', '、', '､'])
}

/// 만들기 전 확인(`/api/instances/check`)에서 만들기를 막는 것(같은 이름, 다른 서버가 쓰는 폴더, 파일이 든 폴더)과
/// 알리기만 하는 것(다른 서버도 쓰는 포트, Windows가 예약한 포트)
/// `from`은 가져올 폴더: 가져오기는 데몬이 폴더 경로(`folder.path`)를 싣지 않는다
pub(super) fn check_problems(check: &Value, from: Option<&str>) -> (Vec<String>, Vec<String>) {
    let mut blockers = Vec::new();
    if check.pointer("/name/taken") == Some(&json!(true)) {
        blockers.push(t!("add.name_taken", name = s(&check["name"], "value")));
    }
    if check.pointer("/name/invalid").and_then(Value::as_str).is_some() {
        blockers.push(t!("add.name_comma"));
    }
    let folder = &check["folder"];
    let path = folder.get("path").and_then(Value::as_str).or(from).unwrap_or_default();
    if let Some(other) = folder.get("in_use_by").and_then(Value::as_str) {
        blockers.push(t!("add.folder_in_use", dir = path, name = other));
    }
    if folder.get("not_empty") == Some(&json!(true)) {
        blockers.push(t!("add.folder_not_empty", dir = path));
    }
    let mut warnings = Vec::new();
    for port in check.get("ports").and_then(Value::as_array).into_iter().flatten() {
        let number = port.get("port").map(Value::to_string).unwrap_or_default();
        let users: Vec<&str> = port.get("in_use_by").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        if !users.is_empty() {
            warnings.push(t!("add.port_in_use", port = number, servers = users.join(", ")));
        }
        if port.get("reserved") == Some(&json!(true)) {
            warnings.push(t!("add.port_reserved", port = number));
        }
    }
    (blockers, warnings)
}

/// 고를 수 있는 서버 버전 (`/api/module/:name/versions`): GUI처럼 정식 릴리스만, 종류가 없는 항목은 그대로
pub(super) fn release_versions(list: &Value) -> Vec<String> {
    list.get("versions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|v| v.get("type").and_then(Value::as_str).is_none_or(|t| t.is_empty() || t == "release"))
        .filter_map(|v| match v.get("id") {
            Some(id) => Some(id.as_str().map(str::to_string).unwrap_or_else(|| id.to_string())),
            None => v.as_str().map(str::to_string),
        })
        .collect()
}


/// 폴더를 보고 어떤 게임인지 (모듈 이름들)
async fn detected(ctx: &Ctx, dir: &str) -> Result<Vec<String>> {
    let r = ctx.api.detect(dir).await?;
    Ok(r.get("matches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|m| s(m, "name").to_string())
        .collect())
}

/// 기존 폴더가 이 게임 서버인지 데몬에게 물어본다
async fn check_folder(ctx: &Ctx, dir: &str, module: &str) -> Result<()> {
    let matches = detected(ctx, dir).await?;
    if matches.iter().any(|m| m == module) {
        return Ok(());
    }
    let msg = if matches.is_empty() {
        t!("add.folder_unrecognized", dir = dir)
    } else {
        t!("add.folder_other_game", dir = dir, games = matches.join(", "))
    };
    ctx.out.warn(&msg);
    ctx.ask.ensure(&t!("add.use_anyway"))
}

/// 모듈이 요구하는 익스텐션이 꺼져 있거나 없으면 켜고 설치한다 (물어본 뒤)
async fn ensure_extensions(ctx: &Ctx, module: &Value) -> Result<()> {
    let required: Vec<String> = module
        .pointer("/install/requires_extensions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    if required.is_empty() {
        return Ok(());
    }
    let exts = resolve::extensions(ctx).await?;
    let state = |id: &str| {
        exts.iter()
            .find(|e| s(e, "id") == id)
            .map(|e| e.get("enabled").and_then(Value::as_bool) == Some(true))
    };
    let missing: Vec<&String> = required.iter().filter(|id| state(id) != Some(true)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let names = missing.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
    if !ctx.ask.confirm(&t!("add.needs_extensions", ids = names), true)? {
        return Err(CliError::Cancelled.into());
    }
    for id in missing {
        if state(id).is_none() {
            let spin = crate::progress::spinner(&t!("ext.installing", id = id));
            let r = ctx.api.install_extension(id, json!({})).await;
            spin.finish_and_clear();
            r?;
        }
        let r = ctx.api.enable_extension(id).await?;
        ctx.out.done(&t!("ext.enabled", id = id), &json!({ "id": id }));
        // `extension enable`처럼 미확인·변경된 익스텐션이면 알린다
        crate::cmd::print_trust_warnings(ctx, &r);
    }
    Ok(())
}

/// 따라가던 설치 항목이 끝을 보이기 전에 사라졌다. 부르는 쪽은 항목이 있을 때만 따라가므로 성공으로 치지 않는다
pub(super) fn provision_lost(p: &Value) -> bool {
    p.get("active").and_then(Value::as_bool) != Some(true)
}

/// 항목이 사라졌는데 그 이름의 서버가 설치 중이 아닌 채로 있으면, 끝난 항목을 다른 곳(GUI 카드)에서 다음 폴링 전에 닫은 것이다.
/// 실패한 새 설치는 닫을 때 지워지므로 남은 서버는 성공으로 본다 (L65). 새로 만든 서버의 설치에만 쓴다
pub(super) fn closed_after_finish(servers: &Value, name: &str) -> bool {
    servers
        .get("servers")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find(|x| s(x, "name") == name))
        .is_some_and(|x| status_of(x) != "provisioning")
}

/// 설치 진행률을 끝날 때까지 보여 준다. Ctrl+C는 기다리기만 멈춘다 (설치는 계속).
/// `created`: 방금 만든 서버의 첫 설치다. 기존 서버의 업데이트는 실패해도 닫을 때 서버가 남으므로 사라진 항목을 성공으로 보지 않는다
pub(super) async fn follow_provision(ctx: &Ctx, name: &str, created: bool) -> Result<Value> {
    let bar = crate::progress::percent_bar(&t!("add.installing"));
    loop {
        let p = ctx.api.provision_progress(name).await?;
        if provision_lost(&p) {
            bar.finish_and_clear();
            if created && closed_after_finish(&ctx.api.servers(true).await?, name) {
                return Ok(json!({ "active": false, "done": true }));
            }
            return Err(failed(t!("progress.lost", name = name)));
        }
        if let Some(pct) = p.get("percent").and_then(Value::as_u64) {
            bar.set_position(pct.min(100));
        }
        let msg = s(&p, "message");
        if !msg.is_empty() {
            bar.set_message(msg.to_string());
        }
        if p.get("done").and_then(Value::as_bool) == Some(true) {
            bar.finish_and_clear();
            return Ok(p);
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(700)) => {}
            _ = tokio::signal::ctrl_c() => {
                bar.finish_and_clear();
                ctx.out.note(&t!("add.still_installing", name = name));
                ctx.out.hint(&format!("saba-chan-cli server progress {}", shell_word(name)));
                return Err(CliError::Cancelled.into());
            }
        }
    }
}

pub(super) async fn remove(ctx: &Ctx, query: Option<&str>, keep_settings: bool) -> Result<()> {
    let sv = resolve::server_exact(ctx, query).await?;
    let rt = runtime(ctx, &sv).await?;
    let running = status_of(&rt) == "running";
    ctx.ask.ensure(&remove_question(&sv.name, &rt, running, keep_settings))?;
    let r = ctx.api.delete_instance(&sv.id, keep_settings).await?;
    let (done, failed) = removed_message(&sv.name, &r);
    if let Some(failed) = failed {
        ctx.out.warn(&failed);
    }
    ctx.out.done(&done, &r);
    Ok(())
}

/// 지우기 전에 묻는 말. 가져온 서버(`linked_folder`)는 연결만 끊고 폴더는 남긴다고, 보관한 원래 설정이 있으면
/// 그것으로 되돌린다고(`--keep-settings`로 둘 수 있다) 알린다 (GUI 삭제 확인과 같은 내용)
pub(super) fn remove_question(name: &str, rt: &Value, running: bool, keep_settings: bool) -> String {
    let base = if running { t!("remove.confirm_running", name = name) } else { t!("remove.confirm", name = name) };
    let folder = s(rt, "linked_folder");
    let mut q = base;
    if !folder.is_empty() {
        q = format!("{} {}", q, t!("remove.unlink_folder_kept", path = folder));
    }
    // 옮겨 온 서버도 서버 폴더 밖(데이터 폴더)에 남는 설정은 되돌린다
    if rt.get("restorable_settings").and_then(Value::as_bool) == Some(true) {
        q.push(' ');
        q.push_str(&if keep_settings { t!("remove.unlink_keeps_settings") } else { t!("remove.unlink_restores") });
    }
    q
}

/// 지운 뒤 알릴 말과, 가져오기 전 설정으로 되돌리지 못한 파일이 있으면 그 경고
pub(super) fn removed_message(name: &str, r: &Value) -> (String, Option<String>) {
    let folder = s(r, "linked_folder");
    let failed = r.get("settings_restore_failed").and_then(Value::as_u64).filter(|n| *n > 0).map(|n| t!("remove.unlink_restore_failed", n = n));
    if folder.is_empty() {
        return (t!("remove.done", name = name), failed);
    }
    (t!("remove.unlinked", name = name, path = folder), failed)
}
