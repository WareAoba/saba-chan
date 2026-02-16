use anyhow::Result;
use serde_json::Value;
use tokio::process::Command;
use tokio::io::AsyncWriteExt;
use crate::utils::apply_creation_flags;

/// Plugin runner executes Python modules (short-lived)
/// Returns JSON output from stdout only
/// Called by Supervisor for module lifecycle management
pub async fn run_plugin(module_path: &str, function: &str, config: Value) -> Result<Value> {
    tracing::info!("Executing plugin: {} -> {}", module_path, function);

    // Construct command: python module_path function
    // Config JSON is passed via stdin to avoid command-line length limits
    // and prevent sensitive data from appearing in process listings
    let config_json = serde_json::to_string(&config)?;
    
    // Try to find working Python command
    let python_cmd = detect_python_command().await.unwrap_or("python");
    
    tracing::info!("Using Python command: {}", python_cmd);
    
    let mut cmd = Command::new(python_cmd);
    cmd.arg(module_path)
        .arg(function)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    apply_creation_flags(&mut cmd);
    
    let mut child = cmd.spawn()?;
    
    // Write config JSON to stdin, then explicitly drop to signal EOF
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(config_json.as_bytes()).await?;
        stdin.shutdown().await?;
    }
    
    let output = child.wait_with_output().await?;

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
async fn detect_python_command() -> Option<&'static str> {
    // Try commands in order of preference
    let candidates = vec!["python", "python3", "py"];
    
    for cmd_name in candidates {
        let mut cmd = Command::new(cmd_name);
        cmd.arg("--version");
        apply_creation_flags(&mut cmd);
        if let Ok(output) = cmd.output().await {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout);
                tracing::debug!("Found Python: {} -> {}", cmd_name, version.trim());
                return Some(cmd_name);
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
    async fn test_detect_python_command() {
        // Python 명령어 탐지
        let result = detect_python_command().await;
        
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
}
