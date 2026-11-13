//! 인스턴스 설정: 게임 설정 파일과 저장된 설정을 맞춘다

use crate::ipc::IPCServer;
use crate::supervisor::Supervisor;

/// 게임 설정 파일을 모듈 `import_settings`로 읽어 `module_settings`와 인스턴스 필드(포트·비밀번호)를 맞춘다.
/// 호출자가 설정 파일 잠금(`InstanceLock::files`나 `take_for_writes`)을 쥐고 있어야 한다. 모듈의 답을 그대로 돌려준다. `success`가 아니거나 설정이 비어 있으면 아무것도 바꾸지 않는다
pub(crate) async fn refresh_from_files(state: &IPCServer, id: &str) -> anyhow::Result<serde_json::Value> {
    let result = Supervisor::manage_properties(&state.supervisor, id, "import", None).await?;
    let ok = result.get("success").and_then(|v| v.as_bool()) == Some(true);
    let settings = result.get("settings").and_then(|v| v.as_object()).filter(|s| !s.is_empty());
    if let (true, Some(settings)) = (ok, settings) {
        let mut supervisor = state.supervisor.write().await;
        if let Some(mut inst) = supervisor.instance_store.get(id).cloned() {
            let types = supervisor.module_loader.get_module(&inst.module_name).map(|m| m.metadata.setting_types()).unwrap_or_default();
            // 게임 포트 같은 인스턴스 필드도 파일 값으로 맞춘다 (GUI는 그쪽을 먼저 읽는다)
            inst.apply_imported_settings(settings, &types);
            if let Err(e) = supervisor.instance_store.update(id, inst) {
                tracing::warn!("Settings refresh from game files failed for {}: {}", id, e);
            }
        }
    }
    Ok(result)
}
