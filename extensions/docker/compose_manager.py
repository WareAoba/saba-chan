"""
Docker Compose Manager Extension — saba-chan Docker Isolation

Rust의 DockerComposeManager를 Python으로 포팅한 모듈.
run_plugin 프로토콜에 맞게 각 함수는 stdin으로 config JSON을 받고,
stdout으로 결과 JSON을 출력합니다.
"""

import json
import os
import subprocess
import sys
import platform

# ── WSL2 모드 전역 상태 ──────────────────────────
# Windows에서는 Docker Desktop 대신 WSL2 내부의 standalone dockerd를 사용.
# 따라서 Windows이면 자동으로 WSL2 모드 활성화.
_wsl2_mode = (platform.system() == "Windows")
WSL2_DOCKER_DIR = "/opt/saba-chan/docker"


def _set_wsl2_mode(enabled: bool):
    global _wsl2_mode
    _wsl2_mode = enabled


def _is_wsl2_mode() -> bool:
    return _wsl2_mode


# ── Docker CLI 경로 유틸리티 ──────────────────────

def _local_docker_dir() -> str:
    """exe 옆의 docker/ 디렉토리 경로"""
    exe_dir = os.path.dirname(os.path.abspath(sys.argv[0]))
    return os.path.join(exe_dir, "docker")


def _docker_cli() -> list:
    """Docker CLI 명령 prefix 반환"""
    if _is_wsl2_mode():
        return ["wsl", "-u", "root", "--", f"{WSL2_DOCKER_DIR}/docker"]
    return ["docker"]


def _compose_cli() -> list:
    """Docker Compose CLI 명령 prefix 반환"""
    if _is_wsl2_mode():
        return ["wsl", "-u", "root", "--", f"{WSL2_DOCKER_DIR}/docker", "compose"]
    # 로컬 portable compose
    local_compose = os.path.join(
        _local_docker_dir(),
        "docker-compose.exe" if platform.system() == "Windows" else "docker-compose",
    )
    if os.path.exists(local_compose):
        return [local_compose]
    return ["docker", "compose"]


def _run_cmd(cmd: list, cwd: str = None, timeout: int = 120) -> tuple:
    """명령 실행 → (success, stdout, stderr)"""
    try:
        kwargs = dict(
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=cwd,
            timeout=timeout,
        )
        if platform.system() == "Windows":
            kwargs["creationflags"] = 0x08000000  # CREATE_NO_WINDOW
        result = subprocess.run(cmd, **kwargs)
        stdout = result.stdout.decode("utf-8", errors="replace")
        stderr = result.stderr.decode("utf-8", errors="replace")
        return result.returncode == 0, stdout, stderr
    except subprocess.TimeoutExpired:
        return False, "", "Command timed out"
    except FileNotFoundError:
        return False, "", f"Command not found: {cmd[0]}"
    except Exception as e:
        return False, "", str(e)


def _compose_cmd(instance_dir: str, compose_file: str = "docker-compose.yml") -> list:
    """compose 명령의 기본 부분 구성"""
    base = _compose_cli()
    if _is_wsl2_mode():
        base += ["-f", compose_file]
    else:
        base += ["-f", os.path.join(instance_dir, compose_file)]
    return base


def _container_name(module_name: str, instance_id: str) -> str:
    """saba-{module}-{instance_id[:8]} 규칙"""
    return f"saba-{module_name}-{instance_id[:8]}"


def _progress(percent: int = None, message: str = None):
    """PROGRESS 프로토콜로 진행률 보고"""
    info = {}
    if percent is not None:
        info["percent"] = percent
    if message is not None:
        info["message"] = message
    sys.stderr.write(f"PROGRESS:{json.dumps(info)}\n")
    sys.stderr.flush()


# ═══════════════════════════════════════════════════
#  Hook 함수들 — Rust DockerComposeManager 대응
# ═══════════════════════════════════════════════════

def start(config: dict) -> dict:
    """server.pre_start — docker compose up -d
    
    Rust: DockerComposeManager::start() (D4)
    """
    instance_dir = config["instance_dir"]
    compose_file = "docker-compose.yml"
    compose_path = os.path.join(instance_dir, compose_file)

    if not os.path.exists(compose_path):
        return {"handled": True, "success": False, "error": f"No {compose_file} found in {instance_dir}"}

    cmd = _compose_cmd(instance_dir, compose_file) + ["up", "-d"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=300)

    if success:
        return {"handled": True, "success": True, "message": "Docker Compose containers started", "stdout": stdout}
    else:
        return {"handled": True, "success": False, "error": f"Docker Compose up failed: {stderr or stdout}"}


