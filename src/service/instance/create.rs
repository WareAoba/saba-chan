//! 인스턴스 만들기: 새 서버, 갈래(branch), 가져오기(migration). 핸들러 `POST /api/instances`·`/check`가 부른다

use axum::http::StatusCode;
use serde_json::{json, Value};

use super::provision::{install_new_server, InstallJob, INSTALL_STAGING};
use super::{extension_required, invalid_field, open_user_runtime, other_instance_folders, pre_delete_hook};
use crate::ipc::IPCServer;
use crate::service::{Failure, Outcome};

/// 모듈 설정 기본값에서 인스턴스 폴더로 바뀌는 자리표시자
pub const INSTANCE_DIR_PLACEHOLDER: &str = "{instance_dir}";

/// 새 서버의 버전 (`version`): 비었으면 None(최신). 경로 조각·명령 인자가 되지 않게 글자를 제한한다
fn requested_version(payload: &Value) -> Result<Option<String>, Failure> {
    let bad = || {
        invalid_field("version", "version must be a version name")
    };
    match payload.get("version") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) if v.is_empty() => Ok(None),
        Some(Value::String(v))
            if v.len() <= 64
                && !v.starts_with('.')
                && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+')) =>
        {
            Ok(Some(v.clone()))
        }
        Some(_) => Err(bad()),
    }
}

/// 폴더에 무엇이든 들어 있는가 (없는 폴더는 비었다)
fn folder_has_entries(dir: &std::path::Path) -> bool {
    // 중간에 끝난 설치가 남긴 설치 폴더는 다음 설치가 치운다
    std::fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|e| e.file_name() != INSTALL_STAGING))
}

/// 브랜치 서버 만들기 (`branch_of`: 상위 서버 id): 만들기 요청을 상위 서버의 모듈·버전·게임 설정으로 다시 쓴다.
/// 요청에서 받는 것은 이름과 포트뿐이다. 포트 설정과 자리를 가리키는 값(인스턴스 폴더 기본값 칸, 경로 칸, 상위 서버 폴더 안)은
/// 옮기지 않아 브랜치가 제 것을 쓴다. 가져온 서버는 그 칸이 비었거나 서버 폴더 밖을 가리켜 값만 봐서는 가릴 수 없다.
/// 돌려주는 것: (다시 쓴 요청, 상위 서버). `branch_of`가 없으면 요청 그대로
fn branch_request(supervisor: &crate::supervisor::Supervisor, payload: Value) -> Result<(Value, Option<crate::instance::ServerInstance>), Failure> {
    let Some(wanted) = payload.get("branch_of").filter(|v| !v.is_null()) else {
        return Ok((payload, None));
    };
    let refuse = |status: StatusCode, code: &str, error: &str| Failure::coded(status, code, error);
    let Some(parent) = wanted.as_str().and_then(|id| supervisor.instance_store.get(id)).cloned() else {
        return Err(refuse(StatusCode::NOT_FOUND, "branch_parent_not_found", "The server to branch from does not exist"));
    };
    // 한 단계만: 브랜치 아래에 또 브랜치를 두지 않는다 (상위 서버가 사라진 브랜치는 독립 서버다)
    if parent.branch_of.as_deref().is_some_and(|id| supervisor.instance_store.get(id).is_some()) {
        return Err(refuse(StatusCode::CONFLICT, "branch_of_branch", "A branch server cannot have its own branch"));
    }
    let metadata = supervisor.module_loader.get_module(&parent.module_name).ok().map(|m| m.metadata);
    let folders = [Some(supervisor.instance_store.instance_dir(&parent.id).to_string_lossy().into_owned()), parent.working_dir.clone()];
    let inside_parent = |value: &Value| value.as_str().is_some_and(|v| folders.iter().flatten().any(|dir| !dir.is_empty() && v.contains(dir.as_str())));
    let settings: serde_json::Map<String, Value> = metadata
        .as_ref()
        .and_then(|m| m.settings.as_ref())
        .into_iter()
        .flat_map(|s| s.fields.iter())
        .filter(|f| !f.port && !crate::module_loader::CORE_PORT_FIELDS.contains(&f.name.as_str()) && f.field_type != "file" && f.format.as_deref() != Some("path"))
        .filter(|f| !f.default.as_ref().and_then(|d| d.as_str()).is_some_and(|d| d.contains(INSTANCE_DIR_PLACEHOLDER)))
        .filter_map(|f| parent.module_settings.get(&f.name).filter(|v| !v.is_null() && !inside_parent(v)).map(|v| (f.name.clone(), v.clone())))
        .collect();
    let mut request = json!({
        "name": payload.get("name"),
        "port": payload.get("port"),
        "module_name": parent.module_name,
        "module_settings": settings,
        "extension_data": parent.extension_data,
    });
    // 버전을 골라 받는 모듈이면 상위 서버와 같은 버전을 받는다
    let downloads = metadata.as_ref().and_then(|m| m.install.as_ref()).is_some_and(|i| i.method == "download");
    if let Some(version) = parent.server_version.as_ref().filter(|_| downloads) {
        let version = json!({ "version": version });
        if matches!(requested_version(&version), Ok(Some(_))) {
            request["version"] = version["version"].clone();
        }
    }
    if let Some(managed) = parent.module_settings.get("managed_start").filter(|v| v.is_boolean()) {
        request["managed_start"] = managed.clone();
    }
    Ok((request, Some(parent)))
}

/// 브랜치 서버가 만들 때 한 번 가져오는 사바쨩 설정: 데몬이 읽는 설정, 예약, 실행 인자·메모리 상한, 게임 채팅, 사용 모드와 명단, 자동 업데이트.
/// 그 뒤로는 두 서버가 따로다
fn copy_saba_settings(branch: &mut crate::instance::ServerInstance, parent: &crate::instance::ServerInstance) {
    branch.branch_of = Some(parent.id.clone());
    for (key, _) in saba_chan_updater_lib::constants::DAEMON_SETTINGS {
        if let Some(value) = parent.module_settings.get(*key) {
            branch.module_settings.insert(key.to_string(), value.clone());
        }
    }
    branch.schedule = parent.schedule.clone();
    branch.backup_keep = parent.backup_keep;
    branch.extra_args = parent.extra_args.clone();
    branch.memory_limit_mb = parent.memory_limit_mb;
    branch.protocol_mode = parent.protocol_mode.clone();
    branch.game_chat = parent.game_chat;
    branch.game_chat_keyword = parent.game_chat_keyword.clone();
    branch.access = parent.access;
    branch.controllers = parent.controllers.clone();
    branch.controllers_only = parent.controllers_only;
    branch.kick_players = parent.kick_players.clone();
    branch.auto_update = parent.auto_update;
}

/// `POST /api/instances` 본체: 새 서버를 만든다 (설치는 뒤에서 한다). 답은 만든 서버 `{success, id, …}`이고 핸들러가 201로 답한다
pub(crate) async fn create(state: &IPCServer, payload: Value) -> Outcome {
    let plan = plan(state, payload, false).await?;
    commit(state, plan).await
}

/// `POST /api/instances/check` 본체: 만들기와 같은 검사를 하고 이름·서버 폴더·포트가 겹치는지 답한다. 아무것도 만들지 않는다
pub(crate) async fn check(state: &IPCServer, payload: Value) -> Outcome {
    let plan = plan(state, payload, true).await?;
    Ok(check_answer(&plan))
}

