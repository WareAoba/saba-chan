use anyhow::Result;
use std::process::Command;
use serde_json::Value;

/// Plugin runner executes Python modules (short-lived)
/// Returns JSON output from stdout only
#[allow(dead_code)]
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
    
    // Log stderr for debugging (not necessarily an error)
    if !stderr.is_empty() {
        tracing::debug!("Plugin stderr: {}", stderr);
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

    #[tokio::test]
    async fn test_plugin_runner_stub() {
        // Stub test: actual execution depends on Python environment
        let config = json!({"ram": "8G"});
        let _result = run_plugin("stub.py", "start", config);
        // Would fail without actual Python module, which is expected
    }
}