def stop(config: dict) -> dict:
    """server.post_stop — docker compose stop
    
    Rust: DockerComposeManager::stop() (D5)
    """
    instance_dir = config["instance_dir"]
    compose_file = "docker-compose.yml"

    cmd = _compose_cmd(instance_dir, compose_file) + ["stop"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=120)

    if success:
        return {"handled": True, "success": True, "message": "Docker Compose containers stopped"}
    else:
        return {"handled": True, "success": False, "error": f"Docker Compose stop failed: {stderr or stdout}"}


def cleanup(config: dict) -> dict:
    """server.pre_delete — docker compose down
    
    Rust: DockerComposeManager::down() (D6)
    """
    instance_dir = config["instance_dir"]
    compose_file = "docker-compose.yml"

    cmd = _compose_cmd(instance_dir, compose_file) + ["down"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=120)

    # down 실패는 치명적이지 않음
    return {"handled": True, "success": True, "message": "Docker Compose down completed"}


def status(config: dict) -> dict:
    """server.status — docker compose ps + docker top

    Rust: DockerComposeManager::status() (D7) + server_process_running() (D8)
    
    반환:
      handled=True
      running: bool
      server_process_running: bool
      container_name: str or None
      status: "running" | "starting" | "stopped"
    """
    instance_dir = config["instance_dir"]
    compose_file = "docker-compose.yml"
    process_patterns = config.get("process_patterns", [])

    # docker compose ps --format json
    cmd = _compose_cmd(instance_dir, compose_file) + ["ps", "--format", "json", "-a"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=30)

    if not success:
        return {
            "handled": True,
            "running": False,
            "server_process_running": False,
            "container_name": None,
            "status": "stopped",
        }

    # stdout 파싱 — JSON-lines 또는 단일 JSON
    containers = []
    for line in stdout.strip().splitlines():
        line = line.strip()
        if line:
            try:
                containers.append(json.loads(line))
            except json.JSONDecodeError:
                pass

    container_running = any(
        c.get("State") == "running" for c in containers
    )
    container_name = None
    if containers:
        container_name = containers[0].get("Name") or containers[0].get("Names")

    if not container_running:
        return {
            "handled": True,
            "running": False,
            "server_process_running": False,
            "container_name": container_name,
            "status": "stopped",
        }

    # 컨테이너 running → 프로세스 패턴 매칭 (D8)
    server_proc_running = True  # default if no patterns
    matched_process = None

    if process_patterns and container_name:
        server_proc_running, matched_process = _check_server_process(container_name, process_patterns)

    final_status = "running" if server_proc_running else "starting"

    return {
        "handled": True,
        "running": True,
        "server_process_running": server_proc_running,
        "container_name": container_name,
        "matched_process": matched_process,
        "status": final_status,
    }


def _check_server_process(container_name: str, process_patterns: list) -> tuple:
    """docker top으로 서버 프로세스 존재 확인 (D8)
    
    Note: `-eo args` 옵션은 일부 컨테이너 이미지(steamcmd 등)에서
    지원하지 않으므로 기본 `docker top`을 사용한다 (원래 Rust 구현과 동일).
    """
    docker = _docker_cli()
    cmd = docker + ["top", container_name]
    success, stdout, stderr = _run_cmd(cmd, timeout=15)

    if not success:
        return False, None

    # 헤더 스킵 후 각 라인에서 패턴 매칭
    lines = stdout.splitlines()[1:]  # 첫 줄은 헤더
    for line in lines:
        line_lower = line.lower()
        for pattern in process_patterns:
            if pattern.lower() in line_lower:
                return True, pattern

    return False, None