/// 만들기 요청을 검사해 정한 것. 만들기 잠금(감독자 쓰기 잠금)을 쥔 채 `commit`으로 넘긴다
struct Plan<'a> {
    supervisor: tokio::sync::RwLockWriteGuard<'a, crate::supervisor::Supervisor>,
    /// 브랜치 서버의 상위 서버
    parent: Option<crate::instance::ServerInstance>,
    instance: crate::instance::ServerInstance,
    /// 요청한 이름과 모듈
    name: String,
    module: String,
    /// 이름에 쉼표가 있다 (확인 단계만 여기까지 온다)
    comma: bool,
    chosen_version: Option<String>,
    module_trust: Option<saba_chan_updater_lib::integrity::Trust>,
    module_install: Option<ModuleInstall>,
    folder: ServerFolder,
    /// 포트마다 그 포트를 쓰는 다른 서버 (확인 단계용)
    port_users: std::collections::BTreeMap<u16, Vec<String>>,
}

/// 이름으로 서버를 찾는 클라이언트가 있어 같은 이름은 둘 두지 않는다 (rename과 같은 409).
/// 채팅이 대소문자·공백 칸 수를 가리지 않고 찾으므로 그것만 다른 이름도 같은 이름이다
fn name_taken(supervisor: &crate::supervisor::Supervisor, name: &str) -> bool {
    supervisor.instance_store.list().iter().any(|i| crate::chat::words::same(&i.name, name))
}

/// 만들기 단계: 요청을 검사하고 인스턴스를 꾸린다 (저장하지 않는다). 확인 단계(`dry_run`)는 이름이 겹치거나 쉼표가 있어도 거절하지 않는다
async fn plan(state: &IPCServer, payload: Value, dry_run: bool) -> Result<Plan<'_>, Failure> {
    let supervisor = state.supervisor.write().await;
    let (payload, parent) = branch_request(&supervisor, payload)?;
    let (Some(name), Some(module)) = (payload.get("name").and_then(Value::as_str), payload.get("module_name").and_then(Value::as_str)) else {
        return Err(Failure::coded(StatusCode::BAD_REQUEST, "invalid_request", "Invalid request"));
    };
    if name_taken(&supervisor, name) && !dry_run {
        return Err(Failure::coded(StatusCode::CONFLICT, "name_taken", format!("An instance named '{}' already exists", name)));
    }
    // 문장 명령이 부르는 말을 쉼표로 나누므로 이름에 쉼표를 두지 않는다 (rename과 같은 400)
    let comma = name.contains(crate::chat::words::is_comma);
    if comma && !dry_run {
        return Err(super::invalid_name());
    }
    // 고른 버전(없으면 최신)과 실행 방식(`managed_start`, 없으면 데몬 기본)
    let chosen_version = requested_version(&payload)?;
    let managed_start = match payload.get("managed_start") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(invalid_field("managed_start", "managed_start must be true or false")),
    };
    let mut instance = crate::instance::ServerInstance::new(name, module);

    // 모듈 정보에서 process_name, default_port, install 설정 가져오기
    let ModuleDefaults { trust: module_trust, rest_port_defaulted, install: module_install } =
        apply_module_definition(&supervisor, &payload, &mut instance, module, managed_start, dry_run)?;

    // ── 앱 설정의 새 서버 기본값. 기본값과 같으면 비워 둬 지금처럼 읽을 때 기본값을 쓴다 ──
    let app = state.config_store.get_gui_settings().await;
    if app.new_server_backup_keep as usize != crate::service::backup::DEFAULT_KEEP {
        instance.backup_keep = Some(app.new_server_backup_keep);
    }
    if !app.new_server_auto_restart && !instance.module_settings.contains_key("auto_restart") {
        instance.module_settings.insert("auto_restart".into(), json!(false));
    }
    if let Some(parent) = &parent {
        copy_saba_settings(&mut instance, parent);
    }

    // ── 익스텐션이 만들 때 물은 칸 (`AddServer.options` → `extension_data`, Q-7). 설치 방식 값은 뒤에서 덮는다 ──
    match payload.get("extension_data") {
        None | Some(Value::Null) => {}
        Some(Value::Object(given)) => {
            for (key, value) in given.iter().filter(|(_, v)| !v.is_null()) {
                instance.extension_data.insert(key.clone(), value.clone());
            }
        }
        Some(_) => return Err(invalid_field("extension_data", "extension_data must be an object")),
    }

    // ── 필수 익스텐션 활성화 검증: 모듈 `[install] requires_extensions`는 인스턴스에 복사하지 않고 시작 때마다 모듈에서 읽는다 (D2) ──
    let required = supervisor.effective_required_extensions(&instance);
    if !required.is_empty() {
        let found = state.extension_manager.read().await.missing_required(&required);
        if !found.missing.is_empty() {
            return Err(extension_required("create instance", &found));
        }
    }

    // 버전은 내려받아 설치하는 모듈만 고른다 (SteamCMD·익스텐션 설치는 버전을 받지 않는다)
    let downloads = module_install.as_ref().and_then(|(install, _, _)| install.as_ref()).is_some_and(|i| i.method == "download");
    if chosen_version.is_some() && !downloads {
        return Err(invalid_field("version", "this module does not install a chosen version"));
    }

    let folder = choose_server_folder(&supervisor, &payload, &mut instance, &module_install, dry_run)?;
    let port_users = choose_ports(&supervisor, &payload, &mut instance, module, folder.migration_mode, rest_port_defaulted)?;
    let (name, module) = (name.to_string(), module.to_string());
    Ok(Plan { supervisor, parent, instance, name, module, comma, chosen_version, module_trust, module_install, folder, port_users })
}

/// 확인 단계의 답: 이름(겹침·제안), 서버 폴더(쓰는 서버·비었는지), 포트마다 쓰는 서버·Windows 예약
fn check_answer(plan: &Plan<'_>) -> Value {
    let Plan { supervisor, parent, instance, name, module, comma, folder, port_users, .. } = plan;
    let taken = |n: &str| name_taken(supervisor, n);
    // 브랜치는 보낸 이름(상위 서버 이름에서 온 것)을 지켜 번호만 붙인다
    let suggested = match parent {
        Some(_) => (1..).map(|i| if i == 1 { name.to_string() } else { format!("{name} {i}") }).find(|n| !taken(n)),
        None => (1..).map(|i| if i == 1 { module.to_string() } else { format!("{module}-{i}") }).find(|n| !taken(n)),
    };
    let fields = port_fields_of(supervisor, module);
    let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
    let ports: Vec<Value> = crate::validator::instance_ports(instance, &fields).into_iter().map(|(field, port)| json!({
        "field": field,
        "port": port,
        "in_use_by": port_users.get(&port).cloned().unwrap_or_default(),
        "reserved": crate::validator::port_reserved_by_windows(port),
    })).collect();
    // 이름에 쓸 수 없는 글자가 있을 때만 `invalid`를 싣는다
    let mut name_check = json!({ "value": name, "taken": taken(name), "suggested": suggested });
    if *comma {
        name_check["invalid"] = json!("comma");
    }
    json!({
        "success": true,
        "name": name_check,
        "folder": {
            "custom": folder.custom_folder,
            "path": instance.working_dir.as_deref().filter(|_| folder.custom_folder),
            "in_use_by": folder.folder_in_use,
            "not_empty": folder.folder_not_empty,
        },
        "ports": ports,
    })
}

