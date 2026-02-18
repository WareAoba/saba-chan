//! Docker Compose integration for containerized game server management.
//!
//! When an instance directory contains a `docker-compose.yml`, the supervisor
//! can delegate lifecycle operations (start / stop / status / logs) to
//! `docker compose` instead of running the game server process natively.


use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Whether the Docker daemon is running inside WSL2 (set after ensure_docker_engine).
static WSL2_MODE: AtomicBool = AtomicBool::new(false);

/// WSL2 Docker binary path inside the WSL2 distro.
const WSL2_DOCKER_DIR: &str = "/opt/saba-chan/docker";

pub fn set_wsl2_mode(enabled: bool) {
    WSL2_MODE.store(enabled, Ordering::Relaxed);
    if enabled {
        tracing::info!("Docker WSL2 mode enabled -- all docker commands will go through WSL");
    }
}

pub fn is_wsl2_mode() -> bool {
    WSL2_MODE.load(Ordering::Relaxed)
}

/// Docker Compose configuration (optional in module.toml `[docker]` section)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerComposeConfig {
    /// Whether Docker mode is enabled for this instance
    #[serde(default)]
    pub enabled: bool,
    /// Custom compose file name (default: "docker-compose.yml")
    #[serde(default = "default_compose_file")]
    pub compose_file: String,
    /// Service name to target within the compose file (optional)
    #[serde(default)]
    pub service_name: Option<String>,
    /// Additional environment variables to pass
    #[serde(default)]
    pub environment: std::collections::HashMap<String, String>,
}

fn default_compose_file() -> String {
    "docker-compose.yml".to_string()
}

impl Default for DockerComposeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            compose_file: default_compose_file(),
            service_name: None,
            environment: std::collections::HashMap::new(),
        }
    }
}

/// Manages Docker Compose operations for an instance.
pub struct DockerComposeManager {
    /// The directory containing docker-compose.yml (instance directory)
    working_dir: PathBuf,
    /// Compose file name
    compose_file: String,
    /// Optional service name
    service_name: Option<String>,
}

impl DockerComposeManager {
    /// Create a new DockerComposeManager for an instance directory.
    pub fn new(instance_dir: &Path, config: Option<&DockerComposeConfig>) -> Self {
        let (compose_file, service_name) = if let Some(cfg) = config {
            (cfg.compose_file.clone(), cfg.service_name.clone())
        } else {
            (default_compose_file(), None)
        };

        Self {
            working_dir: instance_dir.to_path_buf(),
            compose_file,
            service_name,
        }
    }

    /// Check if a docker-compose.yml exists in the instance directory
    pub fn has_compose_file(&self) -> bool {
        self.working_dir.join(&self.compose_file).exists()
    }

    /// Get the path to the compose file
    pub fn compose_file_path(&self) -> PathBuf {
        self.working_dir.join(&self.compose_file)
    }

    /// Detect the Docker Compose CLI command.
    /// In WSL2 mode, routes through `wsl -u root docker compose`.
    fn compose_command(&self) -> (PathBuf, Vec<String>) {
        if is_wsl2_mode() {
            return (
                PathBuf::from("wsl"),
                vec![
                    "-u".into(), "root".into(), "--".into(),
                    format!("{}/docker", WSL2_DOCKER_DIR),
                    "compose".into(),
                ],
            );
        }
        // Local portable docker-compose binary
        let local_compose = local_compose_exe();
        if local_compose.exists() {
            return (local_compose, vec![]);
        }
        // System: `docker compose` (V2 plugin)
        (docker_cli_path(), vec!["compose".to_string()])
    }

    /// Build the base command with working directory and compose file
    fn build_command(&self) -> tokio::process::Command {
        let (program, base_args) = self.compose_command();
        let mut cmd = tokio::process::Command::new(program);
        for arg in &base_args {
            cmd.arg(arg);
        }
        // In WSL2 mode use relative compose filename (CWD is auto-translated)
        if is_wsl2_mode() {
            cmd.arg("-f").arg(&self.compose_file);
        } else {
            cmd.arg("-f").arg(self.compose_file_path());
        }
        cmd.current_dir(&self.working_dir);
        crate::utils::apply_creation_flags(&mut cmd);
        cmd
    }