def container_stats(config: dict) -> dict:
    """server.stats — docker stats --no-stream
    
    Rust: docker_container_stats() (D11)
    
    반환: docker_memory_usage, docker_memory_percent, docker_cpu_percent
    """
    ext_data = config.get("extension_data", {})
    instance_dir = config.get("instance_dir", "")
    
    # 먼저 컨테이너 이름 파악
    compose_file = "docker-compose.yml"
    cmd = _compose_cmd(instance_dir, compose_file) + ["ps", "--format", "json", "-a"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=15)

    container_name = None
    if success:
        for line in stdout.strip().splitlines():
            try:
                c = json.loads(line.strip())
                container_name = c.get("Name") or c.get("Names")
                break
            except json.JSONDecodeError:
                pass

    if not container_name:
        return {"handled": True, "success": False, "error": "Container name not found"}

    # docker stats
    docker = _docker_cli()
    fmt = "{{json .}}"
    cmd = docker + ["stats", "--no-stream", "--format", fmt, container_name]
    success, stdout, stderr = _run_cmd(cmd, timeout=15)

    if not success:
        return {"handled": True, "success": False, "error": f"docker stats failed: {stderr}"}

    try:
        stats = json.loads(stdout.strip())
    except json.JSONDecodeError:
        return {"handled": True, "success": False, "error": f"Invalid stats JSON: {stdout}"}

    # 파싱: MemUsage "256MiB / 4GiB", MemPerc "6.25%", CPUPerc "12.50%"
    mem_usage = stats.get("MemUsage", "")
    mem_perc_str = stats.get("MemPerc", "0%").replace("%", "")
    cpu_perc_str = stats.get("CPUPerc", "0%").replace("%", "")

    try:
        mem_perc = float(mem_perc_str)
    except ValueError:
        mem_perc = 0.0
    try:
        cpu_perc = float(cpu_perc_str)
    except ValueError:
        cpu_perc = 0.0

    return {
        "handled": True,
        "success": True,
        "container_name": container_name,
        "docker_memory_usage": mem_usage,
        "docker_memory_percent": mem_perc,
        "docker_cpu_percent": cpu_perc,
    }


def shutdown_all(config: dict) -> dict:
    """daemon.shutdown — 모든 Docker 인스턴스의 compose down
    
    Rust: main.rs shutdown 루프 (M1)
    """
    instances = config.get("instances", [])
    results = []

    for inst in instances:
        ext_data = inst.get("extension_data", {})
        if not ext_data.get("docker_enabled", False):
            continue

        instance_dir = inst.get("instance_dir", "")
        if not instance_dir:
            continue

        compose_file = "docker-compose.yml"
        compose_path = os.path.join(instance_dir, compose_file)
        if not os.path.exists(compose_path):
            continue

        cmd = _compose_cmd(instance_dir, compose_file) + ["down"]
        success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=60)
        results.append({
            "instance_id": inst.get("instance_id", ""),
            "success": success,
        })

    return {"handled": True, "success": True, "results": results}


def enrich_server_info(config: dict) -> dict:
    """server.list_enrich — ServerInfo에 Docker 상태 + 리소스 통계 병합
    
    Rust: list_servers의 Docker 관련 필드 (M11)
    docker compose ps로 컨테이너 상태를 확인하고,
    running이면 docker stats로 CPU/메모리 사용량도 조회.
    """
    ext_data = config.get("extension_data", {})
    instance_dir = config.get("instance_dir", "")
    process_patterns = config.get("process_patterns", [])
    compose_file = "docker-compose.yml"
    compose_path = os.path.join(instance_dir, compose_file) if instance_dir else ""

    # compose 파일이 없으면 Docker 모드가 아님
    if not instance_dir or not os.path.exists(compose_path):
        return {
            "handled": False,
            "docker_enabled": ext_data.get("docker_enabled", False),
            "docker_cpu_limit": ext_data.get("docker_cpu_limit"),
            "docker_memory_limit": ext_data.get("docker_memory_limit"),
        }

    # docker compose ps — 컨테이너 상태 확인
    cmd = _compose_cmd(instance_dir, compose_file) + ["ps", "--format", "json", "-a"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=15)

    if not success:
        return {
            "handled": True,
            "status": "stopped",
            "docker_enabled": ext_data.get("docker_enabled", False),
            "docker_cpu_limit": ext_data.get("docker_cpu_limit"),
            "docker_memory_limit": ext_data.get("docker_memory_limit"),
        }

    containers = []
    for line in stdout.strip().splitlines():
        line = line.strip()
        if line:
            try:
                containers.append(json.loads(line))
            except json.JSONDecodeError:
                pass

    container_running = any(c.get("State") == "running" for c in containers)
    container_name = containers[0].get("Name") or containers[0].get("Names") if containers else None

    if not container_running:
        return {
            "handled": True,
            "status": "stopped",
            "docker_enabled": ext_data.get("docker_enabled", False),
            "docker_cpu_limit": ext_data.get("docker_cpu_limit"),
            "docker_memory_limit": ext_data.get("docker_memory_limit"),
        }

    # 프로세스 패턴 매칭 (starting vs running)
    final_status = "running"
    if process_patterns and container_name:
        proc_running, _ = _check_server_process(container_name, process_patterns)
        if not proc_running:
            final_status = "starting"

    # docker stats — CPU/메모리 사용량
    mem_usage = None
    mem_perc = None
    cpu_perc = None
    if container_name:
        docker = _docker_cli()
        fmt = "{{json .}}"
        stat_cmd = docker + ["stats", "--no-stream", "--format", fmt, container_name]
        stat_ok, stat_out, _ = _run_cmd(stat_cmd, timeout=10)
        if stat_ok and stat_out.strip():
            try:
                stats = json.loads(stat_out.strip())
                mem_usage = stats.get("MemUsage", "")
                try:
                    mem_perc = float(stats.get("MemPerc", "0%").replace("%", ""))
                except ValueError:
                    pass
                try:
                    cpu_perc = float(stats.get("CPUPerc", "0%").replace("%", ""))
                except ValueError:
                    pass
            except json.JSONDecodeError:
                pass

    return {
        "handled": True,
        "status": final_status,
        "docker_enabled": ext_data.get("docker_enabled", False),
        "docker_cpu_limit": ext_data.get("docker_cpu_limit"),
        "docker_memory_limit": ext_data.get("docker_memory_limit"),
        "memory_usage": mem_usage,
        "memory_percent": mem_perc,
        "cpu_percent": cpu_perc,
    }