/// 만든 답: 미검증·변경된 모듈로 만들면 알림을 싣고(`module_trust`, `warnings`), 자동 포트포워딩 확인을 건다
/// (설정이 켜져 있을 때만. 만들기 잠금이 풀린 뒤에 돈다)
fn created(state: &IPCServer, trust: Option<saba_chan_updater_lib::integrity::Trust>, module: &str, mut body: Value) -> Value {
    if let Some(trust) = trust {
        body["module_trust"] = json!(trust);
        body["warnings"] = json!(crate::module_audit::trust_warnings("module", module, trust));
    }
    if let Some(id) = body["id"].as_str().map(str::to_string) {
        let state = state.clone();
        tokio::spawn(async move { crate::port_check::auto_check(&state, vec![id]).await });
    }
    body
}

/// 만들기 단계: 인스턴스를 저장하고, 새 서버는 뒤에서 설치하고(`provision`), 있는 서버는 가져온다(`import_existing`)
async fn commit(state: &IPCServer, plan: Plan<'_>) -> Outcome {
    let Plan { mut supervisor, parent, instance, module_trust, module_install, chosen_version, folder, module, .. } = plan;
    let id = instance.id.clone();
    let instance_dir = supervisor.instance_store.instance_dir(&id);

    // 설치가 끝날 때까지 업데이트·삭제·복원·시작이 같은 폴더에 끼어들지 않게 작업 잠금을 쥔다.
    // 가져오기는 이 요청이 끝날 때까지 쥔다: 격리된 `import_settings`가 끝나면 그때 없던 데몬 항목 자리를 지우므로
    // 그 사이 시작·설정 저장·백업이 데몬 항목을 만들지 않게 한다.
    // 목록에 보이기 전에 잡는다 (보인 뒤에 잡으면 그 틈의 삭제가 먼저 잡아 설치가 지워지는 폴더로 돈다)
    let install_guard = crate::instance::lock::InstanceLock::try_take_for_writes(&id).map(|g| g.purpose(crate::instance::lock::Purpose::Provision));

    // 가져오기 "옮기기": 목록에 보이기 전에 옮긴다 (실패하면 아무것도 만들지 않는다)
    if let (Some(src), Some(dest)) = (&folder.moved_from, instance.working_dir.as_deref()) {
        if let Err(code) = crate::instance::migration::move_into(src, std::path::Path::new(dest)) {
            let _ = std::fs::remove_dir_all(&instance_dir);
            return Err(Failure::coded(StatusCode::CONFLICT, code, format!("Could not move the server folder into saba-chan ({})", code)));
        }
    }

    // 인스턴스 저장 (프로비저닝은 비동기로 수행)
    if let Err(e) = supervisor.instance_store.add(instance.clone()) {
        // 옮긴 서버 폴더는 제자리로 돌려놓는다 (저장하지 못한 인스턴스 폴더에 남기지 않게)
        if let (Some(src), Some(dest)) = (&folder.moved_from, instance.working_dir.as_deref()) {
            if let Err(back) = std::fs::rename(dest, src) {
                tracing::error!("Could not move the server folder back to {} after a failed create: {}", src.display(), back);
            }
        }
        // 이 요청이 만든 빈 설치 폴더도 남기지 않는다 (다시 만들 때 "파일이 든 폴더"가 되지 않게)
        if let Some(dir) = instance.working_dir.as_deref().filter(|_| folder.made_folder) {
            let _ = std::fs::remove_dir(dir);
        }
        return Err(Failure::coded(StatusCode::INTERNAL_SERVER_ERROR, "save_failed", format!("Failed to create instance: {}", e)));
    }
    // 브랜치 서버: 상위 서버의 명령 낱말(디스코드)도 한 번 가져온다. 서버를 부르는 낱말은 서버마다 달라야 해서 옮기지 않는다
    if let Some(parent) = &parent {
        copy_branch_words(state, &parent.id, &id);
    }
    // ── Extension hook: server.post_create (결과를 기다리지 않는다) ──
    {
        let ext_mgr = state.extension_manager.clone();
        let meta = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
        let mut ctx = crate::extension::hook_context(&instance, &instance_dir, meta.as_ref());
        ctx["migration"] = serde_json::json!(folder.migration_mode);
        let host_context = Some(state.host_context(&id));
        tokio::spawn(async move {
            let _ = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.post_create", ctx)).await;
        });
    }
    let module_name = instance.module_name.clone();
    if folder.migration_mode {
        let answer = import_existing(state, supervisor, instance, &instance_dir, &module, &module_install, folder.moved_from.as_deref()).await?;
        drop(install_guard);
        return Ok(created(state, module_trust, &module_name, answer));
    }
    // ── Native provisioning: 서버 바이너리 설치 ──
    // install 설정이 있는 모듈만 프로비저닝 (download / 익스텐션 방식)
    let Some(install_config) = module_install.as_ref().and_then(|(install, _, _)| install.clone()) else {
        return Ok(created(state, module_trust, &module_name, json!({ "success": true, "id": id })));
    };
    provision(state, supervisor, instance, &instance_dir, &module, install_config, install_guard, chosen_version, &folder).await;
    Ok(created(state, module_trust, &module_name, json!({ "success": true, "id": id, "provisioning": true })))
}

/// 브랜치 서버가 상위 서버의 디스코드 명령 낱말을 한 번 가져온다 (기다리지 않는다)
fn copy_branch_words(state: &IPCServer, parent_id: &str, branch_id: &str) {
    let (store, parent_id, branch_id) = (state.config_store.clone(), parent_id.to_string(), branch_id.to_string());
    tokio::spawn(async move {
        let copied = store.update_server_words(|table| match table.get(&parent_id).map(|w| w.commands.clone()).filter(|c| !c.is_empty()) {
            Some(commands) => {
                table.insert(branch_id, crate::chat::words::InstanceWords { words: Vec::new(), commands });
                ((), true)
            }
            None => ((), false),
        });
        if let Err(e) = copied.await {
            tracing::warn!("Could not copy the Discord command words to the branch server: {}", e);
        }
    });
}