    /// Start containers: `docker compose up -d`
    /// If the container already exists (stopped), this will restart it quickly.
    /// If the container doesn't exist, this creates and starts it.
    pub async fn start(&self) -> Result<Value> {
        if !self.has_compose_file() {
            return Err(anyhow::anyhow!(
                "No {} found in {}",
                self.compose_file,
                self.working_dir.display()
            ));
        }

        let mut cmd = self.build_command();
        cmd.arg("up").arg("-d");

        if let Some(ref service) = self.service_name {
            cmd.arg(service);
        }

        tracing::info!("Docker Compose up: {}", self.working_dir.display());
        let output = cmd.output().await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(json!({
                "success": true,
                "message": "Docker Compose containers started",
                "stdout": stdout,
            }))
        } else {
            Err(anyhow::anyhow!(
                "Docker Compose up failed: {}",
                if stderr.is_empty() { &stdout } else { &stderr }
            ))
        }
    }

    /// Stop containers: `docker compose stop` (keeps containers, fast restart)
    /// Use `down()` to remove containers entirely.
    pub async fn stop(&self) -> Result<Value> {
        let mut cmd = self.build_command();
        cmd.arg("stop");

        if let Some(ref service) = self.service_name {
            cmd.arg(service);
        }

        tracing::info!("Docker Compose stop: {}", self.working_dir.display());
        let output = cmd.output().await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(json!({
                "success": true,
                "message": "Docker Compose containers stopped",
                "stdout": stdout,
            }))
        } else {
            Err(anyhow::anyhow!(
                "Docker Compose stop failed: {}",
                if stderr.is_empty() { &stdout } else { &stderr }
            ))
        }
    }

    /// Remove containers and networks: `docker compose down`
    /// Called on saba-chan shutdown or when deleting an instance.
    pub async fn down(&self) -> Result<Value> {
        let mut cmd = self.build_command();
        cmd.arg("down");

        tracing::info!("Docker Compose down: {}", self.working_dir.display());
        let output = cmd.output().await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(json!({
                "success": true,
                "message": "Docker Compose containers removed",
                "stdout": stdout,
            }))
        } else {
            // down 실패는 치명적이지 않음 (이미 삭제된 경우 등)
            tracing::warn!("Docker Compose down warning: {}", if stderr.is_empty() { &stdout } else { &stderr });
            Ok(json!({
                "success": true,
                "message": "Docker Compose down completed with warnings",
                "stderr": stderr,
            }))
        }
    }

    /// Get container status: `docker compose ps --format json`
    pub async fn status(&self) -> Result<Value> {
        let mut cmd = self.build_command();
        cmd.arg("ps").arg("--format").arg("json").arg("-a");

        let output = cmd.output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        if output.status.success() {
            // docker compose ps can return one JSON object per line (not a JSON array)
            let containers: Value = serde_json::from_str(&stdout)
                .unwrap_or_else(|_| {
                    // Try parsing as JSON-lines (one object per line)
                    let arr: Vec<Value> = stdout.lines()
                        .filter_map(|line| serde_json::from_str(line).ok())
                        .collect();
                    if arr.is_empty() { json!({"raw": stdout}) } else { Value::Array(arr) }
                });

            let container_running = match &containers {
                Value::Array(arr) => arr.iter().any(|c| {
                    c.get("State").and_then(|s| s.as_str()) == Some("running")
                }),
                Value::Object(obj) => obj.get("State").and_then(|s| s.as_str()) == Some("running"),
                _ => stdout.contains("running"),
            };

            // Extract container name for process checking
            let container_name = match &containers {
                Value::Array(arr) => arr.first()
                    .and_then(|c| c.get("Name").or_else(|| c.get("Names")))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string()),
                Value::Object(obj) => obj.get("Name").or_else(|| obj.get("Names"))
                    .and_then(|n| n.as_str())
                    .map(|s| s.to_string()),
                _ => None,
            };

            Ok(json!({
                "success": true,
                "running": container_running,
                "container_name": container_name,
                "containers": containers,
            }))
        } else {
            Ok(json!({
                "success": true,
                "running": false,
                "containers": [],
            }))
        }
    }

    /// Check if the game server process is running inside the container.
    /// Uses `docker top <container>` and matches against known process patterns.
    /// Returns (server_running, matched_process_name).
    pub async fn server_process_running(
        &self,
        container_name: &str,
        process_patterns: &[String],
    ) -> (bool, Option<String>) {
        if process_patterns.is_empty() {
            // No patterns to match — fall back to container-level status
            return (true, None);
        }

        let docker_top = self.docker_top_command(container_name);
        let output = match docker_top.await {
            Ok(out) => out,
            Err(e) => {
                tracing::debug!("docker top failed for {}: {}", container_name, e);
                return (false, None);
            }
        };

        // Parse docker top output — each line after header is a process
        for line in output.lines().skip(1) {
            let line_lower = line.to_lowercase();
            for pattern in process_patterns {
                if line_lower.contains(&pattern.to_lowercase()) {
                    return (true, Some(pattern.clone()));
                }
            }
        }

        (false, None)
    }

    /// Run `docker top <container>` to get process list inside a container.
    async fn docker_top_command(&self, container_name: &str) -> Result<String> {
        let output = if is_wsl2_mode() {
            tokio::process::Command::new("wsl")
                .args(["-u", "root", "--",
                       &format!("{}/docker", WSL2_DOCKER_DIR),
                       "top", container_name])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await?
        } else {
            let mut cmd = tokio::process::Command::new(docker_cli_path());
            cmd.args(["top", container_name]);
            crate::utils::apply_creation_flags(&mut cmd);
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await?
        };

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(anyhow::anyhow!("docker top failed: {}", stderr))
        }
    }

    /// Get logs: `docker compose logs --tail N`
    pub async fn logs(&self, tail: Option<u32>, follow: bool) -> Result<Value> {
        let mut cmd = self.build_command();
        cmd.arg("logs");

        if let Some(n) = tail {
            cmd.arg("--tail").arg(n.to_string());
        }
        if follow {
            cmd.arg("--follow");
        }

        if let Some(ref service) = self.service_name {
            cmd.arg(service);
        }

        let output = cmd.output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(json!({
            "success": true,
            "logs": stdout,
        }))
    }

    /// Restart containers: `docker compose restart`
    pub async fn restart(&self) -> Result<Value> {
        let mut cmd = self.build_command();
        cmd.arg("restart");

        if let Some(ref service) = self.service_name {
            cmd.arg(service);
        }

        let output = cmd.output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            Ok(json!({
                "success": true,
                "message": "Docker Compose containers restarted",
            }))
        } else {
            Err(anyhow::anyhow!(
                "Docker Compose restart failed: {}",
                if stderr.is_empty() { &stdout } else { &stderr }
            ))
        }
    }
}