def get_logs(config: dict) -> dict:
    """server.logs — docker compose logs
    
    Rust: DockerComposeManager::logs() (D9)
    """
    instance_dir = config["instance_dir"]
    lines = config.get("lines", 100)
    compose_file = "docker-compose.yml"

    cmd = _compose_cmd(instance_dir, compose_file) + ["logs", "--tail", str(lines)]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=30)

    return {
        "handled": True,
        "success": True,
        "logs": stdout,
    }


def pre_create(config: dict) -> dict:
    """server.pre_create — 인스턴스 생성 전 Docker 설정
    
    Rust: create_instance Docker 분기 (M7)
    """
    # Docker 모드에서는 추가 설정이 필요할 수 있음
    return {
        "handled": False,
        "success": True,
    }


def provision(config: dict) -> dict:
    """server.post_create — Docker 프로비저닝 파이프라인
    
    Rust: docker_provision() (M8)
    3단계: Docker 엔진 확인 → SteamCMD 설치 → compose 생성
    """
    instance_id = config.get("instance_id", "")
    instance_dir = config.get("instance_dir", "")
    ext_data = config.get("extension_data", {})
    module_config = config.get("module_config", {})

    # ── Step 0: Docker Engine 확인 ──
    _progress(0, "Checking Docker Engine...")

    from . import docker_engine
    engine_result = docker_engine._ensure_inner(config.get("docker_engine_config", {
        "base_dir": _local_docker_dir(),
        "timeout": 300,
        "wait_timeout": 120,
    }))

    if not engine_result.get("daemon_ready", False):
        return {
            "handled": True,
            "success": False,
            "error": f"Docker를 사용할 수 없습니다: {engine_result.get('message', 'unknown')}",
        }

    # WSL2 모드 설정
    if engine_result.get("wsl_mode", False):
        _set_wsl2_mode(True)

    _progress(33, "Docker Engine ready")

    # ── Step 1: SteamCMD 서버 파일 다운로드 ──
    install_config = module_config.get("install", {})
    server_dir = os.path.join(instance_dir, "server")
    os.makedirs(server_dir, exist_ok=True)

    if install_config.get("method") == "steamcmd":
        app_id = install_config.get("app_id")
        if app_id:
            _progress(40, f"Downloading server files (app {app_id})...")
            steamcmd_config = {
                "app_id": app_id,
                "install_dir": server_dir,
                "anonymous": install_config.get("anonymous", True),
                "platform": "linux" if _is_wsl2_mode() else install_config.get("platform"),
                "beta": install_config.get("beta"),
            }
            try:
                # steamcmd.py의 install 함수 호출
                ext_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
                steamcmd_path = os.path.join(ext_dir, "steamcmd.py")
                # 직접 import하여 호출 (같은 Python 프로세스 내에서)
                import importlib.util
                spec = importlib.util.spec_from_file_location("steamcmd", steamcmd_path)
                if spec and spec.loader:
                    steamcmd = importlib.util.module_from_spec(spec)
                    spec.loader.exec_module(steamcmd)
                    result = steamcmd.install(steamcmd_config)
                    if not result.get("success", False):
                        return {
                            "handled": True,
                            "success": False,
                            "error": f"SteamCMD 설치 실패: {result.get('error', result.get('message', 'unknown'))}",
                        }
            except Exception as e:
                return {
                    "handled": True,
                    "success": False,
                    "error": f"SteamCMD 실행 실패: {e}",
                }

    _progress(66, "Server files ready")

    # ── Step 2: docker-compose.yml 생성 ──
    _progress(80, "Generating docker-compose.yml...")

    docker_section = module_config.get("docker", {})
    if not docker_section.get("image"):
        return {
            "handled": True,
            "success": False,
            "error": "모듈에 [docker] image 설정이 없습니다",
        }

    instance_data = config.get("instance", config)
    yaml = _generate_compose_yaml(docker_section, instance_data)
    compose_path = os.path.join(instance_dir, "docker-compose.yml")
    with open(compose_path, "w", encoding="utf-8") as f:
        f.write(yaml)

    _progress(100, "docker-compose.yml generated")

    return {
        "handled": True,
        "success": True,
        "message": "Docker 프로비저닝 완료: docker-compose.yml 생성됨",
    }