/// 새 서버의 설치를 뒤에서 시작한다 (`install_new_server`). 진행은 프로비저닝 추적으로 알린다
#[allow(clippy::too_many_arguments)]
async fn provision(
    state: &IPCServer,
    mut supervisor: tokio::sync::RwLockWriteGuard<'_, crate::supervisor::Supervisor>,
    mut instance: crate::instance::ServerInstance,
    instance_dir: &std::path::Path,
    module: &str,
    install_config: crate::module_loader::ModuleInstallConfig,
    install_guard: Option<crate::instance::lock::InstanceLock>,
    chosen_version: Option<String>,
    folder: &ServerFolder,
) {
    let id = instance.id.clone();
    // ── 설치 방식 표시 (신규 생성): hook 조건이 본다. 설치 값은 모듈 `[install]`에만 있다 ──
    instance.apply_install_extension_data(&install_config.method, &[]);
    // 인스턴스 재저장 (extension_data 업데이트)
    let _ = supervisor.instance_store.update(&id, instance.clone());

    // 진행 문구는 데몬 언어로 (모듈이 보내는 문구는 SABA_LANG으로 이미 번역돼 온다)
    let text = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);

    // install_dir 결정: 고른 폴더, 아니면 instance_dir / install_subdir (기본 "server")
    let install_subdir = install_config.install_subdir.clone().unwrap_or_else(|| "server".to_string());
    let install_dir = match instance.working_dir.as_deref().filter(|_| folder.custom_folder) {
        Some(dir) => std::path::PathBuf::from(dir),
        None => instance_dir.join(&install_subdir),
    };
    let install_dir_str = install_dir.to_string_lossy().to_string();
    // 고른 폴더에 설치가 실패하면 그 설치가 만든 것만 치운다 (사바쨩 폴더는 롤백이 인스턴스 폴더째 지운다)
    let leftovers_dir = folder.custom_folder.then(|| install_dir.clone());

    // working_dir가 아직 설정되지 않았으면 install_dir로 설정
    if instance.working_dir.is_none() {
        instance.working_dir = Some(install_dir_str.clone());
        // 인스턴스 재저장 (working_dir 업데이트)
        let _ = supervisor.instance_store.update(&id, instance.clone());
    }

    // 초기 프로비저닝 상태 등록
    let provision_label = match install_config.method.as_str() {
        "manual" | "download" => "download".to_string(),
        other => other.to_string(),
    };
    let tracker = state.provision_tracker.clone();
    tracker.mark_creating(&id);
    tracker.update(&id, &instance.name, crate::ipc::ProvisionProgress {
        step: 0,
        total: 1,
        label: provision_label.clone(),
        message: text.t("provision.installing", &[]),
        done: false,
        error: None,
        percent: Some(0),
        steps: None, details: Default::default(),
    });

    let accepted_agreements: Vec<String> = instance.accepted_agreements.keys().cloned().collect();
    // install_server가 보는 인스턴스 설정(서버 종류처럼 받을 파일을 바꾸는 값)과 데이터 폴더
    // (설치 중에 데이터 폴더에 첫 설정을 쓰는 모듈, 좀보이드 첫 INI)
    let meta = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
    let target = crate::supervisor::InstanceInstall::of(&instance, instance_dir, meta.as_ref(), state.host_context(&id));

    // Drop supervisor lock before spawning background task
    drop(supervisor);

    crate::audit::spawn(install_new_server(InstallJob {
        install_guard,
        supervisor_arc: state.supervisor.clone(),
        module_name_owned: module.to_string(),
        tracker_done: tracker,
        name_done: instance.name.clone(),
        id_done: id,
        text,
        chosen_version,
        install_method: install_config.method.clone(),
        leftovers_dir,
        made_folder: folder.made_folder,
        install_dir_str,
        accepted_agreements,
        target,
        provision_label,
    }));
}

