//! 인스턴스 만들기: 새 서버, 갈래(branch), 가져오기(migration)

use super::*;
use super::delete::{other_instance_folders, pre_delete_hook};
use super::provision::{install_new_server, InstallJob, INSTALL_STAGING};

/// 모듈 설정 기본값에서 인스턴스 폴더로 바뀌는 자리표시자
pub const INSTANCE_DIR_PLACEHOLDER: &str = "{instance_dir}";

/// 필수 익스텐션이 없어 `action`(예: "create instance")을 할 수 없다는 422 응답
pub(crate) fn extension_required(action: &str, found: &crate::extension::MissingExtensions) -> axum::response::Response {
    let msg = if !found.not_installed.is_empty() {
        format!(
            "Cannot {}: required extension(s) not installed: {}. Install them in Settings → Extensions first.",
            action,
            found.not_installed.join(", ")
        )
    } else {
        format!(
            "Cannot {}: required extension(s) not enabled: {}. Enable them in Settings → Extensions first.",
            action,
            found.missing.join(", ")
        )
    };
    let error = json!({
        "error": msg,
        "error_code": "extension_required",
        "missing_extensions": found.missing,
        "not_installed": found.not_installed,
    });
    (StatusCode::UNPROCESSABLE_ENTITY, Json(error)).into_response()
}

/// POST /api/instances - 새 인스턴스 생성
pub async fn create_instance(
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    // 요청이 끊겨도(클라이언트 시간 초과) 만들기를 끝까지 마친다: 가져오기는 큰 서버 폴더에 권한을 주는 데 수십 초가 들 수 있고,
    // 중간에 멈추면 인스턴스만 남고 설정 가져오기·원래 설정 보관이 빠진다
    // 따로 돌려도 요청의 출처·언어는 지닌다 (설치 진행 문구가 요청한 화면 언어로 오게)
    match crate::audit::spawn(create_instance_to_the_end(state, payload)).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Instance creation task failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "instance creation failed" }))).into_response()
        }
    }
}

async fn create_instance_to_the_end(state: IPCServer, payload: Value) -> axum::response::Response {
    create_instance_inner(state, payload, false).await.into_response()
}

/// POST /api/instances/check - 만들기 전에 이름·서버 폴더·포트가 다른 서버와 겹치는지 본다 (만들기 화면의 확인 단계).
/// 몸체는 `POST /api/instances`와 같고 아무것도 만들거나 바꾸지 않는다. 만들기가 거절할 값(형식, 없는 프리셋 등)은
/// 같은 오류로 답한다
pub async fn check_new_instance(State(state): State<IPCServer>, Json(payload): Json<Value>) -> axum::response::Response {
    create_instance_inner(state, payload, true).await.into_response()
}