def regenerate_compose(config: dict) -> dict:
    """server.settings_changed — 설정 변경 시 compose 재생성
    
    Rust: provision_compose_file() on settings_changed (D19, D20)
    """
    instance_dir = config.get("instance_dir", "")
    module_config = config.get("module_config", {})
    docker_section = module_config.get("docker", {})

    if not docker_section.get("image"):
        return {"handled": True, "success": False, "error": "No docker config"}

    instance_data = config.get("instance", config)
    yaml = _generate_compose_yaml(docker_section, instance_data)
    compose_path = os.path.join(instance_dir, "docker-compose.yml")
    with open(compose_path, "w", encoding="utf-8") as f:
        f.write(yaml)

    return {"handled": True, "success": True, "message": "docker-compose.yml regenerated"}


# ═══════════════════════════════════════════════════
#  docker-compose.yml 생성 — Rust generate_compose_yaml() 정밀 포팅
# ═══════════════════════════════════════════════════

def _resolve_template(template: str, ctx: dict) -> str:
    """템플릿 변수 치환 — Rust ComposeTemplateContext::resolve() 대응"""
    result = template
    result = result.replace("{instance_id}", ctx.get("instance_id", ""))
    instance_id = ctx.get("instance_id", "")
    result = result.replace("{instance_id_short}", instance_id[:8] if len(instance_id) >= 8 else instance_id)
    result = result.replace("{instance_name}", ctx.get("instance_name", ctx.get("name", "")))
    result = result.replace("{module_name}", ctx.get("module_name", ""))
    
    port = ctx.get("port")
    if port is not None:
        result = result.replace("{port}", str(port))
    
    rcon_port = ctx.get("rcon_port")
    if rcon_port is not None:
        result = result.replace("{rcon_port}", str(rcon_port))
    
    rest_port = ctx.get("rest_port")
    if rest_port is not None:
        result = result.replace("{rest_port}", str(rest_port))
    
    rest_password = ctx.get("rest_password")
    if rest_password is not None:
        result = result.replace("{rest_password}", str(rest_password))
    
    # 모듈 설정에서 추가 변수
    module_settings = ctx.get("module_settings", {})
    for key, value in module_settings.items():
        if isinstance(value, str):
            result = result.replace(f"{{{key}}}", value)
    
    return result


