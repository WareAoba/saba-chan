//! Native server migration: detection, scan, and execute migration plans.

use anyhow::Result;
use serde_json::{json, Value};

use super::Supervisor;
use super::extension_loader;

impl Supervisor {
    /// Detect existing server installations on the host for a given module.
    /// Scans [detection].common_paths from extension.toml and returns found locations.
    pub fn detect_native_servers(&self, module_name: &str) -> Result<Value> {
        let ext = self.extension_loader.get_extension(module_name)?;
        
        // We don't have detection.common_paths in ModuleMetadata directly,
        // so we re-read the extension.toml to get detection paths
        let module_dir = std::path::Path::new(&ext.path);
        let toml_path = module_dir.join("extension.toml");
        if !toml_path.exists() {
            return Ok(json!({
                "module": module_name,
                "servers": [],
                "message": "No extension.toml found for detection"
            }));
        }

        let content = std::fs::read_to_string(&toml_path)?;
        let toml_value: toml::Value = toml::from_str(&content)?;

        let common_paths = toml_value
            .get("detection")
            .and_then(|d| d.get("common_paths"))
            .and_then(|p| p.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
            .unwrap_or_default();

        let process_patterns = toml_value
            .get("detection")
            .and_then(|d| d.get("process_patterns"))
            .and_then(|p| p.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>())
            .unwrap_or_default();

        let mut found_servers = Vec::new();

        // Scan common paths (supports glob-like simple patterns)
        for pattern in &common_paths {
            if pattern.contains('*') {
                // Simple glob: expand parent dir
                if let Some(parent) = std::path::Path::new(pattern).parent() {
                    let parent_str = parent.to_string_lossy().to_string();
                    // Handle C:\Users\*\ pattern
                    if parent_str.contains('*') {
                        // Skip complex globs for now
                        continue;
                    }
                }
            } else {
                let path = std::path::Path::new(pattern);
                if path.exists() && path.is_dir() {
                    // Check if it looks like a server directory
                    let has_server_files = Self::detect_server_files(path, &ext.metadata);
                    if has_server_files {
                        found_servers.push(json!({
                            "path": pattern,
                            "type": "directory",
                            "verified": true,
                        }));
                    } else {
                        found_servers.push(json!({
                            "path": pattern,
                            "type": "directory",
                            "verified": false,
                        }));
                    }
                }
            }
        }

        // Also check for running processes
        let mut running_processes = Vec::new();
        for proc_name in &process_patterns {
            if let Ok(procs) = crate::process_monitor::ProcessMonitor::find_by_name(proc_name) {
                for p in procs {
                    running_processes.push(json!({
                        "pid": p.pid,
                        "name": p.name,
                    }));
                }
            }
        }

        Ok(json!({
            "module": module_name,
            "servers": found_servers,
            "running_processes": running_processes,
        }))
    }

    /// Check if a directory contains server files matching the module
    fn detect_server_files(dir: &std::path::Path, metadata: &extension_loader::ExtensionMetadata) -> bool {
        if let Some(ref exe) = metadata.executable_path {
            let exe_name = std::path::Path::new(exe)
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default();
            // Check if executable exists in the directory
            if dir.join(&exe_name).exists() {
                return true;
            }
        }
        // Check for common server files
        dir.join("server.properties").exists()        // Minecraft
            || dir.join("PalServer.exe").exists()      // Palworld (Windows)
            || dir.join("PalServer.sh").exists()        // Palworld (Linux)
            || dir.join("DefaultPalWorldSettings.ini").exists()  // Palworld
    }

    /// Migrate a native (non-saba-chan) server installation into a saba-chan instance.
    ///
    /// This creates a new instance, copies/moves the server files into it,
    /// generates the Docker setup, and registers it.
    pub fn migrate_native_server(
        &mut self,
        module_name: &str,
        server_path: &std::path::Path,
        instance_name: Option<&str>,
        move_files: bool,
    ) -> Result<Value> {
        // Validate module exists
        let ext = self.extension_loader.get_extension(module_name)?;

        // Validate source path
        if !server_path.exists() {
            return Err(anyhow::anyhow!(
                "Server path not found: {}", server_path.display()
            ));
        }

        // Create new instance
        let name = instance_name
            .map(String::from)
            .unwrap_or_else(|| format!("{} (migrated)", ext.metadata.name));
        let mut instance = crate::instance::ServerInstance::new(&name, module_name);

        // Set default port from module
        instance.port = ext.metadata.default_port;
        // NOTE: executable_path is NOT set for Docker-mode instances.
        // Server binaries live inside {instance_dir}/server/ and are run via Docker.

        let instance_dir = self.instance_store.get_instance_dir(&instance.id);
        std::fs::create_dir_all(&instance_dir)?;

        // Copy or move server files into instance/server/
        let server_dest = instance_dir.join(
            ext.metadata.install.as_ref()
                .map(|i| i.install_subdir.as_str())
                .unwrap_or("server")
        );

        if move_files {
            // Move: rename if on same filesystem, else copy+delete
            if let Err(_) = std::fs::rename(server_path, &server_dest) {
                // Different filesystem — fall back to copy + delete
                crate::utils::copy_dir_recursive(server_path, &server_dest)?;
                std::fs::remove_dir_all(server_path)?;
            }
            tracing::info!("Moved server files from {} to {}", server_path.display(), server_dest.display());
        } else {
            crate::utils::copy_dir_recursive(server_path, &server_dest)?;
            tracing::info!("Copied server files from {} to {}", server_path.display(), server_dest.display());
        }

        // Generate docker-compose.yml if module has [docker]
        if let Some(ref docker_config) = ext.metadata.docker {
            let ctx = crate::docker::ComposeTemplateContext {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                module_name: module_name.to_string(),
                port: instance.port,
                rcon_port: instance.rcon_port,
                rest_port: instance.rest_port,
                rest_password: instance.rest_password.clone(),
                extra_vars: std::collections::HashMap::new(),
            };
            crate::docker::provision_compose_file(&instance_dir, docker_config, &ctx)?;
        }

        // Persist the instance
        let instance_id = instance.id.clone();
        let instance_name_out = instance.name.clone();
        let instance_docker_mode = &instance.docker_mode;
        let docker_provisioned = *instance_docker_mode == crate::instance::DockerMode::DockerCompose && ext.metadata.docker.is_some();
        self.instance_store.add(instance)?;

        tracing::info!(
            "Migrated native server '{}' from {} → instance {} ({})",
            instance_name_out, server_path.display(), instance_id, instance_name_out
        );

        Ok(json!({
            "success": true,
            "instance_id": instance_id,
            "instance_name": instance_name_out,
            "source_path": server_path.to_string_lossy().to_string(),
            "moved": move_files,
            "docker_provisioned": docker_provisioned,
        }))
    }

    // ─── Smart Migration (scan → review → execute) ─────────────

    /// Scan an existing server directory to detect migratable data.
    ///
    /// Reads the module's `[migration]` profile from extension.toml, then
    /// scans the source directory to find matching data (worlds, configs,
    /// mods/plugins, etc.)
    ///
    /// Returns a `MigrationPlan` that the caller (GUI) can present for
    /// review before executing.
    pub fn scan_migration(
        &self,
        module_name: &str,
        source_dir: &std::path::Path,
    ) -> Result<crate::instance::migration::MigrationPlan> {
        let ext = self.extension_loader.get_extension(module_name)?;

        // Read migration config from the module's TOML
        let module_dir = std::path::Path::new(&ext.path);
        let toml_path = module_dir.join("extension.toml");
        let migration_config = if toml_path.exists() {
            crate::instance::migration::read_migration_config(&toml_path)?
        } else {
            tracing::warn!("No extension.toml found for '{}', using empty migration config", module_name);
            crate::instance::migration::MigrationConfig::default()
        };

        let scanner = crate::instance::migration::MigrationScanner::new(
            &migration_config,
            module_name,
        );

        scanner.scan(source_dir)
    }

    /// Execute a reviewed migration plan.
    ///
    /// This creates a new instance, copies the selected
    /// data items into the instance directory, and provisions Docker.
    ///
    /// The `plan` should have been returned by `scan_migration()` and
    /// optionally modified (toggling items on/off).
    pub fn execute_migration(
        &mut self,
        plan: &crate::instance::migration::MigrationPlan,
        instance_name: Option<&str>,
        move_files: bool,
    ) -> Result<Value> {
        let ext = self.extension_loader.get_extension(&plan.module_name)?;

        // Create new instance
        let name = instance_name
            .map(String::from)
            .unwrap_or_else(|| format!("{} (migrated)", ext.metadata.name));
        let mut instance = crate::instance::ServerInstance::new(&name, &plan.module_name);
        instance.port = ext.metadata.default_port;

        let instance_dir = self.instance_store.get_instance_dir(&instance.id);
        std::fs::create_dir_all(&instance_dir)?;

        // Create server directory (target for migrated data)
        let server_subdir = ext.metadata.install.as_ref()
            .map(|i| i.install_subdir.as_str())
            .unwrap_or("server");
        std::fs::create_dir_all(instance_dir.join(server_subdir))?;

        // Assign instance ID to the plan for the executor
        let mut exec_plan = plan.clone();
        exec_plan.instance_id = Some(instance.id.clone());

        // Execute migration — copy selected items
        let mut migration_result = crate::instance::migration::MigrationExecutor::execute(
            &exec_plan,
            &instance_dir,
            move_files,
        )?;

        // Generate docker-compose.yml if module has [docker]
        let docker_provisioned = if let Some(ref docker_config) = ext.metadata.docker {
            let ctx = crate::docker::ComposeTemplateContext {
                instance_id: instance.id.clone(),
                instance_name: instance.name.clone(),
                module_name: plan.module_name.clone(),
                port: instance.port,
                rcon_port: instance.rcon_port,
                rest_port: instance.rest_port,
                rest_password: instance.rest_password.clone(),
                extra_vars: std::collections::HashMap::new(),
            };
            crate::docker::provision_compose_file(&instance_dir, docker_config, &ctx)?;
            true
        } else {
            false
        };

        migration_result.instance_id = instance.id.clone();
        migration_result.instance_name = instance.name.clone();
        migration_result.docker_provisioned = docker_provisioned;

        // Persist the instance
        let instance_id = instance.id.clone();
        let instance_name_out = instance.name.clone();
        self.instance_store.add(instance)?;

        tracing::info!(
            "Smart migration completed: {} items migrated for '{}' → instance {} ({})",
            migration_result.items_migrated.len(),
            instance_name_out,
            instance_id,
            instance_name_out,
        );

        Ok(serde_json::to_value(&migration_result)?)
    }

    /// Provision (generate) docker-compose.yml for an existing instance.
    /// Useful for re-generating after settings change or for first-time setup.
    pub fn provision_docker(&self, instance_id: &str) -> Result<Value> {
        let instance = self.instance_store.get(instance_id)
            .ok_or_else(|| anyhow::anyhow!("Instance not found: {}", instance_id))?;

        let module = self.resolve_extension(instance)?;

        let docker_config = ext.metadata.docker.as_ref()
            .ok_or_else(|| anyhow::anyhow!(
                "Module '{}' does not have a [docker] configuration", ext.metadata.name
            ))?;

        let instance_dir = self.instance_store.get_instance_dir(instance_id);
        let ctx = self.build_compose_context(instance, &module);
        let path = crate::docker::provision_compose_file(&instance_dir, docker_config, &ctx)?;

        Ok(json!({
            "success": true,
            "instance_id": instance_id,
            "compose_path": path.to_string_lossy().to_string(),
        }))
    }
}
