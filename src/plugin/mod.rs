use anyhow::Result;
use std::process::Command;
use serde_json::Value;

/// Plugin manager for executing Python modules
#[allow(dead_code)]
pub struct PluginManager {
    python_cmd: Option<String>,
}

impl Default for PluginManager {
    fn default() -> Self {
        Self {
            python_cmd: detect_python_command().map(|s| s.to_string()),
        }
    }
}

#[allow(dead_code)]
impl PluginManager {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn detect_python(&self) -> Option<String> {
        self.python_cmd.clone()
    }
    
    pub async fn run_plugin(&self, module_path: &str, function: &str, config: Value) -> Result<Value> {
        run_plugin(module_path, function, config).await
    }
}

/// Plugin runner executes Python modules (short-lived)
/// Returns JSON output from stdout only
/// Called by Supervisor for module lifecycle management
pub async fn run_plugin(module_path: &str, function: &str, config: Value) -> Result<Value> {
    tracing::info!("Executing plugin: {} -> {}", module_path, function);

    // Construct command: python module_path function config_json
    let config_json = serde_json::to_string(&config)?;
    
    // Try to find working Python command
    let python_cmd = detect_python_command().unwrap_or("python");
    
    tracing::info!("Using Python command: {}", python_cmd);
    
    let output = Command::new(python_cmd)
        .arg(module_path)
        .arg(function)
        .arg(&config_json)
        .output()?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Log stderr for debugging (always show for troubleshooting)
    if !stderr.is_empty() {
        tracing::info!("Plugin stderr: {}", stderr);
    }

    if !output.status.success() {
        tracing::error!("Plugin failed with exit code: {:?}", output.status.code());
        tracing::error!("Plugin stderr: {}", stderr);
        tracing::error!("Plugin stdout: {}", stdout);
        return Err(anyhow::anyhow!("Plugin execution failed: {}", stderr));
    }

    // Try to parse JSON from stdout
    match serde_json::from_str::<Value>(&stdout) {
        Ok(result) => {
            tracing::info!("Plugin result: {:?}", result);
            Ok(result)
        }
        Err(e) => {
            tracing::error!("Failed to parse plugin output as JSON: {}", e);
            tracing::error!("Raw stdout: {}", stdout);
            tracing::error!("Raw stderr: {}", stderr);
            Err(anyhow::anyhow!("Invalid JSON from plugin: {}\nOutput: {}", e, stdout))
        }
    }
}

/// Detect available Python command
fn detect_python_command() -> Option<&'static str> {
    // Try commands in order of preference
    let candidates = vec!["python", "python3", "py"];
    
    for cmd in candidates {
        if let Ok(output) = Command::new(cmd).arg("--version").output() {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout);
                tracing::debug!("Found Python: {} -> {}", cmd, version.trim());
                return Some(cmd);
            }
        }
    }
    
    tracing::warn!("No Python command found, defaulting to 'python'");
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_detect_python_command() {
        // Python 명령어 탐지
        let result = detect_python_command();
        
        // 결과가 Some이거나 None일 수 있음 (환경에 따라)
        match result {
            Some(cmd) => println!("Detected Python: {}", cmd),
            None => println!("No Python found, will use default 'python'"),
        }
    }

    #[tokio::test]
    async fn test_run_plugin_with_invalid_path() {
        let result = run_plugin(
            "nonexistent/module.py",
            "test_function",
            json!({"test": "data"})
        ).await;

        // 존재하지 않는 파일이므로 에러가 발생해야 함
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_plugin_runner_stub() {
        // Stub test: actual execution depends on Python environment
        let config = json!({"ram": "8G"});
        let _result = run_plugin("stub.py", "start", config);
        // Would fail without actual Python module, which is expected
    }
}