def _generate_compose_yaml(docker_config: dict, instance: dict) -> str:
    """
    docker-compose.yml YAML 생성 — Rust generate_compose_yaml() 정밀 포팅
    
    docker_config: module.toml [docker] 섹션
    instance: ServerInstance 데이터 (id, name, module_name, port, ext_data 등)
    """
    ctx = {
        "instance_id": instance.get("instance_id", instance.get("id", "")),
        "instance_name": instance.get("instance_name", instance.get("name", "")),
        "module_name": instance.get("module_name", ""),
        "port": instance.get("port"),
        "rcon_port": instance.get("rcon_port"),
        "rest_port": instance.get("rest_port"),
        "rest_password": instance.get("rest_password"),
        "module_settings": instance.get("module_settings", {}),
    }

    ext_data = instance.get("extension_data", {})
    
    lines = []
    service_name = ctx["module_name"]
    container = _container_name(ctx["module_name"], ctx["instance_id"])

    lines.append("services:")
    lines.append(f"  {service_name}:")
    lines.append(f"    image: {_resolve_template(docker_config['image'], ctx)}")
    lines.append(f"    container_name: {container}")

    # Restart policy
    restart = docker_config.get("restart", "unless-stopped")
    lines.append(f"    restart: {restart}")

    # Ports
    ports = docker_config.get("ports", [])
    if ports:
        lines.append("    ports:")
        for p in ports:
            lines.append(f'      - "{_resolve_template(p, ctx)}"')

    # Volumes
    volumes = docker_config.get("volumes", [])
    if volumes:
        lines.append("    volumes:")
        for v in volumes:
            lines.append(f'      - "{_resolve_template(v, ctx)}"')

    # Environment
    environment = docker_config.get("environment", {})
    if environment:
        lines.append("    environment:")
        for key, value in environment.items():
            lines.append(f'      {key}: "{_resolve_template(str(value), ctx)}"')

    # Working directory
    working_dir = docker_config.get("working_dir")
    if working_dir:
        lines.append(f"    working_dir: {_resolve_template(working_dir, ctx)}")

    # Entrypoint
    entrypoint = docker_config.get("entrypoint")
    if entrypoint:
        resolved = _resolve_template(entrypoint, ctx)
        parts = resolved.split()
        if len(parts) == 1:
            lines.append(f'    entrypoint: ["{parts[0]}"]')
        else:
            items = ", ".join(f'"{p}"' for p in parts)
            lines.append(f"    entrypoint: [{items}]")

    # Command
    command = docker_config.get("command")
    if command:
        resolved = _resolve_template(command, ctx)
        escaped = resolved.replace('"', '\\"')
        lines.append(f'    command: ["{escaped}"]')

    # User
    user = docker_config.get("user")
    if user:
        lines.append(f'    user: "{_resolve_template(user, ctx)}"')

    # Resource limits — per-instance override 적용
    cpu_limit = ext_data.get("docker_cpu_limit", docker_config.get("cpu_limit"))
    memory_limit = ext_data.get("docker_memory_limit", docker_config.get("memory_limit"))

    if cpu_limit is not None or memory_limit is not None:
        lines.append("    deploy:")
        lines.append("      resources:")
        lines.append("        limits:")
        if cpu_limit is not None:
            lines.append(f'          cpus: "{cpu_limit}"')
        if memory_limit is not None:
            mem_val = _resolve_template(str(memory_limit), ctx) if isinstance(memory_limit, str) else str(memory_limit)
            lines.append(f"          memory: {mem_val}")

    # stdin + tty
    lines.append("    stdin_open: true")
    lines.append("    tty: true")

    return "\n".join(lines) + "\n"


# ═══════════════════════════════════════════════════
#  run_plugin 프로토콜 엔트리포인트
# ═══════════════════════════════════════════════════

_FUNCTIONS = {
    "start": start,
    "stop": stop,
    "cleanup": cleanup,
    "status": status,
    "container_stats": container_stats,
    "shutdown_all": shutdown_all,
    "enrich_server_info": enrich_server_info,
    "get_logs": get_logs,
    "pre_create": pre_create,
    "provision": provision,
    "regenerate_compose": regenerate_compose,
}


def main():
    if len(sys.argv) < 2:
        print(json.dumps({"error": "No function specified"}))
        sys.exit(1)

    # sys.argv[0] = 모듈 경로, sys.argv[1] = 함수명
    func_name = sys.argv[1]
    func = _FUNCTIONS.get(func_name)
    if not func:
        print(json.dumps({"error": f"Unknown function: {func_name}"}))
        sys.exit(1)

    # stdin에서 config JSON 읽기
    config_str = sys.stdin.read()
    try:
        config = json.loads(config_str)
    except json.JSONDecodeError as e:
        print(json.dumps({"error": f"Invalid JSON config: {e}"}))
        sys.exit(1)

    # WSL2 모드 오버라이드 (config에서 명시적으로 전달된 경우)
    if "wsl2_mode" in config:
        _set_wsl2_mode(bool(config["wsl2_mode"]))

    result = func(config)
    print(json.dumps(result))


if __name__ == "__main__":
    main()
