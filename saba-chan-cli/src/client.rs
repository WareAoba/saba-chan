use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DaemonClient {
    client: reqwest::Client,
    base_url: String,
}

#[allow(dead_code)]
impl DaemonClient {
    pub fn new(base_url: Option<&str>) -> Self {
        let base_url = base_url
            .unwrap_or("http://127.0.0.1:57474")
            .to_string();

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("Failed to create HTTP client");

        Self { client, base_url }
    }

    // ============ Instances (Servers) ============

    /// GET /api/instances — 인스턴스 설정 목록 (status 없음, 베어 배열)
    pub async fn list_instances(&self) -> anyhow::Result<Vec<Value>> {
        let url = format!("{}/api/instances", self.base_url);
        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        let data: Value = response.json().await?;
        
        // /api/instances returns a bare JSON array
        Ok(data.as_array().cloned().unwrap_or_default())
    }

    /// GET /api/servers — 서버 런타임 상태 (status, pid 포함, {"servers": [...]})
    pub async fn list_servers(&self) -> anyhow::Result<Vec<Value>> {
        let url = format!("{}/api/servers", self.base_url);
        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        let data: Value = response.json().await?;
        
        Ok(data
            .get("servers")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// GET /api/server/{name}/status
    pub async fn get_server_status(&self, name: &str) -> anyhow::Result<Value> {
        let url = format!("{}/api/server/{}/status", self.base_url, name);
        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    /// POST /api/server/{name}/start
    pub async fn start_server(&self, name: &str, module: &str) -> anyhow::Result<Value> {
        let url = format!("{}/api/server/{}/start", self.base_url, name);
        let body = serde_json::json!({ "module": module, "config": {} });
        let response = self.client.post(&url).json(&body).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    /// POST /api/server/{name}/stop
    pub async fn stop_server(&self, name: &str, force: bool) -> anyhow::Result<Value> {
        let url = format!("{}/api/server/{}/stop", self.base_url, name);
        let body = serde_json::json!({ "force": force });
        let response = self.client.post(&url).json(&body).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn get_instance(&self, id: &str) -> anyhow::Result<Value> {
        let url = format!("{}/api/instance/{}", self.base_url, id);
        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn start_instance(&self, id: &str) -> anyhow::Result<Value> {
        let url = format!("{}/api/instance/{}/start", self.base_url, id);
        let response = self.client.post(&url).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn stop_instance(&self, id: &str, force: bool) -> anyhow::Result<Value> {
        let url = format!("{}/api/instance/{}/stop", self.base_url, id);
        let body = serde_json::json!({ "force": force });
        let response = self.client.post(&url).json(&body).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn execute_command(&self, id: &str, command: &str, args: Option<Value>) -> anyhow::Result<Value> {
        let url = format!("{}/api/instance/{}/command", self.base_url, id);
        let body = serde_json::json!({
            "command": command,
            "args": args.unwrap_or(Value::Null)
        });
        let response = self.client.post(&url).json(&body).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn execute_rcon_command(&self, id: &str, command: &str) -> anyhow::Result<Value> {
        let url = format!("{}/api/instance/{}/rcon", self.base_url, id);
        let body = serde_json::json!({ "command": command });
        let response = self.client.post(&url).json(&body).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn execute_rest_command(&self, id: &str, command: &str) -> anyhow::Result<Value> {
        let url = format!("{}/api/instance/{}/rest", self.base_url, id);
        let body = serde_json::json!({ "command": command });
        let response = self.client.post(&url).json(&body).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn create_instance(&self, data: Value) -> anyhow::Result<Value> {
        let url = format!("{}/api/instance", self.base_url);
        let response = self.client.post(&url).json(&data).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn delete_instance(&self, id: &str) -> anyhow::Result<Value> {
        let url = format!("{}/api/instance/{}", self.base_url, id);
        let response = self.client.delete(&url).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn update_instance_settings(&self, id: &str, settings: Value) -> anyhow::Result<Value> {
        let url = format!("{}/api/instance/{}/settings", self.base_url, id);
        let response = self.client.patch(&url).json(&settings).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    // ============ Modules ============

    pub async fn list_modules(&self) -> anyhow::Result<Vec<Value>> {
        let url = format!("{}/api/modules", self.base_url);
        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        let data: Value = response.json().await?;
        
        Ok(data
            .get("modules")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub async fn get_module(&self, name: &str) -> anyhow::Result<Value> {
        let url = format!("{}/api/module/{}", self.base_url, name);
        let response = self.client.get(&url).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }

    pub async fn reload_modules(&self) -> anyhow::Result<Value> {
        let url = format!("{}/api/modules/reload", self.base_url);
        let response = self.client.post(&url).send().await?;
        
        if !response.status().is_success() {
            anyhow::bail!("Server returned {}: {}", response.status(), response.text().await?);
        }

        Ok(response.json().await?)
    }
}