/// 있는 서버 가져오기 (마이그레이션): 기존 디렉토리에 직접 연결, 추가 설치 없음.
/// 모듈 `import_settings`로 게임 설정을 읽어 저장하고, 원래 설정을 보관하고, 첫 백업을 건다
async fn import_existing(
    state: &IPCServer,
    supervisor: tokio::sync::RwLockWriteGuard<'_, crate::supervisor::Supervisor>,
    mut instance: crate::instance::ServerInstance,
    instance_dir: &std::path::Path,
    module: &str,
    module_install: &Option<ModuleInstall>,
    moved_from: Option<&std::path::Path>,
) -> Outcome {
    let id = instance.id.clone();
    // ── 설치 방식 표시 (마이그레이션) ──
    // hook 조건(`instance.ext_data.install_method_<방식>`)이 맞게 동작하도록
    if let Some((Some(install), _, _)) = module_install {
        instance.apply_install_extension_data(&install.method, &[]);
    }

    // ── executable_path 자동 계산 ──
    // migration_source(=working_dir)와 server_executable(바이너리 파일명)을 결합
    // 프론트엔드가 모듈 기본값을 보낼 수 있으므로, migration에서는 항상 재계산
    if let (Some(work_dir), Some((_, _, Some(exe_name)))) = (&instance.working_dir, module_install) {
        let resolved = std::path::Path::new(work_dir).join(exe_name);
        if resolved.exists() {
            instance.executable_path = Some(resolved.to_string_lossy().to_string());
            tracing::info!("Migration: executable_path auto-resolved to '{}'", resolved.display());
        } else {
            tracing::warn!("Migration: expected executable '{}' not found in '{}'", exe_name, work_dir);
        }
    }

    // ── 기존 INI 설정 import ──
    // 모듈 호출은 데몬 락 밖에서 (C12). 필요한 값만 챙기고 락을 놓는다.
    let metadata = supervisor.module_loader.get_module(module).ok().map(|m| m.metadata);
    drop(supervisor);
    let mut detected: Option<Value> = None;
    if let Some((_, lifecycle_path, _)) = module_install {
        match migration_import_call(lifecycle_path, instance.working_dir.as_deref(), instance_dir).in_instance(Some(state.host_context(&id))).run().await {
            Ok(result) => {
                if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                    match apply_imported(&instance, instance_dir, metadata.as_ref(), module, result) {
                        Ok((imported, found)) => {
                            instance = imported;
                            detected = found;
                        }
                        Err(mut failure) => {
                            // 거두지 못했으면(옮긴 폴더를 돌려놓지 못함) 남은 인스턴스를 알린다
                            if !undo_import(state, &instance, moved_from).await {
                                failure["id"] = json!(&id);
                            }
                            return Err(Failure::new(StatusCode::UNPROCESSABLE_ENTITY, failure));
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Migration import_settings failed for '{}': {} (non-fatal)", instance.name, e);
            }
        }
    }
    // 마이그레이션 변경사항 저장 (executable_path, import된 설정 등). 못 하면 저장소에는 가져오기 전
    // 인스턴스만 남으므로 거두고 실패로 알린다
    let saved = state.supervisor.write().await.instance_store.update(&id, instance.clone());
    if let Err(e) = saved {
        tracing::warn!("Migration: could not save the imported settings of '{}': {:#}", instance.name, e);
        let mut failure = json!({ "success": false, "error": format!("Failed to save the imported settings: {:#}", e), "error_code": "save_failed" });
        if !undo_import(state, &instance, moved_from).await {
            failure["id"] = json!(&id);
        }
        return Err(Failure::new(StatusCode::INTERNAL_SERVER_ERROR, failure));
    }
    // 해제할 때 되돌릴 원래 게임 설정 파일. 약관 반영·첫 시작이 파일을 고치기 전에 보관한다
    // (옮긴 서버도 서버 폴더 밖 데이터 폴더의 설정은 남으므로 보관한다)
    save_migration_originals(state, &instance, instance_dir, metadata.as_ref()).await;
    // 가져온 서버는 설치를 거치지 않으므로, 만들 때 동의한 약관을 모듈이 지금 반영한다 (예: eula.txt)
    if !instance.accepted_agreements.is_empty() {
        let ids: Vec<String> = instance.accepted_agreements.keys().cloned().collect();
        match crate::supervisor::Supervisor::accept_agreements(&state.supervisor, &id, &ids).await {
            Ok(Ok(result)) if result.get("success").and_then(|v| v.as_bool()) != Some(false) => {}
            other => tracing::warn!("Migration: module did not apply agreements for '{}': {:?}", instance.name, other),
        }
    }
    // 모듈이 판정한 것(서버 종류, 설치된 플러그인 등)을 그대로 돌려준다
    let mut response = json!({ "success": true, "id": id });
    if let Some(detected) = detected {
        response["detected"] = detected;
    }
    // 가져온 폴더에서 미리 알릴 것: 쓸 수 없는 폴더, 사바쨩 밖에서 켜 둔 서버
    // 프로세스 목록 읽기는 막는 호출이라 런타임 밖에서
    let folder = instance.working_dir.clone().map(std::path::PathBuf::from);
    let notices = match folder {
        Some(folder) => tokio::task::spawn_blocking(move || crate::instance::migration::notices(&folder)).await.unwrap_or_default(),
        None => Vec::new(),
    };
    if !notices.is_empty() {
        response["notices"] = json!(notices);
    }
    // 가져온 시점을 백업으로 남긴다 (사용자 결정 2026-11-08): 어긋나면 이때로 되돌린다.
    // 수동 백업이라 정리에서 빠진다. 기다리지 않고, 실패해도 가져오기는 그대로다.
    // 사바쨩 밖에서 켜 둔 서버는 쓰는 중인 월드를 반쯤 베낄 수 있어 건너뛴다 (끈 뒤 직접 백업한다)
    if notices.contains(&"server_running") {
        tracing::info!("Migration: '{}' is running outside saba-chan, skipping the first backup", id);
    } else {
        let backup_state = state.clone();
        let backup_id = id.clone();
        tokio::spawn(async move {
            match crate::service::backup::take_backup(&backup_state, &backup_id, crate::backup::BackupKind::Manual).await {
                Ok(Some(b)) => tracing::info!("Migration: first backup '{}' of '{}'", b.name, backup_id),
                Ok(None) => tracing::info!("Migration: nothing to back up for '{}'", backup_id),
                Err(failure) => tracing::warn!("Migration: first backup of '{}' failed ({})", backup_id, failure.status),
            }
        });
    }
    Ok(response)
}

/// 모듈 `import_settings`가 읽은 설정을 인스턴스에 얹는다. 모듈이 돌려준 경로는 인스턴스 경계 안이어야 한다
/// (밖이면 그 모듈 오류 본문). 돌려주는 것: (얹은 인스턴스, 모듈이 판정한 것 `detected`)
fn apply_imported(
    instance: &crate::instance::ServerInstance,
    instance_dir: &std::path::Path,
    metadata: Option<&crate::module_loader::ModuleMetadata>,
    module: &str,
    mut result: Value,
) -> Result<(crate::instance::ServerInstance, Option<Value>), Value> {
    let setting_types = metadata.map(|m| m.setting_types()).unwrap_or_default();
    let mut imported = instance.clone();
    if let Some(settings) = result.get("settings").and_then(|v| v.as_object()) {
        imported.apply_imported_settings(settings, &setting_types);
        for field in metadata.iter().flat_map(|m| m.settings.iter()).flat_map(|s| &s.fields) {
            if (field.field_type == "file" || field.format.as_deref() == Some("path")) && settings.contains_key(&field.name) {
                imported.imported_settings.insert(field.name.clone());
            }
        }
        tracing::info!("Migration import_settings: {} settings imported for '{}'", settings.len(), instance.name);
    }
    let boundary = crate::module_paths::Boundary::for_instance(&imported, instance_dir, metadata);
    if let Err(e) = boundary.returned_paths(&mut result, module, "import_settings") {
        return Err(crate::plugin::error_response(&e));
    }
    // 모듈이 서버 파일을 직접 찾았으면 (예: paper-1.21.4.jar) 그것을 쓴다
    if let Some(path) = result.get("executable_path").and_then(|v| v.as_str()).filter(|p| !p.trim().is_empty()) {
        let resolved = std::path::Path::new(path);
        if resolved.is_file() {
            imported.executable_path = Some(resolved.to_string_lossy().to_string());
        }
    }
    // 쓰던 시작 스크립트에만 있던 실행 인자 (모듈이 자기가 정하는 메모리·GC 같은 인자는 뺀 것)
    if let Some(args) = result.get("extra_args").filter(|v| v.is_array()).and_then(super::settings::extra_args_from) {
        imported.extra_args = args;
    }
    Ok((imported, result.get("detected").cloned()))
}


/// 만들기 단계: 서버 폴더를 정한다 (보낸 `working_dir`, 가져오기 `migration_source`·`migration_mode`, 고른 폴더 만들기)
fn choose_server_folder(
    supervisor: &crate::supervisor::Supervisor,
    payload: &Value,
    instance: &mut crate::instance::ServerInstance,
    module_install: &Option<ModuleInstall>,
    dry_run: bool,
) -> Result<ServerFolder, Failure> {
    // 선택적 필드 설정
    if let Some(path) = payload.get("executable_path").and_then(|v| v.as_str()) {
        instance.executable_path = Some(path.to_string());
    }
    if let Some(dir) = payload.get("working_dir").and_then(|v| v.as_str()) {
        instance.working_dir = Some(dir.to_string());
    }
    // migration_source → working_dir (마이그레이션: 기존 서버 디렉토리 직접 연결)
    // `migration_mode = "move"`면 그 폴더를 인스턴스 폴더 안으로 옮겨 전속시킨다 (기본 "link": 그 자리에 연결)
    let move_mode = match payload.get("migration_mode").and_then(|v| v.as_str()) {
        None | Some("link") => false,
        Some("move") => true,
        Some(_) => {
            return Err(invalid_field("migration_mode", "migration_mode must be \"link\" or \"move\""));
        }
    };
    let mut moved_from: Option<std::path::PathBuf> = None;
    let migration_mode = if let Some(src) = payload.get("migration_source").and_then(|v| v.as_str()) {
        // 상대 경로는 데몬의 작업 폴더 기준으로 풀린다: 보내는 쪽이 절대 경로로 바꿔 보내야 한다
        if !std::path::Path::new(src).is_absolute() {
            return Err(invalid_field("migration_source", "migration_source must be an absolute path"));
        }
        if move_mode {
            // 옮길 폴더도 다른 인스턴스가 쓰는 폴더와 겹치면 안 된다
            if let Some(other) = supervisor.instance_store.folder_user(std::path::Path::new(src), &instance.id) {
                let error = json!({ "success": false, "error": format!("Server '{}' already uses this folder", other.name), "error_code": "working_dir_in_use", "instance": other.name });
                return Err(Failure::new(StatusCode::CONFLICT, error));
            }
            let subdir = module_install.as_ref().and_then(|(install, _, _)| install.as_ref()).and_then(|i| i.install_subdir.clone()).unwrap_or_else(|| "server".to_string());
            let Some(dest) = crate::instance::migration::move_target(&supervisor.instance_store.instance_dir(&instance.id), &subdir) else {
                tracing::warn!("Migration: module install_subdir '{}' is not a folder inside the instance, not moving", subdir);
                return Err(Failure::coded(StatusCode::CONFLICT, "move_failed", "The module's install folder is not inside the instance folder"));
            };
            instance.working_dir = Some(dest.to_string_lossy().into_owned());
            moved_from = Some(std::path::PathBuf::from(src));
        } else if instance.working_dir.is_none() {
            instance.working_dir = Some(src.to_string());
        }
        true
    } else {
        false
    };
    // 새 서버를 사바쨩 폴더 밖에 설치한다 (`working_dir`): 그 폴더에 바로 설치하므로 절대 경로여야 하고,
    // 파일이 든 폴더에는 설치하지 않는다 (있는 서버는 가져오기로 연결한다)
    let custom_folder = !migration_mode && instance.working_dir.is_some();
    if custom_folder && !instance.working_dir.as_deref().is_some_and(|d| std::path::Path::new(d).is_absolute()) {
        return Err(invalid_field("working_dir", "working_dir must be an absolute path"));
    }
    // 한 폴더에 서버 둘이면 서로의 월드·설정을 덮어쓴다
    let folder_in_use = instance.working_dir.as_deref()
        .and_then(|dir| supervisor.instance_store.folder_user(std::path::Path::new(dir), &instance.id))
        .map(|other| other.name.clone());
    if let Some(other) = folder_in_use.as_ref().filter(|_| !dry_run) {
        let error = json!({
            "success": false,
            "error": format!("Server '{}' already uses this folder", other),
            "error_code": "working_dir_in_use",
            "instance": other,
        });
        return Err(Failure::new(StatusCode::CONFLICT, error));
    }
    let folder_not_empty = custom_folder && instance.working_dir.as_deref().is_some_and(|d| folder_has_entries(std::path::Path::new(d)));
    if folder_not_empty && !dry_run {
        let error = json!({ "success": false, "error": "The folder already has files", "error_code": "folder_not_empty", "fields": [{"field": "working_dir"}] });
        return Err(Failure::new(StatusCode::CONFLICT, error));
    }
    // 새로 만든 폴더면 인스턴스를 저장하지 못했을 때 지운다 (빈 폴더만)
    let made_folder = custom_folder && !dry_run && instance.working_dir.as_deref().is_some_and(|d| !std::path::Path::new(d).exists());
    if custom_folder && !dry_run {
        if let Err(e) = instance.working_dir.as_deref().map_or(Ok(()), std::fs::create_dir_all) {
            let error = json!({ "success": false, "error": format!("Could not create the folder: {}", e), "error_code": "folder_unavailable", "fields": [{"field": "working_dir"}] });
            return Err(Failure::new(StatusCode::CONFLICT, error));
        }
    }
    Ok(ServerFolder { migration_mode, moved_from, custom_folder, folder_in_use, folder_not_empty, made_folder })
}

/// `choose_server_folder`가 정한 것
struct ServerFolder {
    /// 있는 서버를 가져온다
    migration_mode: bool,
    /// 가져오기 "옮기기"의 원래 폴더
    moved_from: Option<std::path::PathBuf>,
    /// 새 서버를 사바쨩 폴더 밖 고른 폴더에 설치한다
    custom_folder: bool,
    /// 그 폴더를 쓰는 다른 서버 (확인 단계용)
    folder_in_use: Option<String>,
    folder_not_empty: bool,
    /// 이 요청이 폴더를 새로 만들었다 (저장 실패 시 지운다)
    made_folder: bool,
}

/// 모듈 설정 중 포트 칸 이름
fn port_fields_of(supervisor: &crate::supervisor::Supervisor, module: &str) -> Vec<String> {
    supervisor.module_loader.get_module(module).ok()
        .and_then(|m| m.metadata.settings.as_ref().map(|s| s.port_fields().into_iter().map(str::to_string).collect()))
        .unwrap_or_default()
}

/// 만들기 단계: 포트를 정한다 (보낸 포트, 아니면 다른 서버·Windows 예약과 겹치지 않게 옮긴 기본 포트).
/// 돌려주는 것: 포트 → 그 포트를 저장해 둔 다른 서버 이름 (확인 단계가 보여 준다)
fn choose_ports(
    supervisor: &crate::supervisor::Supervisor,
    payload: &Value,
    instance: &mut crate::instance::ServerInstance,
    module: &str,
    migration_mode: bool,
    rest_port_defaulted: bool,
) -> Result<std::collections::BTreeMap<u16, Vec<String>>, Failure> {
    let given_port = payload.get("port").filter(|v| !v.is_null());
    if given_port.is_some_and(|v| v.as_u64().and_then(|p| u16::try_from(p).ok()).filter(|&p| p > 0).is_none()) {
        return Err(invalid_field("port", "port must be a number from 1 to 65535"));
    }
    // 다른 인스턴스가 저장해 둔 포트. 새 인스턴스의 포트(기본값으로 채운 REST 포트 포함)는 이것과 겹치지 않게 고른다
    let mut taken = std::collections::HashSet::new();
    // 포트 → 그 포트를 저장해 둔 다른 서버 이름 (확인 단계가 보여 준다)
    let mut port_users: std::collections::BTreeMap<u16, Vec<String>> = Default::default();
    for other in supervisor.instance_store.list() {
        let fields = port_fields_of(supervisor, &other.module_name);
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        for (_, p) in crate::validator::instance_ports(other, &fields) {
            taken.insert(p);
            let users = port_users.entry(p).or_default();
            if !users.contains(&other.name) {
                users.push(other.name.clone());
            }
        }
    }
    // Windows가 예약한 포트(예: 팰월드 REST 8212가 Hyper-V 범위 안)도 피한다: 서버가 그 포트를 열지 못한다
    let taken_or_reserved = |p: u16| taken.contains(&p) || crate::validator::port_reserved_by_windows(p);
    if let Some(port) = given_port.and_then(|v| v.as_u64()).and_then(|p| u16::try_from(p).ok()) {
        instance.port = Some(port);
        // 모듈이 `port` 설정을 선언했으면 그것도 맞춘다 (설치가 module_settings로 게임 설정 파일을 쓴다)
        let declared = supervisor.module_loader.get_module(module).ok()
            .and_then(|m| m.metadata.settings.as_ref().map(|s| s.fields.iter().any(|f| f.name == "port")))
            .unwrap_or(false);
        if declared {
            instance.module_settings.insert("port".to_string(), json!(port));
        }
    } else if !migration_mode {
        // 같은 게임을 여러 개 만들면 기본 포트가 겹친다: 다른 인스턴스와 겹치지 않게 옮긴다
        let fields = port_fields_of(supervisor, module);
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        if let Some(delta) = crate::validator::shift_ports_clear_of(instance, &fields, taken_or_reserved) {
            tracing::info!("Instance '{}': ports moved by +{} to stay clear of other instances and reserved ports", instance.name, delta);
        }
    }
    // 기본 REST 포트가 Windows 예약 범위에 들면(예: 8212가 Hyper-V 범위 8135-8234 안에 든 PC) 서버가 REST를
    // 열지 못한다: 게임 포트를 직접 정했어도, 사용자가 정하지 않은 REST 포트는 예약 범위와 다른 인스턴스의
    // 포트를 피해 옮긴다. 설치가 모듈 설정 값으로 게임 설정 파일을 쓴다
    let rest = supervisor.module_loader.get_module(module).ok().and_then(|m| Some((m.metadata.rest()?.port_setting.clone(), m.metadata.rest_port(instance)?)));
    if let Some((setting, default)) = rest.filter(|(_, p)| rest_port_defaulted && crate::validator::port_reserved_by_windows(*p)) {
        if let Some(free) = crate::validator::next_free_port(default, taken_or_reserved) {
            tracing::info!("Instance '{}': REST port {} is reserved by Windows, using {}", instance.name, default, free);
            instance.module_settings.insert(setting, json!(free));
        }
    }
    Ok(port_users)
}

/// 만들기 단계: 모듈 정의를 새 인스턴스에 옮긴다 (기본 포트, 프리셋·보낸 설정, 약관, 설정 기본값, 실행 방식).
/// 모듈을 찾지 못하면 아무것도 옮기지 않는다
fn apply_module_definition(
    supervisor: &crate::supervisor::Supervisor,
    payload: &Value,
    instance: &mut crate::instance::ServerInstance,
    module: &str,
    managed_start: Option<bool>,
    dry_run: bool,
) -> Result<ModuleDefaults, Failure> {
    let Ok(loaded_module) = supervisor.module_loader.get_module(module) else {
        return Ok(ModuleDefaults::default());
    };
    if let Some(body) = crate::module_loader::host_unsupported_body(module, &loaded_module.metadata) {
        return Err(Failure::new(StatusCode::CONFLICT, body));
    }
    let trust = Some(crate::module_audit::module_trust(&loaded_module.path));
    instance.process_name = loaded_module.metadata.process_name.clone();
    if let Some(mode) = loaded_module.metadata.protocol_mode_for(&instance.protocol_mode) {
        instance.protocol_mode = mode;
    }
    if instance.port.is_none() {
        instance.port = loaded_module.metadata.default_port;
    }

    // 고른 프리셋(`[[presets]]`)의 설정이 바탕이 되고, 따로 보낸 설정이 그 위를 덮는다
    let mut chosen = serde_json::Map::new();
    if let Some(id) = payload.get("preset").and_then(|v| v.as_str()).filter(|p| !p.is_empty()) {
        match loaded_module.metadata.presets.iter().find(|p| p.id == id) {
            Some(preset) => chosen.extend(preset.settings.clone()),
            None => {
                return Err(Failure::coded(StatusCode::BAD_REQUEST, "preset_not_found", format!("Module '{}' has no preset '{}'", module, id)));
            }
        }
    }
    if let Some(given) = payload.get("module_settings").and_then(|v| v.as_object()) {
        chosen.extend(given.clone());
    }
    // REST 포트를 모듈 기본값으로 채우는가 (사용자가 정하지 않은 값만 예약 범위 밖으로 옮긴다)
    let rest_port_defaulted =
        loaded_module.metadata.rest().is_some_and(|rest| !chosen.get(&rest.port_setting).is_some_and(|v| !v.is_null()));

    // 만들 때 동의한 약관 (`[[agreements]]` id). 이 PC에서만 받는다
    let accepted = super::requested_agreements(payload, "accept_agreements")?;
    if let Some(unknown) = accepted.iter().find(|id| !loaded_module.metadata.agreements.iter().any(|a| &a.id == *id)) {
        return Err(invalid_field("accept_agreements", format!("Module '{}' has no agreement '{}'", module, unknown)));
    }
    let now = chrono::Utc::now().to_rfc3339();
    instance.accepted_agreements = accepted.into_iter().map(|id| (id, now.clone())).collect();

    // 만들 때 고른 설정 (예: 마인크래프트 서버 종류 — 설치할 jar가 달라진다).
    // 모듈이 선언한 이름만, select는 고를 수 있는 값만 받는다
    if let Some(settings) = loaded_module.metadata.settings.as_ref() {
        for field in &settings.fields {
            let Some(value) = chosen.get(&field.name).filter(|v| !v.is_null()) else { continue };
            let allowed = field.field_type == "list" || match (&field.options, value.as_str()) {
                (Some(options), Some(v)) => options.iter().any(|o| o == v),
                (Some(_), None) => false,
                (None, _) => !value.is_object() && !value.is_array(),
            };
            // 새로 설치하는 서버는 모듈이 설치할 수 있는 값만 (가져오는 서버는 판정된 값 그대로)
            let installable = payload.get("migration_source").is_some_and(|v| !v.is_null())
                || field.install_options.as_ref().is_none_or(|ok| value.as_str().is_some_and(|v| ok.iter().any(|o| o == v)));
            if allowed && !installable {
                let error = json!({ "success": false, "error": format!("module_settings.{} cannot be installed by saba-chan", field.name), "error_code": "not_installable", "fields": [{"field": field.name}] });
                return Err(Failure::new(StatusCode::BAD_REQUEST, error));
            }
            if !allowed || crate::validator::validate_setting_value(field, Some(value)).is_err() {
                // 조용히 버리면 프리셋·기본값이 남아 고른 것과 다른 서버가 된다
                return Err(invalid_field(&field.name, format!("module_settings.{} is not a valid value", field.name)));
            }
            instance.module_settings.insert(field.name.clone(), value.clone());
            // 사용자가 보낸 값이 모듈 기본값·프리셋과 다를 때만 사용자가 정한 값이다
            // (만들기 화면은 기본값을 채워 보낸다)
            let from_module = payload.get("preset").and_then(|v| v.as_str())
                .and_then(|id| loaded_module.metadata.presets.iter().find(|p| p.id == id))
                .and_then(|p| p.settings.get(&field.name))
                .is_some_and(|v| v == value)
                || field.default.as_ref().and_then(|d| d.as_str()).is_some_and(|d| {
                    let dir = supervisor.instance_store.instance_dir(&instance.id);
                    let substituted = d.replace(INSTANCE_DIR_PLACEHOLDER, &dir.to_string_lossy());
                    value.as_str().is_some_and(|v| v == d || v == substituted)
                });
            let given = payload.get("module_settings").and_then(|v| v.get(&field.name)).is_some_and(|v| v == value);
            if given && !from_module {
                instance.user_settings.insert(field.name.clone());
            }
        }
    }

    // settings.fields의 default 값을 module_settings에 자동 채우기
    if let Some(ref settings) = loaded_module.metadata.settings {
        for field in &settings.fields {
            if !instance.module_settings.contains_key(&field.name) {
                if let Some(ref default_val) = field.default {
                    let json_val = match default_val {
                        // `{instance_dir}`: 이 인스턴스 폴더 (예: 인스턴스마다 따로 두는 게임 데이터 폴더).
                        // 기존 서버를 가져오는 경우엔 그 서버가 쓰던 위치를 모듈이 고르게 비워 둔다
                        toml::Value::String(s) if s.contains(INSTANCE_DIR_PLACEHOLDER) => {
                            if payload.get("migration_source").is_some() {
                                serde_json::json!("")
                            } else {
                                let dir = supervisor.instance_store.instance_dir(&instance.id);
                                serde_json::json!(s.replace(INSTANCE_DIR_PLACEHOLDER, &dir.to_string_lossy()))
                            }
                        }
                        toml::Value::String(s) => serde_json::json!(s),
                        toml::Value::Integer(n) => serde_json::json!(n),
                        toml::Value::Float(f) => serde_json::json!(f),
                        toml::Value::Boolean(b) => serde_json::json!(b),
                        // list 기본값 (06 §8.1: TOML 문자열 배열)
                        toml::Value::Array(_) if field.field_type == "list" => match serde_json::to_value(default_val) {
                            Ok(v) => v,
                            Err(_) => continue,
                        },
                        _ => continue,
                    };
                    instance.module_settings.insert(field.name.clone(), json_val);
                }
            }
        }
    }

    if let Some(managed) = managed_start {
        instance.module_settings.insert("managed_start".into(), json!(managed));
    }
    if !dry_run {
        open_user_runtime(&loaded_module, instance);
    }

    Ok(ModuleDefaults {
        trust,
        rest_port_defaulted,
        install: Some((
            loaded_module.metadata.install.clone(),
            loaded_module.entry_path(),  // 모듈 진입 파일 (마이그레이션 설정 import 시 사용)
            loaded_module.metadata.server_executable.clone(),  // 바이너리 파일명 (마이그레이션 시 executable_path 자동 계산)
        )),
    })
}

/// `apply_module_definition`이 알아낸 것
#[derive(Default)]
struct ModuleDefaults {
    /// 모듈 확인 상태 (만든 응답의 `module_trust`·`warnings`)
    trust: Option<saba_chan_updater_lib::integrity::Trust>,
    rest_port_defaulted: bool,
    /// (`[install]`, 모듈 진입 파일, 서버 실행 파일 이름). 모듈이 없으면 None
    install: Option<ModuleInstall>,
}

type ModuleInstall = (Option<crate::module_loader::ModuleInstallConfig>, String, Option<String>);

/// 가져오기의 모듈 `import_settings` 호출. instance_dir: 모듈이 서버 폴더 대신 인스턴스 폴더에 둘 것(가져온 서버의
/// 포터블 런타임 등)의 자리. 런타임을 받을 수 있으므로 설치와 같은 시간 한도를 주고, 이 호출만 인스턴스 폴더에 쓴다
/// (`sandbox::MIGRATION_IMPORT`)
fn migration_import_call(lifecycle_path: &str, working_dir: Option<&str>, instance_dir: &std::path::Path) -> crate::plugin::PluginCall {
    let config = json!({
        "working_dir": working_dir,
        "instance_dir": instance_dir,
        crate::sandbox::MIGRATION_IMPORT: true,
    });
    crate::plugin::PluginCall::new(lifecycle_path, "import_settings", config).timeout(crate::plugin::INSTALL_TIMEOUT_SECS)
}

/// 가져온 서버의 게임 설정 파일(모듈 `config_files`)을 인스턴스 폴더에 보관한다 (`instance::migration`).
/// 모듈이 모르거나 실패하면 보관하지 않는다: 가져오기는 막지 않고, 해제할 때 되돌릴 것이 없을 뿐이다
async fn save_migration_originals(
    state: &IPCServer,
    instance: &crate::instance::ServerInstance,
    instance_dir: &std::path::Path,
    metadata: Option<&crate::module_loader::ModuleMetadata>,
) {
    let files = match crate::supervisor::Supervisor::manage_properties(&state.supervisor, &instance.id, "config_files", None).await {
        Ok(result) if result.get("success").and_then(Value::as_bool) == Some(true) => result,
        Ok(result) => {
            tracing::info!("Migration: module did not list its config files for '{}': {}", instance.name, result.get("error_code").and_then(serde_json::Value::as_str).unwrap_or("failed"));
            return;
        }
        Err(e) => {
            tracing::warn!("Migration: could not ask the module for its config files for '{}': {}", instance.name, e);
            return;
        }
    };
    // 모듈이 밝힌 경로는 그 서버의 폴더(서버 폴더·인스턴스 폴더·선언한 데이터 폴더) 안의 것만 보관한다
    let boundary = crate::module_paths::Boundary::for_instance(instance, instance_dir, metadata);
    let files: Vec<std::path::PathBuf> = files.get("files").and_then(Value::as_array).into_iter().flatten()
        .filter_map(Value::as_str)
        .filter_map(|f| boundary.resolve(std::path::Path::new(f)))
        .collect();
    match crate::instance::migration::save(instance_dir, &files) {
        Ok(n) => tracing::info!("Migration: kept {} original config file(s) of '{}' to restore when unlinked", n, instance.name),
        Err(e) => tracing::warn!("Migration: could not keep the original config files of '{}': {}", instance.name, e),
    }
}

/// 가져오기가 실패한 새 인스턴스를 거둔다: 옮겨 온 서버 폴더는 원래 자리로 돌려놓고, 그 서버 폴더에 준 격리 실행의
/// 권한·표시를 거둔 뒤 인스턴스를 지운다. 돌려놓지 못하면 서버 파일이 든 인스턴스를 그대로 둔다(지우면 서버도 지워진다).
/// 인스턴스를 지웠으면 true
async fn undo_import(state: &IPCServer, instance: &crate::instance::ServerInstance, moved_from: Option<&std::path::Path>) -> bool {
    // 만들 때 server.post_create를 알렸으니 지울 때도 알린다
    pre_delete_hook(state, instance).await;
    let folder = match (moved_from, instance.working_dir.as_deref()) {
        (Some(src), Some(dest)) => {
            // Windows 파일 잠금 대비 재시도 (가져오기 실행기·백신이 잠깐 쥐고 있을 수 있다)
            let mut moved = None;
            for attempt in 0..3 {
                match std::fs::rename(dest, src) {
                    Ok(()) => {
                        moved = Some(src.to_path_buf());
                        break;
                    }
                    Err(e) => tracing::warn!("Could not move the server folder back to {} after a failed import ({}/3): {}", src.display(), attempt + 1, e),
                }
                if attempt < 2 {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
            if moved.is_none() {
                tracing::error!("Could not move the server folder back to {} after a failed import", src.display());
                return false;
            }
            moved
        }
        (None, dir) => dir.filter(|d| !d.trim().is_empty()).map(std::path::PathBuf::from),
        (Some(_), None) => None,
    };
    if let Some(folder) = folder {
        let others = other_instance_folders(&*state.supervisor.read().await, &instance.id);
        // 실행 파일 폴더는 그 서버 폴더 안일 때만 (옮겨 왔으면 옮기기 전 자리로 바꿔 본다)
        let exe_dir = instance.executable_path.as_deref().and_then(|p| std::path::Path::new(p).parent()).map(|d| match (moved_from, instance.working_dir.as_deref()) {
            (Some(_), Some(dest)) => d.strip_prefix(dest).map(|rel| folder.join(rel)).unwrap_or_else(|_| d.to_path_buf()),
            _ => d.to_path_buf(),
        });
        let dirs: Vec<std::path::PathBuf> = std::iter::once(folder.clone()).chain(exe_dir.filter(|d| d != &folder && crate::module_paths::is_inside(d, std::slice::from_ref(&folder)))).collect();
        let name = instance.name.clone();
        let moved_back = moved_from.is_some().then(|| folder.clone());
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(failed) = crate::sandbox::release(&dirs, &others) {
                tracing::warn!("Could not take back sandbox permissions on the server folder of '{}': {}", name, failed.join("; "));
            }
            // 인스턴스 폴더 안에서 물려받은 권한·표시를 걷고 원래 자리의 권한을 되찾는다
            if let Some(folder) = moved_back {
                if let Err(e) = crate::sandbox::adopt_parent(&folder) {
                    tracing::warn!("Could not restore the permissions of the server folder of '{}': {}", name, e);
                }
            }
        })
        .await;
    }
    // Windows 파일 잠금 대비 재시도 (삭제와 같다)
    for attempt in 0..3 {
        match state.supervisor.write().await.remove_instance(&instance.id) {
            Ok(()) => return true,
            Err(e) => tracing::warn!("Could not remove '{}' after a failed import ({}/3): {}", instance.name, attempt + 1, e),
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// 가져오기의 `import_settings`는 포터블 런타임을 받을 수 있어 설치와 같은 시간 한도를 받고,
    /// 이 호출만 인스턴스 폴더에 쓴다는 표시를 싣는다 (기본 120초면 느린 회선에서 가져온 설정을 모두 잃는다)
    #[test]
    fn the_migration_import_waits_like_an_install_and_is_marked() {
        let call = super::migration_import_call("game/lifecycle.py", Some("/srv/game"), std::path::Path::new("/data/instances/a"));
        assert_eq!(call.timeout_secs(), crate::plugin::INSTALL_TIMEOUT_SECS);
        assert_eq!(call.config()[crate::sandbox::MIGRATION_IMPORT], json!(true));
        assert_eq!(call.config()["working_dir"], json!("/srv/game"));
        assert_eq!(call.config()["instance_dir"], json!("/data/instances/a"));
    }
}