/// 새 서버의 버전 (`version`): 비었으면 None(최신). 경로 조각·명령 인자가 되지 않게 글자를 제한한다
fn requested_version(payload: &Value) -> Result<Option<String>, Box<axum::response::Response>> {
    let bad = || {
        Box::new(invalid_field("version", "version must be a version name"))
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
fn branch_request(supervisor: &crate::supervisor::Supervisor, payload: Value) -> Result<(Value, Option<crate::instance::ServerInstance>), Box<axum::response::Response>> {
    let Some(wanted) = payload.get("branch_of").filter(|v| !v.is_null()) else {
        return Ok((payload, None));
    };
    let refuse = |status: StatusCode, code: &str, error: &str| Box::new((status, Json(json!({ "success": false, "error": error, "error_code": code }))).into_response());
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
        .filter(|f| !f.port && !crate::supervisor::module_loader::CORE_PORT_FIELDS.contains(&f.name.as_str()) && f.field_type != "file" && f.format.as_deref() != Some("path"))
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

async fn create_instance_inner(state: IPCServer, payload: Value, dry_run: bool) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;
    let (payload, parent) = match branch_request(&supervisor, payload) {
        Ok(found) => found,
        Err(resp) => return *resp,
    };

    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing name")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));

    let module_name = payload
        .get("module_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing module_name")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));

    if let (Ok(name), Ok(module)) = (name, module_name) {
        // 이름으로 서버를 찾는 클라이언트가 있어 같은 이름은 둘 두지 않는다 (rename과 같은 409).
        // 채팅이 대소문자·공백 칸 수를 가리지 않고 찾으므로 그것만 다른 이름도 같은 이름이다
        let name_taken = |n: &str| supervisor.instance_store.list().iter().any(|i| crate::chat::words::same(&i.name, n));
        if name_taken(name) && !dry_run {
            let error = json!({ "success": false, "error": format!("An instance named '{}' already exists", name), "error_code": "name_taken" });
            return (StatusCode::CONFLICT, Json(error)).into_response();
        }
        // 문장 명령이 부르는 말을 쉼표로 나누므로 이름에 쉼표를 두지 않는다 (rename과 같은 400)
        let comma = name.contains(crate::chat::words::is_comma);
        if comma && !dry_run {
            return invalid_name().into_response();
        }
        // 고른 버전(없으면 최신)과 실행 방식(`managed_start`, 없으면 데몬 기본)
        let chosen_version = match requested_version(&payload) {
            Ok(v) => v,
            Err(resp) => return *resp,
        };
        let managed_start = match payload.get("managed_start") {
            None | Some(Value::Null) => None,
            Some(Value::Bool(b)) => Some(*b),
            Some(_) => {
                return invalid_field("managed_start", "managed_start must be true or false");
            }
        };
        let mut instance = crate::instance::ServerInstance::new(name, module);

        // 모듈 정보에서 process_name, default_port, install 설정 가져오기
        let ModuleDefaults { trust: module_trust, rest_port_defaulted, install: module_install } =
            match apply_module_definition(&supervisor, &payload, &mut instance, module, managed_start, dry_run) {
                Ok(found) => found,
                Err(resp) => return *resp,
            };

        // ── 앱 설정의 새 서버 기본값. 기본값과 같으면 비워 둬 지금처럼 읽을 때 기본값을 쓴다 ──
        let app = state.config_store.get_gui_settings().await;
        if app.new_server_backup_keep as usize != crate::ipc::handlers::backup::DEFAULT_KEEP {
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
            Some(_) => {
                return invalid_field("extension_data", "extension_data must be an object");
            }
        }

        // ── 필수 익스텐션 활성화 검증: 모듈 `[install] requires_extensions`는 인스턴스에 복사하지 않고 시작 때마다 모듈에서 읽는다 (D2) ──
        let required = supervisor.effective_required_extensions(&instance);
        if !required.is_empty() {
            let found = state.extension_manager.read().await.missing_required(&required);
            if !found.missing.is_empty() {
                return extension_required("create instance", &found);
            }
        }

        // 버전은 내려받아 설치하는 모듈만 고른다 (SteamCMD·익스텐션 설치는 버전을 받지 않는다)
        let downloads = module_install.as_ref().and_then(|(install, _, _)| install.as_ref()).is_some_and(|i| i.method == "download");
        if chosen_version.is_some() && !downloads {
            return invalid_field("version", "this module does not install a chosen version");
        }

        let ServerFolder { migration_mode, moved_from, custom_folder, folder_in_use, folder_not_empty, made_folder } =
            match choose_server_folder(&supervisor, &payload, &mut instance, &module_install, dry_run) {
                Ok(folder) => folder,
                Err(resp) => return *resp,
            };
        let port_users = match choose_ports(&supervisor, &payload, &mut instance, module, migration_mode, rest_port_defaulted) {
            Ok(users) => users,
            Err(resp) => return *resp,
        };

        if dry_run {
            // 브랜치는 보낸 이름(상위 서버 이름에서 온 것)을 지켜 번호만 붙인다
            let suggested = match &parent {
                Some(_) => (1..).map(|i| if i == 1 { name.to_string() } else { format!("{name} {i}") }).find(|n| !name_taken(n)),
                None => (1..).map(|i| if i == 1 { module.to_string() } else { format!("{module}-{i}") }).find(|n| !name_taken(n)),
            };
            let fields = port_fields_of(&supervisor, module);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            let ports: Vec<Value> = crate::validator::instance_ports(&instance, &fields).into_iter().map(|(field, port)| json!({
                "field": field,
                "port": port,
                "in_use_by": port_users.get(&port).cloned().unwrap_or_default(),
                "reserved": crate::validator::port_reserved_by_windows(port),
            })).collect();
            // 이름에 쓸 수 없는 글자가 있을 때만 `invalid`를 싣는다
            let mut name_check = json!({ "value": name, "taken": name_taken(name), "suggested": suggested });
            if comma {
                name_check["invalid"] = json!("comma");
            }
            return (StatusCode::OK, Json(json!({
                "success": true,
                "name": name_check,
                "folder": {
                    "custom": custom_folder,
                    "path": instance.working_dir.as_deref().filter(|_| custom_folder),
                    "in_use_by": folder_in_use,
                    "not_empty": folder_not_empty,
                },
                "ports": ports,
            }))).into_response();
        }

        let id = instance.id.clone();
        // 미검증·변경된 모듈로 만들면 알림만 싣는다 (`module_trust`, `warnings`)
        let module_name_owned = instance.module_name.clone();
        let check_state = state.clone();
        let created = move |mut body: Value| {
            if let Some(trust) = module_trust {
                body["module_trust"] = json!(trust);
                body["warnings"] = json!(super::super::server::trust_warnings("module", &module_name_owned, trust));
            }
            // 자동 포트포워딩 확인 (설정이 켜져 있을 때만). 만들기 잠금이 풀린 뒤에 돈다
            if let Some(id) = body["id"].as_str().map(str::to_string) {
                tokio::spawn(async move { crate::port_check::auto_check(&check_state, vec![id]).await });
            }
            (StatusCode::CREATED, Json(body)).into_response()
        };
        let instance_name = instance.name.clone();
        let module_name_owned = module.to_string();
        let instance_dir = supervisor.instance_store.instance_dir(&id);

        // 설치가 끝날 때까지 업데이트·삭제·복원·시작이 같은 폴더에 끼어들지 않게 작업 잠금을 쥔다.
        // 가져오기는 이 요청이 끝날 때까지 쥔다: 격리된 `import_settings`가 끝나면 그때 없던 데몬 항목 자리를 지우므로
        // 그 사이 시작·설정 저장·백업이 데몬 항목을 만들지 않게 한다.
        // 목록에 보이기 전에 잡는다 (보인 뒤에 잡으면 그 틈의 삭제가 먼저 잡아 설치가 지워지는 폴더로 돈다)
        let install_guard = super::super::backup::InstanceLock::try_take_for_writes(&id).map(|g| g.purpose(super::super::backup::Purpose::Provision));

        // 가져오기 "옮기기": 목록에 보이기 전에 옮긴다 (실패하면 아무것도 만들지 않는다)
        if let (Some(src), Some(dest)) = (&moved_from, instance.working_dir.as_deref()) {
            if let Err(code) = crate::instance::migration::move_into(src, std::path::Path::new(dest)) {
                let _ = std::fs::remove_dir_all(&instance_dir);
                let error = json!({ "success": false, "error": format!("Could not move the server folder into saba-chan ({})", code), "error_code": code });
                return (StatusCode::CONFLICT, Json(error)).into_response();
            }
        }

        // 인스턴스 저장 (프로비저닝은 비동기로 수행)
        match supervisor.instance_store.add(instance.clone()) {
            Ok(_) => {
                // 브랜치 서버: 상위 서버의 명령 낱말(디스코드)도 한 번 가져온다. 서버를 부르는 낱말은 서버마다 달라야 해서 옮기지 않는다
                if let Some(parent) = &parent {
                    let (store, parent_id, branch_id) = (state.config_store.clone(), parent.id.clone(), id.clone());
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
                // ── Extension hook: server.post_create (결과를 기다리지 않는다) ──
                {
                    let ext_mgr = state.extension_manager.clone();
                    let meta = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
                    let mut ctx = crate::extension::hook_context(&instance, &instance_dir, meta.as_ref());
                    ctx["migration"] = serde_json::json!(migration_mode);
                    let host_context = Some(state.host_context(&id));
                    tokio::spawn(async move {
                        let _ = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.post_create", ctx)).await;
                    });
                }
                if !migration_mode {
                    // ── Native provisioning: 서버 바이너리 설치 ──
                    // install 설정이 있는 모듈만 프로비저닝 (download / 익스텐션 방식)
                    let has_install = module_install.as_ref()
                        .and_then(|(install, _, _)| install.as_ref())
                        .is_some();

                    if has_install {
                        let install_config = module_install.as_ref()
                            .and_then(|(install, _, _)| install.clone())
                            .unwrap();

                        // ── 설치 방식 표시 (신규 생성): hook 조건이 본다. 설치 값은 모듈 `[install]`에만 있다 ──
                        instance.apply_install_extension_data(&install_config.method, &[]);
                        // 인스턴스 재저장 (extension_data 업데이트)
                        let _ = supervisor.instance_store.update(&id, instance.clone());

                        let tracker = state.provision_tracker.clone();
                        let supervisor_arc = state.supervisor.clone();
                        // 진행 문구는 데몬 언어로 (모듈이 보내는 문구는 SABA_LANG으로 이미 번역돼 온다)
                        let text = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);

                        // install_dir 결정: 고른 폴더, 아니면 instance_dir / install_subdir (기본 "server")
                        let install_subdir = install_config.install_subdir.clone()
                            .unwrap_or_else(|| "server".to_string());
                        let install_dir = match instance.working_dir.as_deref().filter(|_| custom_folder) {
                            Some(dir) => std::path::PathBuf::from(dir),
                            None => instance_dir.join(&install_subdir),
                        };
                        let install_dir_str = install_dir.to_string_lossy().to_string();
                        // 고른 폴더에 설치가 실패하면 그 설치가 만든 것만 치운다 (사바쨩 폴더는 롤백이 인스턴스 폴더째 지운다)
                        let leftovers_dir = custom_folder.then(|| install_dir.clone());

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
                        tracker.mark_creating(&id);
                        tracker.update(&id, &instance_name, crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: provision_label.clone(),
                            message: text.t("provision.installing", &[]),
                            done: false,
                            error: None,
                            percent: Some(0),
                            steps: None, details: Default::default(),
                        });

                        let tracker_done = tracker.clone();
                        let name_done = instance_name.clone();
                        let id_done = id.clone();
                        let accepted_agreements: Vec<String> = instance.accepted_agreements.keys().cloned().collect();
                        // install_server가 보는 인스턴스 설정(서버 종류처럼 받을 파일을 바꾸는 값)과 데이터 폴더
                        // (설치 중에 데이터 폴더에 첫 설정을 쓰는 모듈, 좀보이드 첫 INI)
                        let meta = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
                        let target = crate::supervisor::InstanceInstall::of(&instance, &instance_dir, meta.as_ref(), state.host_context(&id));

                        // Drop supervisor lock before spawning background task
                        drop(supervisor);

                        crate::audit::spawn(install_new_server(InstallJob {
                            install_guard,
                            supervisor_arc,
                            module_name_owned,
                            tracker_done,
                            name_done,
                            id_done,
                            text,
                            chosen_version,
                            install_method: install_config.method.clone(),
                            leftovers_dir,
                            made_folder,
                            install_dir_str,
                            accepted_agreements,
                            target,
                            provision_label,
                        }));

                        let response = json!({
                            "success": true,
                            "id": id,
                            "provisioning": true,
                        });
                        created(response)
                    } else {
                        created(json!({ "success": true, "id": id }))
                    }
                } else {
                    // 마이그레이션: 기존 디렉토리에 직접 연결, 추가 설치 없음

                    // ── 설치 방식 표시 (마이그레이션) ──
                    // hook 조건(`instance.ext_data.install_method_<방식>`)이 맞게 동작하도록
                    if let Some((Some(ref install), _, _)) = module_install {
                        instance.apply_install_extension_data(&install.method, &[]);
                    }

                    // ── executable_path 자동 계산 ──
                    // migration_source(=working_dir)와 server_executable(바이너리 파일명)을 결합
                    // 프론트엔드가 모듈 기본값을 보낼 수 있으므로, migration에서는 항상 재계산
                    if let (Some(work_dir), Some((_, _, Some(exe_name)))) = (&instance.working_dir, &module_install) {
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
                    let metadata = supervisor.module_loader.get_module(&module_name_owned).ok().map(|m| m.metadata);
                    let setting_types = metadata.as_ref().map(|m| m.setting_types()).unwrap_or_default();
                    drop(supervisor);
                    let mut detected: Option<Value> = None;
                    if let Some((_, ref lifecycle_path, ref _server_exe)) = module_install {
                        match migration_import_call(lifecycle_path, instance.working_dir.as_deref(), &instance_dir).in_instance(Some(state.host_context(&id))).run().await {
                            Ok(mut result) => {
                                if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                                    let mut imported = instance.clone();
                                    if let Some(settings) = result.get("settings").and_then(|v| v.as_object()) {
                                        imported.apply_imported_settings(settings, &setting_types);
                                        for field in metadata.iter().flat_map(|m| m.settings.iter()).flat_map(|s| &s.fields) {
                                            if (field.field_type == "file" || field.format.as_deref() == Some("path")) && settings.contains_key(&field.name) {
                                                imported.imported_settings.insert(field.name.clone());
                                            }
                                        }
                                        tracing::info!("Migration import_settings: {} settings imported for '{}'", settings.len(), instance_name);
                                    }
                                    let boundary = crate::module_paths::Boundary::for_instance(&imported, &instance_dir, metadata.as_ref());
                                    if let Err(e) = boundary.returned_paths(&mut result, &module_name_owned, "import_settings") {
                                        let mut failure = crate::plugin::error_response(&e);
                                        // 거두지 못했으면(옮긴 폴더를 돌려놓지 못함) 남은 인스턴스를 알린다
                                        if !undo_import(&state, &instance, moved_from.as_deref()).await {
                                            failure["id"] = json!(&id);
                                        }
                                        return (StatusCode::UNPROCESSABLE_ENTITY, Json(failure)).into_response();
                                    }
                                    instance = imported;
                                    // 모듈이 서버 파일을 직접 찾았으면 (예: paper-1.21.4.jar) 그것을 쓴다
                                    if let Some(path) = result.get("executable_path").and_then(|v| v.as_str()).filter(|p| !p.trim().is_empty()) {
                                        let resolved = std::path::Path::new(path);
                                        if resolved.is_file() {
                                            instance.executable_path = Some(resolved.to_string_lossy().to_string());
                                        }
                                    }
                                    detected = result.get("detected").cloned();
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Migration import_settings failed for '{}': {} (non-fatal)", instance_name, e);
                            }
                        }
                    }
                    // 마이그레이션 변경사항 저장 (executable_path, import된 설정 등). 못 하면 저장소에는 가져오기 전
                    // 인스턴스만 남으므로 거두고 실패로 알린다
                    let saved = state.supervisor.write().await.instance_store.update(&id, instance.clone());
                    if let Err(e) = saved {
                        tracing::warn!("Migration: could not save the imported settings of '{}': {:#}", instance_name, e);
                        let mut failure = json!({ "success": false, "error": format!("Failed to save the imported settings: {:#}", e), "error_code": "save_failed" });
                        if !undo_import(&state, &instance, moved_from.as_deref()).await {
                            failure["id"] = json!(&id);
                        }
                        return (StatusCode::INTERNAL_SERVER_ERROR, Json(failure)).into_response();
                    }
                    // 해제할 때 되돌릴 원래 게임 설정 파일. 약관 반영·첫 시작이 파일을 고치기 전에 보관한다
                    // (옮긴 서버도 서버 폴더 밖 데이터 폴더의 설정은 남으므로 보관한다)
                    save_migration_originals(&state, &instance, &instance_dir, metadata.as_ref()).await;
                    // 가져온 서버는 설치를 거치지 않으므로, 만들 때 동의한 약관을 모듈이 지금 반영한다 (예: eula.txt)
                    if !instance.accepted_agreements.is_empty() {
                        let ids: Vec<String> = instance.accepted_agreements.keys().cloned().collect();
                        match crate::supervisor::Supervisor::accept_agreements(&state.supervisor, &id, &ids).await {
                            Ok(Ok(result)) if result.get("success").and_then(|v| v.as_bool()) != Some(false) => {}
                            other => tracing::warn!("Migration: module did not apply agreements for '{}': {:?}", instance_name, other),
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
                            match super::super::backup::take_backup(&backup_state, &backup_id, crate::backup::BackupKind::Manual).await {
                                Ok(Some(b)) => tracing::info!("Migration: first backup '{}' of '{}'", b.name, backup_id),
                                Ok(None) => tracing::info!("Migration: nothing to back up for '{}'", backup_id),
                                Err(failure) => tracing::warn!("Migration: first backup of '{}' failed ({})", backup_id, failure.status),
                            }
                        });
                    }
                    created(response)
                }
            }
            Err(e) => {
                // 옮긴 서버 폴더는 제자리로 돌려놓는다 (저장하지 못한 인스턴스 폴더에 남기지 않게)
                if let (Some(src), Some(dest)) = (&moved_from, instance.working_dir.as_deref()) {
                    if let Err(back) = std::fs::rename(dest, src) {
                        tracing::error!("Could not move the server folder back to {} after a failed create: {}", src.display(), back);
                    }
                }
                // 이 요청이 만든 빈 설치 폴더도 남기지 않는다 (다시 만들 때 "파일이 든 폴더"가 되지 않게)
                if let Some(dir) = instance.working_dir.as_deref().filter(|_| made_folder) {
                    let _ = std::fs::remove_dir(dir);
                }
                let error = json!({ "error": format!("Failed to create instance: {}", e) });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({ "error": "Invalid request" });
        (StatusCode::BAD_REQUEST, Json(error)).into_response()
    }
}

/// 만들기 단계: 서버 폴더를 정한다 (보낸 `working_dir`, 가져오기 `migration_source`·`migration_mode`, 고른 폴더 만들기)
fn choose_server_folder(
    supervisor: &crate::supervisor::Supervisor,
    payload: &Value,
    instance: &mut crate::instance::ServerInstance,
    module_install: &Option<ModuleInstall>,
    dry_run: bool,
) -> Result<ServerFolder, Box<axum::response::Response>> {
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
            return Err(Box::new(invalid_field("migration_mode", "migration_mode must be \"link\" or \"move\"")));
        }
    };
    let mut moved_from: Option<std::path::PathBuf> = None;
    let migration_mode = if let Some(src) = payload.get("migration_source").and_then(|v| v.as_str()) {
        // 상대 경로는 데몬의 작업 폴더 기준으로 풀린다: 보내는 쪽이 절대 경로로 바꿔 보내야 한다
        if !std::path::Path::new(src).is_absolute() {
            return Err(Box::new(invalid_field("migration_source", "migration_source must be an absolute path")));
        }
        if move_mode {
            // 옮길 폴더도 다른 인스턴스가 쓰는 폴더와 겹치면 안 된다
            if let Some(other) = supervisor.instance_store.folder_user(std::path::Path::new(src), &instance.id) {
                let error = json!({ "success": false, "error": format!("Server '{}' already uses this folder", other.name), "error_code": "working_dir_in_use", "instance": other.name });
                return Err(Box::new((StatusCode::CONFLICT, Json(error)).into_response()));
            }
            let subdir = module_install.as_ref().and_then(|(install, _, _)| install.as_ref()).and_then(|i| i.install_subdir.clone()).unwrap_or_else(|| "server".to_string());
            let Some(dest) = crate::instance::migration::move_target(&supervisor.instance_store.instance_dir(&instance.id), &subdir) else {
                tracing::warn!("Migration: module install_subdir '{}' is not a folder inside the instance, not moving", subdir);
                let error = json!({ "success": false, "error": "The module's install folder is not inside the instance folder", "error_code": "move_failed" });
                return Err(Box::new((StatusCode::CONFLICT, Json(error)).into_response()));
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
        return Err(Box::new(invalid_field("working_dir", "working_dir must be an absolute path")));
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
        return Err(Box::new((StatusCode::CONFLICT, Json(error)).into_response()));
    }
    let folder_not_empty = custom_folder && instance.working_dir.as_deref().is_some_and(|d| folder_has_entries(std::path::Path::new(d)));
    if folder_not_empty && !dry_run {
        let error = json!({ "success": false, "error": "The folder already has files", "error_code": "folder_not_empty", "fields": [{"field": "working_dir"}] });
        return Err(Box::new((StatusCode::CONFLICT, Json(error)).into_response()));
    }
    // 새로 만든 폴더면 인스턴스를 저장하지 못했을 때 지운다 (빈 폴더만)
    let made_folder = custom_folder && !dry_run && instance.working_dir.as_deref().is_some_and(|d| !std::path::Path::new(d).exists());
    if custom_folder && !dry_run {
        if let Err(e) = instance.working_dir.as_deref().map_or(Ok(()), std::fs::create_dir_all) {
            let error = json!({ "success": false, "error": format!("Could not create the folder: {}", e), "error_code": "folder_unavailable", "fields": [{"field": "working_dir"}] });
            return Err(Box::new((StatusCode::CONFLICT, Json(error)).into_response()));
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
) -> Result<std::collections::BTreeMap<u16, Vec<String>>, Box<axum::response::Response>> {
    let given_port = payload.get("port").filter(|v| !v.is_null());
    if given_port.is_some_and(|v| v.as_u64().and_then(|p| u16::try_from(p).ok()).filter(|&p| p > 0).is_none()) {
        return Err(Box::new(invalid_field("port", "port must be a number from 1 to 65535")));
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
) -> Result<ModuleDefaults, Box<axum::response::Response>> {
    let Ok(loaded_module) = supervisor.module_loader.get_module(module) else {
        return Ok(ModuleDefaults::default());
    };
    if let Some(body) = crate::supervisor::module_loader::host_unsupported_body(module, &loaded_module.metadata) {
        return Err(Box::new((StatusCode::CONFLICT, Json(body)).into_response()));
    }
    let trust = Some(super::super::server::module_trust(&loaded_module.path));
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
                let error = json!({ "error": format!("Module '{}' has no preset '{}'", module, id), "error_code": "preset_not_found" });
                return Err(Box::new((StatusCode::BAD_REQUEST, Json(error)).into_response()));
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
    let accepted = super::super::managed::requested_agreements(payload, "accept_agreements")?;
    if let Some(unknown) = accepted.iter().find(|id| !loaded_module.metadata.agreements.iter().any(|a| &a.id == *id)) {
        return Err(Box::new(invalid_field("accept_agreements", format!("Module '{}' has no agreement '{}'", module, unknown))));
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
                let error = json!({ "error": format!("module_settings.{} cannot be installed by saba-chan", field.name), "error_code": "not_installable", "fields": [{"field": field.name}] });
                return Err(Box::new((StatusCode::BAD_REQUEST, Json(error)).into_response()));
            }
            if !allowed || crate::validator::validate_setting_value(field, Some(value)).is_err() {
                // 조용히 버리면 프리셋·기본값이 남아 고른 것과 다른 서버가 된다
                return Err(Box::new(invalid_field(&field.name, format!("module_settings.{} is not a valid value", field.name))));
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

type ModuleInstall = (Option<crate::supervisor::module_loader::ModuleInstallConfig>, String, Option<String>);

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
    metadata: Option<&crate::supervisor::module_loader::ModuleMetadata>,
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

    #[tokio::test]
    async fn contract_paths_migration_reports_outside_executables_without_saving_them() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&server).unwrap();
        let outside = tmp.path().join("outside.exe");
        std::fs::write(&outside, "outside").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        let result = serde_json::json!({"success":true,"executable_path":outside,"settings":{"motd":"untrusted"}});
        std::fs::write(dir.join("lifecycle.py"), format!("import sys\nprint('{{\"success\":true,\"functions\":[\"import_settings\"]}}' if sys.argv[1] == 'functions' else {:?})\n", result.to_string())).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server}))).await).await;
        assert_eq!(response.status.as_u16(), 422, "{}", response.body);
        assert_eq!(response.body["error_code"], "module_path_outside");
        // 가져오기가 실패했으니 아무것도 만들지 않는다
        assert!(state.supervisor.read().await.instance_store.list().is_empty());
    }

    /// 옮겨 온 서버의 가져오기가 실패하면 서버 폴더를 원래 자리로 돌려놓고 인스턴스를 만들지 않는다
    /// (남은 인스턴스를 지우면 옮겨 온 서버도 함께 지워진다)
    #[tokio::test]
    async fn a_moved_server_goes_back_when_its_import_fails() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&server).unwrap();
        std::fs::write(server.join("world.dat"), "world").unwrap();
        let outside = tmp.path().join("outside.exe");
        std::fs::write(&outside, "outside").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        let result = serde_json::json!({"success":true,"executable_path":outside});
        std::fs::write(dir.join("lifecycle.py"), format!("import sys\nprint('{{\"success\":true,\"functions\":[\"import_settings\"]}}' if sys.argv[1] == 'functions' else {:?})\n", result.to_string())).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server, "migration_mode":"move"}))).await).await;
        assert_eq!(response.status.as_u16(), 422, "{}", response.body);
        assert_eq!(std::fs::read_to_string(server.join("world.dat")).unwrap(), "world");
        assert!(state.supervisor.read().await.instance_store.list().is_empty());
        let left: Vec<_> = std::fs::read_dir(tmp.path().join("instances")).unwrap().flatten().map(|e| e.file_name()).filter(|n| n != "order.json" && n != crate::instance::STATE_DIR).collect();
        assert!(left.is_empty(), "instance folders left: {:?}", left);
    }

    /// 가져오면 그 시점을 수동 백업으로 남긴다 (모듈 `[backup] paths`의 월드)
    #[tokio::test]
    async fn an_imported_server_gets_a_first_backup() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(server.join("world")).unwrap();
        std::fs::write(server.join("world").join("level.dat"), "world").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[backup]\npaths = ['world']\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import sys\nprint('{\"success\":true,\"functions\":[]}' if sys.argv[1] == 'functions' else '{\"success\":true}')\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server}))).await).await;
        assert_eq!(response.status.as_u16(), 201, "{}", response.body);
        let id = response.body["id"].as_str().unwrap().to_string();
        let backups = tmp.path().join("instances").join(&id).join("backups");
        let mut made = Vec::new();
        for _ in 0..100 {
            made = crate::backup::list(&backups);
            if !made.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(made.len(), 1, "one backup right after the import");
        assert_eq!(made[0].kind, crate::backup::BackupKind::Manual, "kept out of pruning");
    }

    /// 가져온 설정을 저장하지 못하면 성공으로 알리지 않는다: 저장소에는 가져오기 전 인스턴스(실행 파일·설정 없음)만 남는다
    #[tokio::test]
    async fn an_import_whose_result_cannot_be_saved_is_not_reported_as_created() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        // 데몬의 모듈 폴더 아래에 둔다: 그 밖의 모듈은 Windows 격리 실행에서 인스턴스 폴더 쓰기를 받지 못한다
        let modules = crate::plugin::resolve_modules_dir();
        let fixture = tempfile::Builder::new().prefix("import-save-").tempdir_in(&modules).unwrap();
        let dir = fixture.path();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = '{name}'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n")).unwrap();
        // 가져오는 동안 instance.json 자리가 폴더로 바뀐다 (저장이 실패하는 디스크 상태 대신).
        // 기록은 테스트가 망가뜨린다: Windows 격리 실행은 인스턴스 폴더의 데몬 항목을 바꾸지 못한다
        std::fs::write(dir.join("lifecycle.py"),
            "import json, os, sys, time\nc = json.load(sys.stdin)\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings']}))\n\
             else:\n    d = c['instance_dir']\n    open(os.path.join(d, 'started'), 'w').close()\n    \
             for _ in range(300):\n        if os.path.exists(os.path.join(d, 'go')): break\n        time.sleep(0.1)\n    \
             print(json.dumps({'success': True, 'settings': {'motd': 'mine'}}))\n",
        ).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let instances = tmp.path().join("instances");
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), instances.to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&server).unwrap();

        let request = serde_json::json!({"name":"g", "module_name":name, "migration_source":server});
        let created = tokio::spawn({
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(request)).await).await }
        });
        let mut instance_dir = None;
        for _ in 0..300 {
            instance_dir = std::fs::read_dir(&instances).into_iter().flatten().flatten().map(|e| e.path()).find(|p| p.join("started").exists());
            if instance_dir.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let instance_dir = instance_dir.expect("the import started");
        let record = instance_dir.join("instance.json");
        std::fs::remove_file(&record).unwrap();
        std::fs::create_dir(&record).unwrap();
        std::fs::write(instance_dir.join("go"), b"").unwrap();

        let response = created.await.unwrap();
        assert_eq!(response.status.as_u16(), 500, "{}", response.body);
        assert_eq!(response.body["error_code"], "save_failed");
        assert!(server.is_dir());
    }

    /// 가져오기가 오래 걸려 클라이언트가 요청을 끊어도(GUI 시간 초과) 데몬은 설정 가져오기까지 마친다:
    /// 중간에 멈추면 기본값인 인스턴스만 남고 되돌릴 원래 설정도 보관되지 않는다
    #[tokio::test]
    async fn an_import_cut_off_by_the_client_still_finishes() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"),
            "import json, sys, time\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings']}))\n\
             else:\n    time.sleep(1.5)\n    print(json.dumps({'success': True, 'settings': {'motd': 'mine'}}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&server).unwrap();

        let request = super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server})));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(300), request).await.is_err(), "the client gave up first");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let imported = state.supervisor.read().await.instance_store.list().first().and_then(|i| i.module_settings.get("motd").cloned());
            if imported == Some(serde_json::json!("mine")) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the import never finished after the request was dropped");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// 가져오기는 끝날 때까지 그 인스턴스의 작업 잠금을 쥔다: 그동안 시작·설정 저장·백업이 데몬 항목을 만들지 않는다
    /// (격리된 가져오기가 끝나면 시작할 때 없던 데몬 항목 자리를 지우므로, 그 사이 데몬이 만든 것이 지워지지 않게)
    #[tokio::test]
    async fn an_import_holds_the_instance_lock_until_it_is_done() {
        use axum::{extract::{Path, State}, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"),
            "import json, sys, time\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings']}))\n\
             else:\n    time.sleep(1.5)\n    print(json.dumps({'success': True, 'settings': {'motd': 'mine'}}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&server).unwrap();

        let request = tokio::spawn(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server}))));
        let id = loop {
            if let Some(id) = state.supervisor.read().await.instance_store.list().first().map(|i| i.id.clone()) {
                break id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        assert!(super::super::super::backup::InstanceLock::take(&id).is_none(), "the import holds the lock");
        let saved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "motd": "theirs" }))).await).await;
        assert_eq!(saved.status, 409, "{}", saved.body);
        let created = crate::ipc::internal::into_internal(request.await.unwrap()).await;
        assert_eq!(created.status, 201, "{}", created.body);
        assert!(super::super::super::backup::InstanceLock::take(&id).is_some(), "the lock is released once the import is done");
    }

    /// 가져올 서버 폴더는 절대 경로여야 한다: 상대 경로는 데몬의 작업 폴더 기준으로 풀려 엉뚱한 폴더를 연결하거나 옮긴다
    #[tokio::test]
    async fn a_relative_migration_source_is_refused() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        for mode in ["link", "move"] {
            let bad = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":"rel/dir", "migration_mode":mode}))).await).await;
            assert_eq!((bad.status.as_u16(), bad.body["error_code"].as_str(), bad.body["fields"][0]["field"].as_str()), (400, Some("invalid_format"), Some("migration_source")), "{mode}: {}", bad.body);
        }
        assert!(state.supervisor.read().await.instance_store.list().is_empty());
    }

    /// 가져오기 "옮기기"는 서버 폴더를 인스턴스 폴더 안으로 옮겨 전속시킨다: 모듈은 옮긴 자리를 읽고,
    /// 지우면 서버 파일도 함께 지워진다. 모르는 방식은 아무것도 만들지 않고 거절한다
    #[tokio::test]
    async fn moving_an_existing_server_makes_it_part_of_the_instance() {
        use axum::{extract::{Path, Query, State}, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"),
            "import json, os, sys\nc = json.load(sys.stdin)\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings']}))\n\
             else: print(json.dumps({'success': True, 'settings': {'motd': open(os.path.join(c['working_dir'], 'motd.txt')).read(), 'seen_instance_dir': c.get('instance_dir')}}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("기존 서버");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::write(server.join("motd.txt"), "mine").unwrap();

        let bad = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"x", "module_name":"game", "migration_source":server, "migration_mode":"copy"}))).await).await;
        assert_eq!(bad.body["error_code"], "invalid_format", "{}", bad.body);
        assert!(server.is_dir() && state.supervisor.read().await.instance_store.list().is_empty());

        let created = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server, "migration_mode":"move"}))).await).await;
        assert_eq!(created.status.as_u16(), 201, "{}", created.body);
        let id = created.body["id"].as_str().unwrap().to_string();
        let (instance, instance_dir) = {
            let sup = state.supervisor.read().await;
            (sup.instance_store.get(&id).unwrap().clone(), sup.instance_store.instance_dir(&id))
        };
        assert!(!server.exists(), "the folder was moved, not copied");
        let moved = instance_dir.join("server");
        assert_eq!(instance.working_dir.as_deref(), Some(moved.to_string_lossy().as_ref()));
        assert_eq!(instance.module_settings["motd"], "mine", "the module read the moved folder");
        // 가져오기도 인스턴스 폴더를 안다 (모듈이 포터블 런타임 같은 것을 서버 폴더 대신 그곳에 둔다)
        assert_eq!(instance.module_settings["seen_instance_dir"], serde_json::json!(instance_dir.to_string_lossy()));
        assert_eq!(crate::instance::migration::linked_folder(instance.working_dir.as_deref(), &instance_dir), None);

        let removed = crate::ipc::internal::into_internal(super::delete_instance(Path(id), State(state.clone()), Query(Default::default())).await).await;
        assert_eq!(removed.status.as_u16(), 200, "{}", removed.body);
        assert!(removed.body.get("linked_folder").is_none());
        assert!(!moved.exists());
    }

    #[tokio::test]
    async fn contract_paths_migration_keeps_declared_external_data_roots_after_reload() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        let server = tmp.path().join("server");
        let data = tmp.path().join("external-data");
        for folder in [&dir, &server, &data] { std::fs::create_dir_all(folder).unwrap(); }
        let executable = data.join("server.exe");
        std::fs::write(&executable, "server").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[[settings.fields]]\nname = 'data_home'\ntype = 'file'\n\n[extensions.saba-core]\ndata_roots = ['{data_home}']\n").unwrap();
        let result = serde_json::json!({"success":true,"executable_path":executable,"settings":{"data_home":data}});
        std::fs::write(dir.join("lifecycle.py"), format!("import sys\nprint('{{\"success\":true,\"functions\":[\"import_settings\"]}}' if sys.argv[1] == 'functions' else {:?})\n", result.to_string())).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server}))).await).await;
        assert_eq!(response.status.as_u16(), 201, "{}", response.body);
        let mut sup = state.supervisor.write().await;
        sup.instance_store.load().unwrap();
        let instance = sup.instance_store.get(response.body["id"].as_str().unwrap()).unwrap();
        assert_eq!(instance.executable_path.as_deref(), executable.to_str());
        assert!(instance.imported_settings.contains("data_home"));
        assert!(!instance.user_settings.contains("data_home"));
        let metadata = sup.module_loader.get_module("game").unwrap().metadata;
        let boundary = crate::module_paths::Boundary::for_instance(instance, &sup.instance_store.instance_dir(&instance.id), Some(&metadata));
        assert!(boundary.resolve(&data.join("deleted/Saves")).is_some());
    }

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

    /// D3·M1: 이 OS를 지원하지 않는 모듈로는 서버를 만들지 않는다
    #[tokio::test]
    async fn create_refuses_a_module_that_does_not_support_this_os() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        let other = if cfg!(windows) { "linux" } else { "windows" };
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[extensions.saba-core]\nplatforms = [\"{other}\"]\n"),
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(
            super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await,
        )
        .await;
        assert_eq!(made.status, axum::http::StatusCode::CONFLICT, "{:?}", made.body);
        assert_eq!(made.body["error_code"], "module_platform_unsupported");
        assert!(state.supervisor.read().await.instance_store.list().is_empty());
    }

    /// 새 서버는 사바쨩이 설치할 수 있는 값(`install_options`)으로만 만든다. 가져오는 서버는 판정된 값 그대로 받는다
    #[tokio::test]
    async fn a_new_server_takes_only_installable_options() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"server_type\"\ntype = \"select\"\noptions = [\"vanilla\", \"spigot\"]\ndefault = \"vanilla\"\ninstall = true\ninstall_options = [\"vanilla\"]\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };

        let refused = create(json!({ "name": "a", "module_name": "game", "module_settings": { "server_type": "spigot" } })).await;
        assert_eq!(refused.status.as_u16(), 400, "{:?}", refused.body);
        assert_eq!(refused.body["error_code"], "not_installable");
        assert_eq!(refused.body["fields"][0]["field"], "server_type");
        let null_source = create(json!({ "name": "a", "module_name": "game", "migration_source": null, "module_settings": { "server_type": "spigot" } })).await;
        assert_eq!(null_source.body["error_code"], "not_installable", "null is not an import");
        assert!(state.supervisor.read().await.instance_store.list().is_empty());

        let made = create(json!({ "name": "a", "module_name": "game", "module_settings": { "server_type": "vanilla" } })).await;
        assert!(made.status.is_success(), "{:?}", made.body);

        let existing = tmp.path().join("old-server");
        std::fs::create_dir_all(&existing).unwrap();
        let imported = create(json!({ "name": "b", "module_name": "game", "migration_source": existing.to_string_lossy(), "module_settings": { "server_type": "spigot" } })).await;
        assert!(imported.status.is_success(), "{:?}", imported.body);
    }

    #[tokio::test]
    async fn create_applies_the_chosen_preset_under_explicit_settings() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"max_players\"\ntype = \"number\"\ndefault = 20\n\n\
             [[settings.fields]]\nname = \"pvp\"\ntype = \"boolean\"\ndefault = true\n\n\
             [[presets]]\nid = \"friends\"\n[presets.settings]\nmax_players = 10\npvp = false\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };

        let made = create(json!({ "name": "a", "module_name": "game", "preset": "friends", "module_settings": { "pvp": true } })).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let instance = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(instance.module_settings["max_players"], json!(10), "preset value");
        assert_eq!(instance.module_settings["pvp"], json!(true), "explicit setting wins");

        let bad = create(json!({ "name": "c", "module_name": "game", "preset": "friends", "module_settings": { "max_players": "lots" } })).await;
        assert_eq!(bad.status.as_u16(), 400);
        assert_eq!(bad.body["fields"][0]["field"], "max_players");

        let unknown = create(json!({ "name": "b", "module_name": "game", "preset": "nope" })).await;
        assert_eq!(unknown.status.as_u16(), 400);
        assert_eq!(unknown.body["error_code"], "preset_not_found");
        let twin = create(json!({ "name": "a", "module_name": "game" })).await;
        assert_eq!((twin.status.as_u16(), twin.body["error_code"].as_str()), (409, Some("name_taken")));
        assert_eq!(state.supervisor.read().await.instance_store.list().len(), 1, "the second server is not made");
    }

    /// 브랜치 서버 (`branch_of`): 상위 서버의 모듈·설정을 만들 때 한 번 옮기고 포트는 따로 정한다. 그 뒤로는 서로 따로다.
    /// 브랜치는 한 단계만 두고, 상위 서버를 지우면 브랜치는 독립 서버로 남는다
    #[tokio::test]
    async fn a_branch_copies_its_parents_settings_once_and_stays_one_level() {
        use axum::extract::{Path, Query, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\ndefault_port = 25565\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\nport = true\n\n\
             [[settings.fields]]\nname = \"max_players\"\ntype = \"number\"\ndefault = 20\n\n\
             [[settings.fields]]\nname = \"data_dir\"\ntype = \"text\"\ndefault = \"{instance_dir}/data\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };
        let get = |id: String| {
            let state = state.clone();
            async move { state.supervisor.read().await.instance_store.get(&id).cloned().unwrap() }
        };

        let made = create(json!({ "name": "main", "module_name": "game", "port": 25565, "module_settings": { "max_players": 5 } })).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let parent_id = made.body["id"].as_str().unwrap().to_string();
        {
            let mut sup = state.supervisor.write().await;
            let mut parent = sup.instance_store.get(&parent_id).cloned().unwrap();
            parent.memory_limit_mb = Some(2048);
            parent.extra_args = vec!["--nogui".into()];
            parent.module_settings.insert("auto_restart".into(), json!(false));
            sup.instance_store.update(&parent_id, parent).unwrap();
        }
        let parent = get(parent_id.clone()).await;

        // 확인 단계: 비어 있는 포트를 알려 준다
        let check = crate::ipc::internal::into_internal(super::check_new_instance(State(state.clone()), Json(json!({ "name": "main test", "branch_of": parent_id }))).await).await;
        assert!(check.status.is_success(), "{:?}", check.body);
        let free = check.body["ports"][0]["port"].as_u64().unwrap();
        assert_ne!(free, 25565, "{:?}", check.body);
        assert_eq!(state.supervisor.read().await.instance_store.list().len(), 1, "a check makes nothing");

        let branched = create(json!({ "name": "main test", "branch_of": parent_id, "module_name": "other", "module_settings": { "max_players": 99 } })).await;
        assert!(branched.status.is_success(), "{:?}", branched.body);
        let branch_id = branched.body["id"].as_str().unwrap().to_string();
        let branch = get(branch_id.clone()).await;
        assert_eq!(branch.branch_of.as_deref(), Some(parent_id.as_str()));
        assert_eq!(branch.module_name, "game", "the parent's module, whatever the request says");
        assert_eq!(branch.module_settings["max_players"], json!(5), "the parent's setting");
        assert_eq!(branch.port.map(u64::from), Some(free), "its own port");
        assert_eq!((branch.memory_limit_mb, branch.extra_args.clone(), branch.module_settings["auto_restart"].clone()), (Some(2048), vec!["--nogui".to_string()], json!(false)));
        // 상위 서버 폴더를 가리키는 값은 옮기지 않는다: 브랜치는 제 폴더를 쓴다
        assert_ne!(branch.module_settings["data_dir"], parent.module_settings["data_dir"]);
        assert!(branch.module_settings["data_dir"].as_str().unwrap().contains(&branch_id));

        // 같은 이름으로 한 번 더 만들려 하면 그 이름에 번호를 붙여 권한다 (모듈 이름으로 바꾸지 않는다)
        let again = crate::ipc::internal::into_internal(super::check_new_instance(State(state.clone()), Json(json!({ "name": "main test", "branch_of": parent_id }))).await).await;
        assert_eq!((again.body["name"]["taken"].as_bool(), again.body["name"]["suggested"].as_str()), (Some(true), Some("main test 2")), "{:?}", again.body);

        // 그 뒤로는 따로: 상위 서버를 고쳐도 브랜치는 그대로다
        {
            let mut sup = state.supervisor.write().await;
            let mut parent = sup.instance_store.get(&parent_id).cloned().unwrap();
            parent.module_settings.insert("max_players".into(), json!(50));
            sup.instance_store.update(&parent_id, parent).unwrap();
        }
        assert_eq!(get(branch_id.clone()).await.module_settings["max_players"], json!(5));

        let deeper = create(json!({ "name": "deeper", "branch_of": branch_id })).await;
        assert_eq!((deeper.status.as_u16(), deeper.body["error_code"].as_str()), (409, Some("branch_of_branch")), "{:?}", deeper.body);
        let orphan = create(json!({ "name": "orphan", "branch_of": "nope" })).await;
        assert_eq!((orphan.status.as_u16(), orphan.body["error_code"].as_str()), (404, Some("branch_parent_not_found")), "{:?}", orphan.body);

        let removed = crate::ipc::internal::into_internal(super::delete_instance(Path(parent_id.clone()), State(state.clone()), Query(super::DeleteQuery { keep_settings: false })).await).await;
        assert!(removed.status.is_success(), "{:?}", removed.body);
        assert_eq!(get(branch_id).await.branch_of, None, "the branch stays as a server of its own");
    }

    /// 가져온 서버의 브랜치도 제 월드·데이터 폴더를 쓴다: 가져온 서버는 `{instance_dir}` 기본값 칸이 비어 있고
    /// 경로 칸이 서버 폴더 밖(쓰던 자리)을 가리키는데, 그 값을 옮기면 두 서버가 같은 월드를 연다
    #[tokio::test]
    async fn a_branch_of_an_imported_server_gets_its_own_data_folders() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\ndefault_port = 25565\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\nport = true\n\n\
             [[settings.fields]]\nname = \"world_name\"\ntype = \"text\"\ndefault = \"world\"\n\n\
             [[settings.fields]]\nname = \"cache_dir\"\ntype = \"text\"\ndefault = \"{instance_dir}/data\"\n\n\
             [[settings.fields]]\nname = \"world_dir\"\ntype = \"text\"\nformat = \"path\"\ndefault = \"{instance_dir}/Worlds\"\n\n\
             [[settings.fields]]\nname = \"data_home\"\ntype = \"text\"\nformat = \"path\"\ndefault = \"\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };

        let made = create(json!({ "name": "old", "module_name": "game", "port": 25565 })).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let parent_id = made.body["id"].as_str().unwrap().to_string();
        // 가져오기가 남기는 값: `{instance_dir}` 기본값 칸은 비고, 경로 칸은 쓰던 자리를 가리킨다
        let old_home = tmp.path().join("oldhome");
        {
            let mut sup = state.supervisor.write().await;
            let mut parent = sup.instance_store.get(&parent_id).cloned().unwrap();
            parent.module_settings.insert("world_name".into(), json!("myworld"));
            parent.module_settings.insert("cache_dir".into(), json!(""));
            parent.module_settings.insert("world_dir".into(), json!(old_home.join("Worlds").to_string_lossy()));
            parent.module_settings.insert("data_home".into(), json!(old_home.to_string_lossy()));
            sup.instance_store.update(&parent_id, parent).unwrap();
        }

        let branched = create(json!({ "name": "old b", "branch_of": parent_id })).await;
        assert!(branched.status.is_success(), "{:?}", branched.body);
        let branch_id = branched.body["id"].as_str().unwrap().to_string();
        let branch = state.supervisor.read().await.instance_store.get(&branch_id).cloned().unwrap();
        assert_eq!(branch.module_settings["world_name"], json!("myworld"), "the parent's setting");
        for key in ["cache_dir", "world_dir"] {
            assert!(branch.module_settings[key].as_str().unwrap().contains(&branch_id), "{key}: {:?}", branch.module_settings[key]);
        }
        assert_eq!(branch.module_settings["data_home"], json!(""), "the parent's old data folder is not the branch's");
    }

    /// 서버 이름의 쉼표는 받지 않는다: 문장 명령이 부르는 말을 쉼표로 나눈다 (사용자 지정 2026-11-06)
    #[tokio::test]
    async fn a_server_name_with_a_comma_is_refused() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };

        let check = crate::ipc::internal::into_internal(super::check_new_instance(State(state.clone()), Json(json!({ "name": "우리, 마크", "module_name": "game" }))).await).await;
        assert_eq!(check.body["name"]["invalid"], "comma", "{}", check.body);
        for name in ["우리, 마크", "우리，마크", "우리、마크"] {
            let refused = create(json!({ "name": name, "module_name": "game" })).await;
            assert_eq!((refused.status.as_u16(), refused.body["error_code"].as_str()), (400, Some("invalid_name")), "{name}");
        }
        assert!(state.supervisor.read().await.instance_store.list().is_empty());

        let made = create(json!({ "name": "우리 마크", "module_name": "game" })).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let renamed = crate::ipc::internal::into_internal(super::rename_instance(State(state.clone()), Path(id.clone()), Json(json!({ "name": "마크,서버" }))).await).await;
        assert_eq!((renamed.status.as_u16(), renamed.body["error_code"].as_str()), (400, Some("invalid_name")));
        assert_eq!(state.supervisor.read().await.instance_store.get(&id).unwrap().name, "우리 마크");
    }

    /// 대소문자·공백 칸 수만 다른 이름은 같은 이름이다: 채팅이 그렇게 서버를 찾으므로 둘이면 이름으로 부를 수 없다
    #[tokio::test]
    async fn a_name_that_differs_only_in_case_or_spacing_is_taken() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };
        let rename = |id: String, name: &'static str| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::rename_instance(State(state), Path(id), Json(json!({ "name": name }))).await).await }
        };

        let first = create(json!({ "name": "My Server", "module_name": "game" })).await;
        assert!(first.status.is_success(), "{:?}", first.body);
        let first = first.body["id"].as_str().unwrap().to_string();
        for name in ["my server", "MY  SERVER"] {
            let twin = create(json!({ "name": name, "module_name": "game" })).await;
            assert_eq!((twin.status.as_u16(), twin.body["error_code"].as_str()), (409, Some("name_taken")), "{name}");
        }
        let check = crate::ipc::internal::into_internal(super::check_new_instance(State(state.clone()), Json(json!({ "name": "my server", "module_name": "game" }))).await).await;
        assert_eq!(check.body["name"]["taken"], true, "{}", check.body);

        let other = create(json!({ "name": "other", "module_name": "game" })).await;
        let other = other.body["id"].as_str().unwrap().to_string();
        let twin = rename(other, "MY SERVER").await;
        assert_eq!((twin.status.as_u16(), twin.body["error_code"].as_str()), (409, Some("name_taken")), "{}", twin.body);
        // 자기 이름의 대소문자는 바꿀 수 있다
        let own = rename(first.clone(), "my server").await;
        assert!(own.status.is_success(), "{}", own.body);
        assert_eq!(state.supervisor.read().await.instance_store.get(&first).unwrap().name, "my server");
    }

    /// 앱 설정의 새 서버 기본값(자동 백업 보관 개수, 크래시 자동 재시작)을 만들 때 넣는다. 기본값이면 지금처럼 비워 둔다
    #[tokio::test]
    async fn create_applies_the_new_server_defaults_from_app_settings() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |name: &str| {
            let (state, body) = (state.clone(), json!({ "name": name, "module_name": "game" }));
            async move {
                let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(body)).await).await;
                assert!(made.status.is_success(), "{:?}", made.body);
                state.supervisor.read().await.instance_store.get(made.body["id"].as_str().unwrap()).cloned().unwrap()
            }
        };

        let plain = create("a").await;
        assert_eq!((plain.backup_keep, plain.module_settings.get("auto_restart")), (None, None));

        state.config_store
            .write_gui(json!({ "newServerBackupKeep": 3, "newServerAutoRestart": false }), crate::config_store::Apply::MergePatch, None)
            .await
            .unwrap();
        let tuned = create("b").await;
        assert_eq!((tuned.backup_keep, tuned.module_settings.get("auto_restart")), (Some(3), Some(&json!(false))));
    }

    #[tokio::test]
    async fn create_with_a_port_also_sets_the_module_port_setting() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let body = json!({ "name": "a", "module_name": "game", "port": 25570 });
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(body)).await).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let instance = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        // 설치는 module_settings로 게임 설정 파일을 쓴다: 두 값이 같아야 한다
        assert_eq!((instance.port, &instance.module_settings["port"]), (Some(25570), &json!(25570)));

        // 포트로 쓸 수 없는 값은 만들지 않고 알린다
        for port in [json!(70000), json!(0), json!("abc")] {
            let body = json!({ "name": "b", "module_name": "game", "port": port });
            let bad = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(body)).await).await;
            assert_eq!((bad.status.as_u16(), &bad.body["error_code"]), (400, &json!("invalid_format")), "{port}");
        }
    }
}