/// Get resource usage stats for a Docker container.
/// Uses `docker stats --no-stream --format json <container_name>`.
/// Returns parsed JSON with MemUsage, MemPerc, CPUPerc etc.
pub async fn docker_container_stats(container_name: &str) -> Result<Value> {
    let output = if is_wsl2_mode() {
        tokio::process::Command::new("wsl")
            .args(["-u", "root", "--",
                   &format!("{}/docker", WSL2_DOCKER_DIR),
                   "stats", "--no-stream", "--format", "{{json .}}", container_name])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await?
    } else {
        let mut cmd = tokio::process::Command::new(docker_cli_path());
        cmd.args(["stats", "--no-stream", "--format", "{{json .}}", container_name]);
        crate::utils::apply_creation_flags(&mut cmd);
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await?
    };

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stats: Value = serde_json::from_str(stdout.trim())
            .unwrap_or_else(|_| json!({"raw": stdout}));
        Ok(stats)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(anyhow::anyhow!("docker stats failed: {}", stderr))
    }
}

/// Detect if Docker is available on the system.
/// In WSL2 mode, checks the Docker binary inside WSL2.
pub fn is_docker_available() -> bool {
    if is_wsl2_mode() {
        return std::process::Command::new("wsl")
            .args(["-u", "root", "--", &format!("{}/docker", WSL2_DOCKER_DIR), "--version"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    std::process::Command::new("docker")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if the Docker daemon is actually responding (not just CLI installed).
/// In WSL2 mode, checks the daemon inside WSL2.
pub fn is_docker_daemon_running() -> bool {
    if is_wsl2_mode() {
        return std::process::Command::new("wsl")
            .args(["-u", "root", "--",
                   &format!("{}/docker", WSL2_DOCKER_DIR),
                   "-H", "unix:///var/run/docker.sock", "info"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Detailed Docker status for the GUI
pub fn docker_status_detail() -> Value {
    let cli_available = is_docker_available();
    let daemon_running = if cli_available { is_docker_daemon_running() } else { false };

    json!({
        "cli_installed": cli_available,
        "daemon_running": daemon_running,
        "ready": cli_available && daemon_running,
        "wsl2_mode": is_wsl2_mode(),
    })
}

// ─── Docker Auto-Provisioner (via Python Extension) ────────────────

/// Result of a Docker provisioning/install attempt
#[derive(Debug, Clone, Serialize)]
pub struct DockerInstallResult {
    pub success: bool,
    pub message: String,
    /// Whether the Docker daemon is up and ready
    pub daemon_ready: bool,
}

/// Get the local Docker Engine directory (next to our executable, under `docker/`).
pub fn local_docker_dir() -> PathBuf {
    let mut dir = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("."))
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    dir.push("docker");
    dir
}

fn local_compose_exe() -> PathBuf {
    #[cfg(target_os = "windows")]
    { local_docker_dir().join("docker-compose.exe") }
    #[cfg(not(target_os = "windows"))]
    { local_docker_dir().join("docker-compose") }
}

/// Return the Docker CLI executable path to use —
/// prefer system-installed, fallback to local portable.
pub fn docker_cli_path() -> PathBuf {
    if is_docker_available() {
        return PathBuf::from("docker");
    }
    let local = local_docker_dir().join(
        if cfg!(target_os = "windows") { "docker.exe" } else { "docker" }
    );
    if local.exists() {
        return local;
    }
    local
}

/// Ensure Docker Engine is available: delegates to the Python ``docker_engine``
/// extension which downloads portable binaries and starts a local daemon.
pub async fn ensure_docker_engine() -> DockerInstallResult {
    ensure_docker_engine_inner(None).await
}

/// Like `ensure_docker_engine`, but with a real-time progress callback from
/// the Python extension (used to report download percentage).
pub async fn ensure_docker_engine_with_progress<F>(on_progress: F) -> DockerInstallResult
where
    F: Fn(crate::plugin::ExtensionProgress) + Send + 'static,
{
    ensure_docker_engine_inner(Some(Box::new(on_progress))).await
}

async fn ensure_docker_engine_inner(
    on_progress: Option<Box<dyn Fn(crate::plugin::ExtensionProgress) + Send>>,
) -> DockerInstallResult {
    // If system or portable docker is already running, skip the extension call.
    if is_docker_available() && is_docker_daemon_running() {
        return DockerInstallResult {
            success: true,
            message: "Docker is already running.".to_string(),
            daemon_ready: true,
        };
    }

    tracing::info!("Invoking docker_engine extension to ensure Docker Engine...");

    let config = json!({
        "base_dir": local_docker_dir().to_string_lossy(),
        "timeout": 300,
        "wait_timeout": 120,
    });

    let ext_result = if let Some(cb) = on_progress {
        crate::plugin::run_extension_with_progress("docker_engine", "ensure", config, cb).await
    } else {
        crate::plugin::run_extension("docker_engine", "ensure", config).await
    };

    match ext_result {
        Ok(result) => {
            let success = result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
            let daemon_ready = result.get("daemon_ready").and_then(|v| v.as_bool()).unwrap_or(false);
            let wsl_mode = result.get("wsl_mode").and_then(|v| v.as_bool()).unwrap_or(false);
            let message = result.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();

            // Set WSL2 mode only when daemon is actually ready
            if daemon_ready && wsl_mode {
                set_wsl2_mode(true);
            }

            // For native (non-WSL) mode, add portable docker dir to PATH
            if daemon_ready && !wsl_mode {
                let docker_dir = local_docker_dir();
                if docker_dir.exists() {
                    if let Ok(current) = std::env::var("PATH") {
                        let dir_str = docker_dir.to_string_lossy().to_string();
                        if !current.contains(&dir_str) {
                            let sep = if cfg!(target_os = "windows") { ";" } else { ":" };
                            std::env::set_var("PATH", format!("{}{}{}", dir_str, sep, current));
                            tracing::info!("Added portable Docker dir to PATH: {}", dir_str);
                        }
                    }
                }
            }

            DockerInstallResult { success, message, daemon_ready }
        }
        Err(e) => {
            tracing::error!("docker_engine extension failed: {}", e);
            DockerInstallResult {
                success: false,
                message: format!("Docker Engine 확장 실행 실패: {}", e),
                daemon_ready: false,
            }
        }
    }
}

/// Get Docker version info via the Python extension.
pub async fn docker_engine_info() -> Value {
    let config = json!({
        "base_dir": local_docker_dir().to_string_lossy(),
    });
    match crate::plugin::run_extension("docker_engine", "info", config).await {
        Ok(result) => result,
        Err(e) => json!({
            "success": false,
            "message": format!("docker_engine info failed: {}", e),
        }),
    }
}

// ─── Docker Compose Template Generator ──────────────────────────────

/// Template variable context for docker-compose.yml generation.
/// All `{variable}` placeholders in module.toml [docker] section get resolved from this.
pub struct ComposeTemplateContext {
    pub instance_id: String,
    pub instance_name: String,
    pub module_name: String,
    pub port: Option<u16>,
    pub rcon_port: Option<u16>,
    pub rest_port: Option<u16>,
    pub rest_password: Option<String>,
    /// Extra variables from module_settings
    pub extra_vars: std::collections::HashMap<String, String>,
}

impl ComposeTemplateContext {
    /// Resolve a template string by replacing `{var}` placeholders with actual values.
    pub fn resolve(&self, template: &str) -> String {
        let mut result = template.to_string();
        result = result.replace("{instance_id}", &self.instance_id);
        result = result.replace("{instance_id_short}", &self.instance_id[..8.min(self.instance_id.len())]);
        result = result.replace("{instance_name}", &self.instance_name);
        result = result.replace("{module_name}", &self.module_name);
        if let Some(port) = self.port {
            result = result.replace("{port}", &port.to_string());
        }
        if let Some(rcon_port) = self.rcon_port {
            result = result.replace("{rcon_port}", &rcon_port.to_string());
        }
        if let Some(rest_port) = self.rest_port {
            result = result.replace("{rest_port}", &rest_port.to_string());
        }
        if let Some(ref pw) = self.rest_password {
            result = result.replace("{rest_password}", pw);
        }
        for (key, value) in &self.extra_vars {
            result = result.replace(&format!("{{{}}}", key), value);
        }
        result
    }
}

/// Generate a docker-compose.yml string from module Docker config + instance context.
pub fn generate_compose_yaml(
    docker_config: &crate::supervisor::module_loader::DockerExtensionConfig,
    ctx: &ComposeTemplateContext,
) -> String {
    let mut lines = Vec::new();
    let service_name = &ctx.module_name;
    let container_name = format!(
        "saba-{}-{}",
        ctx.module_name,
        &ctx.instance_id[..8.min(ctx.instance_id.len())]
    );

    lines.push("services:".to_string());
    lines.push(format!("  {}:", service_name));
    lines.push(format!("    image: {}", ctx.resolve(&docker_config.image)));
    lines.push(format!("    container_name: {}", container_name));

    // Restart policy
    lines.push(format!("    restart: {}", docker_config.restart));

    // Ports
    if !docker_config.ports.is_empty() {
        lines.push("    ports:".to_string());
        for port in &docker_config.ports {
            lines.push(format!("      - \"{}\"", ctx.resolve(port)));
        }
    }

    // Volumes
    if !docker_config.volumes.is_empty() {
        lines.push("    volumes:".to_string());
        for vol in &docker_config.volumes {
            lines.push(format!("      - \"{}\"", ctx.resolve(vol)));
        }
    }

    // Environment
    if !docker_config.environment.is_empty() {
        lines.push("    environment:".to_string());
        for (key, value) in &docker_config.environment {
            lines.push(format!("      {}: \"{}\"", key, ctx.resolve(value)));
        }
    }

    // Working directory
    if let Some(ref wd) = docker_config.working_dir {
        lines.push(format!("    working_dir: {}", ctx.resolve(wd)));
    }

    // Entrypoint
    if let Some(ref ep) = docker_config.entrypoint {
        let resolved = ctx.resolve(ep);
        // Render entrypoint as YAML list for proper escaping
        let parts: Vec<&str> = resolved.split_whitespace().collect();
        if parts.len() == 1 {
            lines.push(format!("    entrypoint: [\"{}\"]", parts[0]));
        } else {
            let items: Vec<String> = parts.iter().map(|p| format!("\"{}\"" , p)).collect();
            lines.push(format!("    entrypoint: [{}]", items.join(", ")));
        }
    }

    // Command — render as YAML list so the entire string is a single exec argument.
    // This is critical when used with entrypoint ["/bin/bash", "-c"] where the
    // whole command must be passed as one argument to bash.
    if let Some(ref cmd) = docker_config.command {
        let resolved = ctx.resolve(cmd);
        lines.push(format!("    command: [\"{}\"]", resolved.replace('"', "\\\"")));
    }

    // User (run container as non-root)
    if let Some(ref user) = docker_config.user {
        lines.push(format!("    user: \"{}\"", ctx.resolve(user)));
    }

    // Resource limits (CPU / Memory)
    if docker_config.cpu_limit.is_some() || docker_config.memory_limit.is_some() {
        lines.push("    deploy:".to_string());
        lines.push("      resources:".to_string());
        lines.push("        limits:".to_string());
        if let Some(cpus) = docker_config.cpu_limit {
            lines.push(format!("          cpus: \"{}\"", cpus));
        }
        if let Some(ref mem) = docker_config.memory_limit {
            lines.push(format!("          memory: {}", ctx.resolve(mem)));
        }
    }

    // Stdin open + tty for interactive containers
    lines.push("    stdin_open: true".to_string());
    lines.push("    tty: true".to_string());

    lines.join("\n") + "\n"
}

/// Write the docker-compose.yml into an instance directory.
/// Returns the path to the generated file.
pub fn provision_compose_file(
    instance_dir: &Path,
    docker_config: &crate::supervisor::module_loader::DockerExtensionConfig,
    ctx: &ComposeTemplateContext,
) -> Result<PathBuf> {
    let yaml = generate_compose_yaml(docker_config, ctx);
    let compose_path = instance_dir.join("docker-compose.yml");
    std::fs::write(&compose_path, &yaml)?;
    tracing::info!(
        "Generated docker-compose.yml for instance '{}' at {}",
        ctx.instance_name,
        compose_path.display()
    );
    Ok(compose_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_docker_compose_config_defaults() {
        let config = DockerComposeConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.compose_file, "docker-compose.yml");
        assert!(config.service_name.is_none());
        assert!(config.environment.is_empty());
    }

    #[test]
    fn test_docker_compose_manager_no_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mgr = DockerComposeManager::new(tmp.path(), None);
        assert!(!mgr.has_compose_file());
    }

    #[test]
    fn test_docker_compose_manager_with_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("docker-compose.yml"), "version: '3'").unwrap();
        let mgr = DockerComposeManager::new(tmp.path(), None);
        assert!(mgr.has_compose_file());
    }

    #[test]
    fn test_docker_compose_manager_custom_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("compose.yaml"), "version: '3'").unwrap();
        let config = DockerComposeConfig {
            enabled: true,
            compose_file: "compose.yaml".to_string(),
            service_name: Some("gameserver".to_string()),
            environment: std::collections::HashMap::new(),
        };
        let mgr = DockerComposeManager::new(tmp.path(), Some(&config));
        assert!(mgr.has_compose_file());
        assert_eq!(mgr.compose_file_path(), tmp.path().join("compose.yaml"));
    }

    #[test]
    fn test_local_docker_paths() {
        let dir = local_docker_dir();
        assert!(dir.ends_with("docker"));
        let cli = docker_cli_path();
        // docker_cli_path should return something
        assert!(!cli.as_os_str().is_empty());
    }

    #[test]
    fn test_compose_template_context_resolve() {
        let ctx = ComposeTemplateContext {
            instance_id: "abcdef12-3456-7890-abcd-ef1234567890".to_string(),
            instance_name: "My Server".to_string(),
            module_name: "palworld".to_string(),
            port: Some(8211),
            rcon_port: None,
            rest_port: Some(8212),
            rest_password: Some("secretpw".to_string()),
            extra_vars: std::collections::HashMap::new(),
        };
        assert_eq!(ctx.resolve("{port}:8211/udp"), "8211:8211/udp");
        assert_eq!(ctx.resolve("{rest_port}:8212/tcp"), "8212:8212/tcp");
        assert_eq!(ctx.resolve("saba-{module_name}-{instance_id_short}"), "saba-palworld-abcdef12");
        assert_eq!(ctx.resolve("{rest_password}"), "secretpw");
    }

    #[test]
    fn test_generate_compose_yaml() {
        use crate::supervisor::module_loader::DockerExtensionConfig;
        let config = DockerExtensionConfig {
            image: "steamcmd/steamcmd:latest".to_string(),
            working_dir: Some("/server".to_string()),
            restart: "unless-stopped".to_string(),
            command: Some("/server/PalServer.sh --port={port}".to_string()),
            entrypoint: None,
            user: None,
            ports: vec!["{port}:8211/udp".to_string(), "{rest_port}:8212/tcp".to_string()],
            volumes: vec!["./server:/server".to_string()],
            environment: {
                let mut m = std::collections::HashMap::new();
                m.insert("PORT".to_string(), "{port}".to_string());
                m
            },
            dockerfile: None,
            extra_options: std::collections::HashMap::new(),
            cpu_limit: Some(4.0),
            memory_limit: Some("8g".to_string()),
        };
        let ctx = ComposeTemplateContext {
            instance_id: "11111111-2222-3333-4444-555555555555".to_string(),
            instance_name: "Test".to_string(),
            module_name: "palworld".to_string(),
            port: Some(8211),
            rcon_port: None,
            rest_port: Some(8212),
            rest_password: None,
            extra_vars: std::collections::HashMap::new(),
        };
        let yaml = generate_compose_yaml(&config, &ctx);
        assert!(yaml.contains("image: steamcmd/steamcmd:latest"));
        assert!(yaml.contains("container_name: saba-palworld-11111111"));
        assert!(yaml.contains("8211:8211/udp"));
        assert!(yaml.contains("8212:8212/tcp"));
        assert!(yaml.contains("working_dir: /server"));
        assert!(yaml.contains("/server/PalServer.sh --port=8211"));
        assert!(yaml.contains("PORT: \"8211\""));
        assert!(yaml.contains("deploy:"));
        assert!(yaml.contains("cpus: \"4\""));
        assert!(yaml.contains("memory: 8g"));
    }

    #[test]
    fn test_provision_compose_file() {
        use crate::supervisor::module_loader::DockerExtensionConfig;
        let tmp = tempfile::TempDir::new().unwrap();
        let config = DockerExtensionConfig {
            image: "openjdk:21".to_string(),
            working_dir: Some("/server".to_string()),
            restart: "unless-stopped".to_string(),
            command: Some("java -jar server.jar".to_string()),
            entrypoint: None,
            user: None,
            ports: vec!["25565:25565/tcp".to_string()],
            volumes: vec!["./server:/server".to_string()],
            environment: std::collections::HashMap::new(),
            dockerfile: None,
            extra_options: std::collections::HashMap::new(),
            cpu_limit: None,
            memory_limit: None,
        };
        let ctx = ComposeTemplateContext {
            instance_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
            instance_name: "MC".to_string(),
            module_name: "minecraft".to_string(),
            port: Some(25565),
            rcon_port: Some(25575),
            rest_port: None,
            rest_password: None,
            extra_vars: std::collections::HashMap::new(),
        };
        let path = provision_compose_file(tmp.path(), &config, &ctx).unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("openjdk:21"));
        assert!(content.contains("saba-minecraft-aaaaaaaa"));
    }
}
