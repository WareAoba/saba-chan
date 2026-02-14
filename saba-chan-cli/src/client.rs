use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DaemonClient {
    client: reqwest::Client,
    /// 장시간 작업용 (install, managed start 등)
    long_client: reqwest::Client,
    base_url: String,
}

#[allow(dead_code)]
impl DaemonClient {
    pub fn new(base_url: Option<&str>) -> Self {
        let base_url = base_url
            .unwrap_or("http://127.0.0.1:57474")
            .to_string();

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("Failed to create HTTP client");

        let long_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to create long-timeout HTTP client");

        Self { client, long_client, base_url }
    }

    // ─── 내부 헬퍼 ───

    async fn get_json(&self, path: &str) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.get(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }
        Ok(response.json().await?)
    }

    async fn post_json(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.post(&url).json(body).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }
        Ok(response.json().await?)
    }

    async fn post_empty(&self, path: &str) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.post(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }
        Ok(response.json().await?)
    }

    async fn post_json_long(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.long_client.post(&url).json(body).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }
        Ok(response.json().await?)
    }

    async fn delete_json(&self, path: &str) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.delete(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }
        Ok(response.json().await?)
    }

    async fn patch_json(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.patch(&url).json(body).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }
        Ok(response.json().await?)
    }

    async fn put_json(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.put(&url).json(body).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }
        Ok(response.json().await?)
    }

    // ============ Servers (런타임) ============

    /// GET /api/servers — 서버 런타임 상태 (status, pid 포함, {"servers": [...]})
    pub async fn list_servers(&self) -> anyhow::Result<Vec<Value>> {
        let data = self.get_json("/api/servers").await?;
        Ok(data
            .get("servers")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// GET /api/server/{name}/status
    pub async fn get_server_status(&self, name: &str) -> anyhow::Result<Value> {
        self.get_json(&format!("/api/server/{}/status", name)).await
    }

    /// POST /api/server/{name}/start
    pub async fn start_server(&self, name: &str, module: &str) -> anyhow::Result<Value> {
        let body = serde_json::json!({ "module": module, "config": {} });
        self.post_json(&format!("/api/server/{}/start", name), &body).await
    }

    /// POST /api/server/{name}/stop
    pub async fn stop_server(&self, name: &str, force: bool) -> anyhow::Result<Value> {
        let body = serde_json::json!({ "force": force });
        self.post_json(&format!("/api/server/{}/stop", name), &body).await
    }

    // ============ Instances (설정) ============

    /// GET /api/instances — 인스턴스 설정 목록 (베어 배열)
    pub async fn list_instances(&self) -> anyhow::Result<Vec<Value>> {
        let data = self.get_json("/api/instances").await?;
        Ok(data.as_array().cloned().unwrap_or_default())
    }

    /// GET /api/instance/{id}
    pub async fn get_instance(&self, id: &str) -> anyhow::Result<Value> {
        self.get_json(&format!("/api/instance/{}", id)).await
    }

    /// POST /api/instances — 인스턴스 생성
    pub async fn create_instance(&self, data: Value) -> anyhow::Result<Value> {
        self.post_json("/api/instances", &data).await
    }

    /// DELETE /api/instance/{id}
    pub async fn delete_instance(&self, id: &str) -> anyhow::Result<Value> {
        self.delete_json(&format!("/api/instance/{}", id)).await
    }

    /// PATCH /api/instance/{id} — 인스턴스 설정 업데이트
    pub async fn update_instance(&self, id: &str, settings: Value) -> anyhow::Result<Value> {
        self.patch_json(&format!("/api/instance/{}", id), &settings).await
    }

    /// PUT /api/instances/reorder — 인스턴스 순서 변경
    pub async fn reorder_instances(&self, order: Value) -> anyhow::Result<Value> {
        self.put_json("/api/instances/reorder", &order).await
    }

    // ============ Commands ============

    /// POST /api/instance/{id}/command
    pub async fn execute_command(&self, id: &str, command: &str, args: Option<Value>) -> anyhow::Result<Value> {
        let body = serde_json::json!({
            "command": command,
            "args": args.unwrap_or(Value::Null)
        });
        self.post_json(&format!("/api/instance/{}/command", id), &body).await
    }

    /// POST /api/instance/{id}/rcon
    pub async fn execute_rcon_command(&self, id: &str, command: &str) -> anyhow::Result<Value> {
        let body = serde_json::json!({ "command": command });
        self.post_json(&format!("/api/instance/{}/rcon", id), &body).await
    }

    /// POST /api/instance/{id}/rest
    pub async fn execute_rest_command(&self, id: &str, command: &str) -> anyhow::Result<Value> {
        let body = serde_json::json!({ "command": command });
        self.post_json(&format!("/api/instance/{}/rest", id), &body).await
    }

    // ============ Managed Process ============

    /// POST /api/instance/{id}/managed/start — 관리형 서버 시작
    pub async fn start_managed(&self, id: &str) -> anyhow::Result<Value> {
        self.post_json_long(
            &format!("/api/instance/{}/managed/start", id),
            &serde_json::json!({}),
        ).await
    }

    /// GET /api/instance/{id}/console — 콘솔 출력 가져오기
    pub async fn get_console(&self, id: &str) -> anyhow::Result<Value> {
        self.get_json(&format!("/api/instance/{}/console", id)).await
    }

    /// POST /api/instance/{id}/stdin — stdin으로 텍스트 전송
    pub async fn send_stdin(&self, id: &str, input: &str) -> anyhow::Result<Value> {
        let body = serde_json::json!({ "input": input });
        self.post_json(&format!("/api/instance/{}/stdin", id), &body).await
    }

    // ============ Instance Utilities ============

    /// POST /api/instance/{id}/validate — 인스턴스 설정 검증
    pub async fn validate_instance(&self, id: &str) -> anyhow::Result<Value> {
        self.post_empty(&format!("/api/instance/{}/validate", id)).await
    }

    /// GET /api/instance/{id}/properties — 서버 속성 파일 읽기
    pub async fn read_properties(&self, id: &str) -> anyhow::Result<Value> {
        self.get_json(&format!("/api/instance/{}/properties", id)).await
    }

    /// PUT /api/instance/{id}/properties — 서버 속성 파일 쓰기
    pub async fn write_properties(&self, id: &str, data: Value) -> anyhow::Result<Value> {
        self.put_json(&format!("/api/instance/{}/properties", id), &data).await
    }

    /// POST /api/instance/{id}/accept-eula — EULA 수락
    pub async fn accept_eula(&self, id: &str) -> anyhow::Result<Value> {
        self.post_empty(&format!("/api/instance/{}/accept-eula", id)).await
    }

    /// POST /api/instance/{id}/diagnose — 서버 진단
    pub async fn diagnose(&self, id: &str) -> anyhow::Result<Value> {
        self.post_empty(&format!("/api/instance/{}/diagnose", id)).await
    }

    // ============ Modules ============

    /// GET /api/modules — 로드된 모듈 목록
    pub async fn list_modules(&self) -> anyhow::Result<Vec<Value>> {
        let data = self.get_json("/api/modules").await?;
        Ok(data
            .get("modules")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// GET /api/module/{name} — 모듈 상세 정보
    pub async fn get_module(&self, name: &str) -> anyhow::Result<Value> {
        self.get_json(&format!("/api/module/{}", name)).await
    }

    /// POST /api/modules/refresh — 모듈 새로고침
    pub async fn refresh_modules(&self) -> anyhow::Result<Value> {
        self.post_empty("/api/modules/refresh").await
    }

    /// GET /api/module/{name}/versions — 사용 가능한 버전 목록
    pub async fn list_versions(&self, module: &str) -> anyhow::Result<Value> {
        self.get_json(&format!("/api/module/{}/versions", module)).await
    }

    /// GET /api/module/{name}/version/{version} — 버전 상세 정보
    pub async fn get_version_details(&self, module: &str, version: &str) -> anyhow::Result<Value> {
        self.get_json(&format!("/api/module/{}/version/{}", module, version)).await
    }

    /// POST /api/module/{name}/install — 서버 설치
    pub async fn install_server(&self, module: &str, data: Value) -> anyhow::Result<Value> {
        self.post_json_long(&format!("/api/module/{}/install", module), &data).await
    }

    // ============ Bot Config ============

    /// GET /api/config/bot — 봇 설정 가져오기
    pub async fn get_bot_config(&self) -> anyhow::Result<Value> {
        self.get_json("/api/config/bot").await
    }

    /// PUT /api/config/bot — 봇 설정 저장
    pub async fn save_bot_config(&self, config: Value) -> anyhow::Result<Value> {
        self.put_json("/api/config/bot", &config).await
    }

    // ============ Client Heartbeat ============

    /// POST /api/client/register — 데몬에 클라이언트 등록
    pub async fn register_client(&self, kind: &str) -> anyhow::Result<String> {
        let body = serde_json::json!({ "kind": kind });
        let data = self.post_json("/api/client/register", &body).await?;
        data.get("client_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow::anyhow!("No client_id in response"))
    }

    /// POST /api/client/{id}/heartbeat — 생존 신호 전송
    pub async fn send_heartbeat(&self, client_id: &str, bot_pid: Option<u32>) -> anyhow::Result<()> {
        let body = match bot_pid {
            Some(pid) => serde_json::json!({ "bot_pid": pid }),
            None => serde_json::json!({}),
        };
        self.post_json(&format!("/api/client/{}/heartbeat", client_id), &body).await?;
        Ok(())
    }

    /// DELETE /api/client/{id}/unregister — 클라이언트 해제
    pub async fn unregister_client(&self, client_id: &str) -> anyhow::Result<()> {
        self.delete_json(&format!("/api/client/{}/unregister", client_id)).await?;
        Ok(())
    }

    // ============ Updates ============

    /// POST /api/updates/check — 업데이트 수동 확인
    pub async fn check_updates(&self) -> anyhow::Result<Value> {
        self.post_empty("/api/updates/check").await
    }

    /// GET /api/updates/status — 업데이트 상태 조회
    pub async fn get_update_status(&self) -> anyhow::Result<Value> {
        self.get_json("/api/updates/status").await
    }

    /// POST /api/updates/download — 업데이트 전체 다운로드
    pub async fn download_updates(&self) -> anyhow::Result<Value> {
        self.post_empty("/api/updates/download").await
    }

    /// POST /api/updates/apply — 업데이트 적용
    pub async fn apply_updates(&self) -> anyhow::Result<Value> {
        self.post_empty("/api/updates/apply").await
    }

    /// GET /api/updates/config — 업데이트 설정 조회
    pub async fn get_update_config(&self) -> anyhow::Result<Value> {
        self.get_json("/api/updates/config").await
    }

    // ============ Installer ============

    /// GET /api/install/status — 설치 상태 확인
    pub async fn get_install_status(&self) -> anyhow::Result<Value> {
        self.get_json("/api/install/status").await
    }

    /// POST /api/install/run — 최초 설치 실행
    pub async fn run_install(&self, components: Option<Vec<String>>) -> anyhow::Result<Value> {
        let body = match components {
            Some(comps) => serde_json::json!({ "components": comps }),
            None => serde_json::json!({}),
        };
        self.post_json_long("/api/install/run", &body).await
    }

    /// POST /api/install/component/{key} — 특정 컴포넌트 설치
    pub async fn install_component(&self, key: &str) -> anyhow::Result<Value> {
        self.post_json_long(
            &format!("/api/install/component/{}", key),
            &serde_json::json!({}),
        ).await
    }

    /// GET /api/install/progress — 설치 진행 상태 조회
    pub async fn get_install_progress(&self) -> anyhow::Result<Value> {
        self.get_json("/api/install/progress").await
    }
}
