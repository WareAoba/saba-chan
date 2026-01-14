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
    
    let output = Command::new("python")
        .arg(module_path)
        .arg(function)
        .arg(&config_json)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("Plugin stderr: {}", stderr);
        return Err(anyhow::anyhow!("Plugin execution failed: {}", stderr));
    }

    let stdout = String::from_utf8(output.stdout)?;
    let result = serde_json::from_str::<Value>(&stdout)?;
    
    tracing::info!("Plugin result: {:?}", result);
    Ok(result)
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
