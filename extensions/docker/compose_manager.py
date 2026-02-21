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
from pathlib import Path

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


# ── Docker 데몬 상태 확인 & 자동 시작 ────────────

def _docker_daemon_running() -> bool:
    """Docker 데몬이 응답하는지 확인"""
    docker = _docker_cli()
    ok, _, _ = _run_cmd(docker + ["info"], timeout=10)
    return ok


def _ensure_docker_daemon() -> dict | None:
    """Docker 데몬이 꺼져 있으면 docker_engine.ensure()로 시작.
    
    성공 시 None, 실패 시 에러 dict 반환.
    """
    if _docker_daemon_running():
        return None

    # 같은 패키지의 docker_engine 모듈 임포트
    try:
        ext_dir = Path(__file__).resolve().parent
        sys.path.insert(0, str(ext_dir.parent))
        from docker.docker_engine import DockerEngine
    except ImportError as e:
        return {"handled": True, "success": False,
                "error": f"Docker daemon is not running and auto-start failed (import error: {e})"}

    engine = DockerEngine()

    # 바이너리 준비
    if not engine.binaries_ready:
        try:
            engine.ensure_available()
        except Exception as e:
            return {"handled": True, "success": False,
                    "error": f"Failed to download Docker Engine: {e}"}

    # 데몬 시작
    try:
        engine.start_daemon()
    except Exception as e:
        return {"handled": True, "success": False,
                "error": f"Failed to start Docker daemon: {e}"}

    # 준비 대기
    if not engine.wait_for_ready(timeout_secs=60):
        return {"handled": True, "success": False,
                "error": "Docker daemon started but did not become ready in time"}

    return None


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


def _find_io_bridge() -> str | None:
    """saba-docker-io 바이너리 경로를 찾음 (Linux native 모드용)"""
    # 1. Go build output (extensions/docker/saba-docker-io/saba-docker-io)
    ext_dir = os.path.dirname(os.path.abspath(__file__))
    candidate = os.path.join(ext_dir, "saba-docker-io", "saba-docker-io")
    if os.path.isfile(candidate):
        return candidate
    # 2. PATH에서 검색
    import shutil
    found = shutil.which("saba-docker-io")
    if found:
        return found
    return None


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


def _parse_compose_ps(stdout: str) -> list:
    """docker compose ps --format json 출력을 파싱.

    Docker Compose 버전에 따라 출력 형식이 다름:
      - v2.17+: JSON-lines (한 줄에 JSON 오브젝트 하나)
      - v2.0~v2.16: JSON 배열 (한 줄짜리 or 여러 줄)
    둘 다 처리하며 항상 list[dict]를 반환.
    """
    text = stdout.strip()
    if not text:
        return []

    # 먼저 전체를 JSON 파싱 시도 (배열 형식 대응)
    try:
        parsed = json.loads(text)
        if isinstance(parsed, list):
            return [c for c in parsed if isinstance(c, dict)]
        if isinstance(parsed, dict):
            return [parsed]
    except json.JSONDecodeError:
        pass

    # JSON-lines 형식: 줄별 파싱
    containers = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
            if isinstance(obj, dict):
                containers.append(obj)
            elif isinstance(obj, list):
                containers.extend(c for c in obj if isinstance(c, dict))
        except json.JSONDecodeError:
            pass
    return containers


def _compose_cmd(instance_dir: str, compose_file: str = "docker-compose.yml") -> list:
    """compose 명령의 기본 부분 구성
    
    인스턴스별 격리는 cwd(=instance_dir)로 자연스럽게 보장됨.
    각 instance_dir이 고유 UUID 디렉토리이므로 Docker Compose 기본
    프로젝트명(=디렉토리명)이 인스턴스별로 다름.
    """
    base = _compose_cli()
    if _is_wsl2_mode():
        base += ["-f", compose_file]
    else:
        base += ["-f", os.path.join(instance_dir, compose_file)]
    return base


def _instance_container_name(config: dict) -> str:
    """config에서 인스턴스별 고유 container_name 도출.
    
    config에 module + instance_id가 있으면 _container_name() 사용,
    없으면 instance_dir의 디렉토리명 폴백.
    """
    module_name = config.get("module", config.get("module_name", ""))
    instance_id = config.get("instance_id", "")
    if module_name and instance_id:
        return _container_name(module_name, instance_id)
    # 폴백: instance_dir의 마지막 디렉토리명
    instance_dir = config.get("instance_dir", "")
    return os.path.basename(instance_dir.rstrip("/\\")) if instance_dir else ""


def _container_name(module_name: str, instance_id: str) -> str:
    """saba-{module}-{instance_id[:8]} 규칙"""
    return f"saba-{module_name}-{instance_id[:8]}"


def _progress(percent: int = None, message: str = None,
              step: int = None, total: int = None, label: str = None,
              steps: list = None):
    """PROGRESS 프로토콜로 진행률 보고
    
    step/total/label/steps는 Rust ExtensionProgress 구조체의 확장 필드로,
    GUI의 프로비저닝 단계 표시에 사용됩니다.
    """
    info = {}
    if percent is not None:
        info["percent"] = percent
    if message is not None:
        info["message"] = message
    if step is not None:
        info["step"] = step
    if total is not None:
        info["total"] = total
    if label is not None:
        info["label"] = label
    if steps is not None:
        info["steps"] = steps
    sys.stderr.write(f"PROGRESS:{json.dumps(info)}\n")
    sys.stderr.flush()


# ═══════════════════════════════════════════════════
#  Hook 함수들 — Rust DockerComposeManager 대응
# ═══════════════════════════════════════════════════

def start(config: dict) -> dict:
    """server.pre_start — docker compose up -d
    
    Rust: DockerComposeManager::start() (D4)
    """
    # Docker 데몬이 꺼져 있으면 자동 시작
    daemon_err = _ensure_docker_daemon()
    if daemon_err is not None:
        return daemon_err

    instance_dir = config["instance_dir"]
    compose_file = "docker-compose.yml"
    compose_path = os.path.join(instance_dir, compose_file)

    if not os.path.exists(compose_path):
        return {"handled": True, "success": False, "error": f"No {compose_file} found in {instance_dir}"}

    # --force-recreate: compose.yml 변경이 반영되도록 컨테이너를 항상 재생성
    # stop()은 compose stop(컨테이너 유지)이므로, 설정 변경 후에도 이전 컨테이너를 재사용하는 문제 방지
    cmd = _compose_cmd(instance_dir, compose_file) + ["up", "-d", "--force-recreate"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=300)

    if not success:
        return {"handled": True, "success": False, "error": f"Docker Compose up failed: {stderr or stdout}"}

    # 컨테이너가 실제로 살아있는지 확인 (up -d는 즉시 종료해도 성공 반환)
    import time
    time.sleep(3)  # 컨테이너 초기화 대기
    check_cmd = _compose_cmd(instance_dir, compose_file) + ["ps", "--format", "json", "-a"]
    check_ok, check_out, _ = _run_cmd(check_cmd, cwd=instance_dir, timeout=15)

    container_healthy = False
    container_state = "unknown"
    if check_ok:
        for c in _parse_compose_ps(check_out):
            container_state = c.get("State", "unknown")
            if container_state == "running":
                container_healthy = True
            break

    # 초기 로그 수집 (문제 진단용)
    log_cmd = _compose_cmd(instance_dir, compose_file) + ["logs", "--tail", "30"]
    _, initial_logs, _ = _run_cmd(log_cmd, cwd=instance_dir, timeout=10)

    if not container_healthy:
        return {
            "handled": True,
            "success": False,
            "error": f"Container started but is not running (state: {container_state}). Check logs for details.",
            "container_state": container_state,
            "initial_logs": initial_logs.strip() if initial_logs else "",
        }

    # log_follower 정보 구성 — 코어가 로그 스트리머 또는 interactive bridge를 생성하도록
    # saba-docker-io 바이너리가 있으면 양방향(stdin+stdout) IO bridge 사용 → 콘솔 입력 지원
    # 없으면 기존 docker compose logs 기반 단방향(read-only) 스트리머 폴백
    container_name = _instance_container_name(config)
    io_bridge_path = f"{WSL2_DOCKER_DIR}/saba-docker-io" if _is_wsl2_mode() else _find_io_bridge()

    # WSL2 모드에서는 바이너리 존재 여부를 확인 (없으면 fallback)
    if io_bridge_path and _is_wsl2_mode():
        check_ok, _, _ = _run_cmd(["wsl", "-u", "root", "--", "test", "-x", io_bridge_path], timeout=5)
        if not check_ok:
            io_bridge_path = None

    if io_bridge_path and container_name:
        if _is_wsl2_mode():
            log_follower = {
                "program": "wsl",
                "args": ["-u", "root", "--", io_bridge_path, container_name, f"{WSL2_DOCKER_DIR}/docker"],
                "working_dir": instance_dir,
                "description": "Docker interactive IO bridge",
                "interactive": True,
            }
        else:
            log_follower = {
                "program": io_bridge_path,
                "args": [container_name, "docker"],
                "working_dir": instance_dir,
                "description": "Docker interactive IO bridge",
                "interactive": True,
            }
    else:
        # Fallback: read-only log stream (no stdin support)
        log_follower_cmd = _compose_cmd(instance_dir, compose_file) + ["logs", "--follow", "--no-color", "--tail", "100"]
        log_follower = {
            "program": log_follower_cmd[0],
            "args": log_follower_cmd[1:],
            "working_dir": instance_dir,
            "description": "Docker compose log stream",
            "strip_prefix": " | ",
        }

    return {
        "handled": True,
        "success": True,
        "message": "Docker Compose containers started",
        "stdout": stdout,
        "initial_logs": initial_logs.strip() if initial_logs else "",
        "log_follower": log_follower,
    }


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
    instance_dir = config.get("instance_dir", "")
    if not instance_dir:
        return {"handled": True, "success": False, "error": "instance_dir not provided"}
    compose_file = "docker-compose.yml"

    cmd = _compose_cmd(instance_dir, compose_file) + ["down"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=120)

    # down 실패는 치명적이지 않음
    return {"handled": True, "success": True, "message": "Docker Compose down completed"}


def status(config: dict) -> dict:
    """server.status — 컨테이너 기반 상태 확인 (인스턴스별 격리)

    docker compose ps (-p 프로젝트명) → 해당 인스턴스의 컨테이너만 조회
    docker top <container> → 해당 컨테이너 내부 프로세스만 확인
    
    반환:
      handled=True
      running: bool - 서버 프로세스가 살아있는지
      server_process_running: bool
      container_name: str or None
      status: "running" | "starting" | "stopped"
    """
    instance_dir = config["instance_dir"]
    compose_file = "docker-compose.yml"
    process_patterns = config.get("process_patterns", [])

    # docker compose ps --format json (cwd로 인스턴스 격리)
    cmd = _compose_cmd(instance_dir, compose_file) + ["ps", "--format", "json", "-a"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=30)

    # stdout 파싱 — JSON-lines 또는 JSON 배열 모두 지원
    containers = _parse_compose_ps(stdout)

    running_containers = [c for c in containers if c.get("State") == "running"]
    container_running = len(running_containers) > 0
    # 실행 중인 컨테이너 우선, 없으면 첫 번째
    target = running_containers[0] if running_containers else (containers[0] if containers else None)
    container_name = (target.get("Name") or target.get("Names")) if target else None

    # ── 컨테이너가 실행 중이 아니면 stopped ──
    if not container_running:
        return {
            "handled": True,
            "running": False,
            "server_process_running": False,
            "container_name": container_name,
            "status": "stopped",
        }

    # ── docker top으로 해당 컨테이너 내부 프로세스만 확인 (인스턴스 격리) ──
    server_proc_running = True  # default if no patterns
    matched_process = None

    if process_patterns and container_name:
        server_proc_running, matched_process = _check_docker_top_process(container_name, process_patterns)

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
    """docker top으로 해당 컨테이너 내부 프로세스만 확인 (인스턴스 격리)

    WSL2/비-WSL2 모두 `docker top <container>` 사용.
    _docker_cli()가 WSL2에서는 자동으로 `wsl -u root -- /opt/.../docker` prefix를 붙임.
    
    Returns: (running: bool, matched_pattern: str | None)
    """
    return _check_docker_top_process(container_name, process_patterns)


def _check_docker_top_process(container_name: str, process_patterns: list) -> tuple:
    """docker top으로 서버 프로세스 존재 확인 (컨테이너 내부만 검사)"""
    docker = _docker_cli()
    cmd = docker + ["top", container_name]
    success, stdout, stderr = _run_cmd(cmd, timeout=15)

    if not success:
        return False, None

    lines = stdout.splitlines()[1:]
    for line in lines:
        line_lower = line.lower()
        for pattern in process_patterns:
            if pattern.lower() in line_lower:
                return True, pattern

    return False, None


def _check_wsl_process(process_patterns: list) -> tuple:
    """(레거시 폴백) WSL2 전역 프로세스 검색 — 인스턴스 격리 없음.
    
    container_name이 없는 경우에만 사용. 가능하면 _check_docker_top_process를 사용.
    Returns: (running: bool, matched_pattern: str | None)
    """
    cmd = ["wsl", "-u", "root", "--", "ps", "-eo", "args", "--no-headers"]
    success, stdout, stderr = _run_cmd(cmd, timeout=10)
    if not success:
        return False, None

    for line in stdout.splitlines():
        line_lower = line.strip().lower()
        for pattern in process_patterns:
            if pattern.lower() in line_lower:
                return True, pattern
    return False, None


def _get_container_stats(container_name: str) -> dict | None:
    """docker stats로 해당 컨테이너의 리소스 사용량 조회 (인스턴스 격리)
    
    `docker stats <container_name> --no-stream --format` 사용.
    Returns: {"memory_usage": str, "memory_percent": float, "cpu_percent": float} or None
    """
    docker = _docker_cli()
    cmd = docker + ["stats", container_name, "--no-stream",
                     "--format", "{{.MemUsage}}@@{{.MemPerc}}@@{{.CPUPerc}}"]
    success, stdout, stderr = _run_cmd(cmd, timeout=15)
    if not success:
        return None

    line = stdout.strip()
    if not line:
        return None

    parts = line.split("@@")
    if len(parts) < 3:
        return None

    try:
        mem_usage = parts[0].strip()  # e.g. "803MiB / 16GiB"
        mem_perc_str = parts[1].strip().rstrip("%")
        cpu_perc_str = parts[2].strip().rstrip("%")
        return {
            "memory_usage": mem_usage.split("/")[0].strip() if "/" in mem_usage else mem_usage,
            "memory_percent": float(mem_perc_str),
            "cpu_percent": float(cpu_perc_str),
        }
    except (ValueError, IndexError):
        return None


def _get_wsl_total_memory_mb() -> float | None:
    """WSL2 전체 메모리(MB) 조회 — /proc/meminfo에서 MemTotal 읽기"""
    cmd = ["wsl", "-u", "root", "--", "grep", "MemTotal", "/proc/meminfo"]
    success, stdout, _ = _run_cmd(cmd, timeout=5)
    if not success:
        return None
    # "MemTotal:       16384000 kB"
    parts = stdout.strip().split()
    if len(parts) >= 2:
        try:
            return int(parts[1]) / 1024.0  # kB → MB
        except ValueError:
            pass
    return None


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
        containers = _parse_compose_ps(stdout)
        running = [c for c in containers if c.get("State") == "running"]
        target = running[0] if running else (containers[0] if containers else None)
        if target:
            container_name = target.get("Name") or target.get("Names")

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
    
    인스턴스별 격리: docker compose ps (cwd=instance_dir) → 해당 컨테이너만 조회
    리소스: docker stats <container_name> → 해당 컨테이너만 통계
    """
    ext_data = config.get("extension_data", {})
    instance_dir = config.get("instance_dir", "")
    process_patterns = config.get("process_patterns", [])
    compose_file = "docker-compose.yml"
    compose_path = os.path.join(instance_dir, compose_file) if instance_dir else ""

    base_fields = {
        "extension_id": "docker",
        "docker_enabled": ext_data.get("docker_enabled", False),
        "docker_cpu_limit": ext_data.get("docker_cpu_limit"),
        "docker_memory_limit": ext_data.get("docker_memory_limit"),
    }

    # compose 파일이 없으면 Docker 모드가 아님
    if not instance_dir or not os.path.exists(compose_path):
        return {"handled": False, **base_fields}

    # ── docker compose ps (cwd로 인스턴스 격리) ──
    cmd = _compose_cmd(instance_dir, compose_file) + ["ps", "--format", "json", "-a"]
    success, stdout, stderr = _run_cmd(cmd, cwd=instance_dir, timeout=15)

    if not success:
        return {"handled": True, "status": "stopped", **base_fields}

    containers = _parse_compose_ps(stdout)

    running_containers = [c for c in containers if c.get("State") == "running"]
    container_running = len(running_containers) > 0
    # 실행 중인 컨테이너 이름 우선 추출
    target = running_containers[0] if running_containers else (containers[0] if containers else None)
    container_name = (target.get("Name") or target.get("Names")) if target else None

    if not container_running:
        return {"handled": True, "status": "stopped", **base_fields}

    # ── docker top으로 서버 프로세스 확인 (인스턴스 격리) ──
    final_status = "running"
    if process_patterns and container_name:
        proc_running, _ = _check_docker_top_process(container_name, process_patterns)
        if not proc_running:
            final_status = "starting"

    # ── docker stats로 해당 컨테이너만 리소스 통계 조회 ──
    mem_usage = None
    mem_perc = None
    cpu_perc = None
    if container_name:
        stats = _get_container_stats(container_name)
        if stats:
            mem_usage = stats["memory_usage"]
            mem_perc = stats["memory_percent"]
            cpu_perc = stats["cpu_percent"]

    return {
        "handled": True,
        "status": final_status,
        "memory_usage": mem_usage,
        "memory_percent": mem_perc,
        "cpu_percent": cpu_perc,
        **base_fields,
    }


def _parse_memory_to_mb(value: str) -> float | None:
    """메모리 문자열을 MB로 변환 (예: '2G' → 2048, '512M' → 512, '1024' → 1024)"""
    value = value.strip().upper()
    try:
        if value.endswith("G"):
            return float(value[:-1]) * 1024.0
        elif value.endswith("M"):
            return float(value[:-1])
        elif value.endswith("K"):
            return float(value[:-1]) / 1024.0
        else:
            return float(value)
    except (ValueError, IndexError):
        return None


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
    ext_data = config.get("extension_data") or {}
    module_config = config.get("module_install") or {}

    # ── Step 0: Docker Engine 확인 ──
    _progress(0, "Checking Docker Engine...",
              step=0, total=3, label="docker_engine",
              steps=["docker_engine", "steamcmd", "compose"])

    import importlib.util as _ilu
    _de_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "docker_engine.py")
    _de_spec = _ilu.spec_from_file_location("docker_engine", _de_path)
    docker_engine = _ilu.module_from_spec(_de_spec)
    _de_spec.loader.exec_module(docker_engine)

    engine_result = docker_engine._plugin_ensure(config.get("docker_engine_config", {
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

    _progress(33, "Docker Engine ready", step=0, total=3, label="docker_engine")

    # ── Step 1: 서버 파일 다운로드 ──
    install_config = module_config.get("install", {})
    server_dir = os.path.join(instance_dir, "server")
    os.makedirs(server_dir, exist_ok=True)
    install_method = install_config.get("method", "")
    _install_java_version = None  # download 방식에서 감지된 Java 버전

    if install_method == "steamcmd":
        app_id = install_config.get("app_id")
        if app_id:
            _progress(40, f"Downloading server files (app {app_id})...", step=1, total=3, label="steamcmd")
            steamcmd_config = {
                "app_id": app_id,
                "install_dir": server_dir,
                "anonymous": install_config.get("anonymous", True),
                "platform": "linux" if _is_wsl2_mode() else install_config.get("platform"),
                "beta": install_config.get("beta"),
            }
            try:
                # steamcmd.py의 _plugin_install 함수 호출
                ext_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
                steamcmd_path = os.path.join(ext_dir, "steamcmd.py")
                import importlib.util
                spec = importlib.util.spec_from_file_location("steamcmd", steamcmd_path)
                if spec and spec.loader:
                    steamcmd = importlib.util.module_from_spec(spec)
                    spec.loader.exec_module(steamcmd)
                    result = steamcmd._plugin_install(steamcmd_config)
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

    elif install_method == "download":
        # 모듈 자체 install_server 호출 (Minecraft 등)
        module_name = config.get("module", "")
        _progress(35, f"Downloading server files ({module_name})...",
                  step=1, total=3, label="download")
        try:
            _root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
            lifecycle_path = os.path.join(_root, "modules", module_name, "lifecycle.py")

            if not os.path.isfile(lifecycle_path):
                return {
                    "handled": True,
                    "success": False,
                    "error": f"모듈 lifecycle을 찾을 수 없습니다: {module_name}",
                }

            import importlib.util as _ilu2
            _lc_spec = _ilu2.spec_from_file_location(f"{module_name}_lifecycle", lifecycle_path)
            lifecycle_mod = _ilu2.module_from_spec(_lc_spec)

            # lifecycle.py가 같은 디렉토리의 i18n.py 등을 임포트하므로
            # 모듈 디렉토리를 sys.path에 임시 추가
            _module_dir = os.path.dirname(lifecycle_path)
            _path_added = _module_dir not in sys.path
            if _path_added:
                sys.path.insert(0, _module_dir)
            try:
                _lc_spec.loader.exec_module(lifecycle_mod)
            finally:
                if _path_added and _module_dir in sys.path:
                    sys.path.remove(_module_dir)

            # 최신 안정 버전 자동 조회
            version = None
            if hasattr(lifecycle_mod, "list_versions"):
                _progress(38, "Fetching latest version...", step=1, total=3, label="download")
                versions_result = lifecycle_mod.list_versions({})
                if versions_result.get("success", True):
                    version = (versions_result.get("latest", {}).get("release")
                               or versions_result.get("latest_release"))

            if not version:
                return {
                    "handled": True,
                    "success": False,
                    "error": f"서버 최신 버전을 조회할 수 없습니다 ({module_name})",
                }

            _progress(42, f"Downloading v{version}...", step=1, total=3, label="download")

            install_result = lifecycle_mod.install_server({
                "version": version,
                "install_dir": server_dir,
                "accept_eula": True,
            })

            if not install_result.get("success", False):
                return {
                    "handled": True,
                    "success": False,
                    "error": f"서버 설치 실패: {install_result.get('message', 'unknown')}",
                }

            # install_result의 java_major_version을 compose 컨텍스트용으로 보존
            _install_java_version = install_result.get("java_major_version")

            _progress(60, f"Server v{version} installed", step=1, total=3, label="download")

        except Exception as e:
            import traceback
            print(traceback.format_exc(), file=sys.stderr)
            return {
                "handled": True,
                "success": False,
                "error": f"서버 다운로드 실패: {e}",
            }

    _progress(66, "Server files ready", step=1, total=3, label="server_files")

    # ── Step 2: docker-compose.yml 생성 ──
    _progress(80, "Generating docker-compose.yml...", step=2, total=3, label="compose")

    docker_section = module_config.get("container", {}) or module_config.get("docker", {})
    if not docker_section.get("image"):
        return {
            "handled": True,
            "success": False,
            "error": f"모듈에 컨테이너 이미지 설정이 없습니다 (module_install keys: {list(module_config.keys())})",
        }

    instance_data = config.get("instance", config)

    # install_server에서 받은 java_major_version으로 Docker 이미지의 {java_version} 치환
    extra_ctx = {}
    if _install_java_version:
        extra_ctx["java_version"] = str(_install_java_version)
    # fallback: 템플릿에 {java_version}이 있는데 값이 없으면 기본값 21
    if "java_version" not in extra_ctx and "{java_version}" in docker_section.get("image", ""):
        extra_ctx["java_version"] = "21"

    yaml = _generate_compose_yaml(docker_section, instance_data, extra_ctx=extra_ctx)
    compose_path = os.path.join(instance_dir, "docker-compose.yml")
    with open(compose_path, "w", encoding="utf-8") as f:
        f.write(yaml)

    _progress(100, "docker-compose.yml generated", step=2, total=3, label="compose")

    return {
        "handled": True,
        "success": True,
        "message": "Docker 프로비저닝 완료: docker-compose.yml 생성됨",
    }


def regenerate_compose(config: dict) -> dict:
    """server.settings_changed — 설정 변경 시 compose 재생성
    
    Rust 코어는 범용 module_extensions를 전달하고,
    이 함수가 자체적으로 docker 섹션을 추출하여 compose를 재생성합니다.
    """
    instance_dir = config.get("instance_dir", "")
    if not instance_dir:
        return {"handled": True, "success": False, "error": "No instance_dir"}

    # 범용 module_extensions에서 자신의 설정(docker)을 추출
    module_extensions = config.get("module_extensions", {})
    docker_section = module_extensions.get("docker", {})

    # 레거시 호환: 이전 방식의 module_config도 지원
    if not docker_section:
        module_config = config.get("module_config", {})
        docker_section = module_config.get("container", {}) or module_config.get("docker", {})

    if not docker_section.get("image"):
        return {"handled": True, "success": False, "error": "No docker config in module_extensions"}

    instance_data = config.get("instance", config)

    # {java_version} 템플릿용 — 기존 compose에서 추출하거나 기본값 사용
    extra_ctx = {}
    if "{java_version}" in docker_section.get("image", ""):
        # 모듈 설정에서 java_version 찾기를 시도 → 기본값 21
        extra_ctx["java_version"] = str(config.get("java_version", "21"))

    yaml = _generate_compose_yaml(docker_section, instance_data, extra_ctx=extra_ctx)
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

    # java_version 치환
    java_version = ctx.get("java_version")
    if java_version is not None:
        result = result.replace("{java_version}", str(java_version))
    
    # 모듈 설정에서 추가 변수
    module_settings = ctx.get("module_settings", {})
    for key, value in module_settings.items():
        result = result.replace(f"{{{key}}}", str(value))
    
    return result


def _generate_compose_yaml(docker_config: dict, instance: dict, extra_ctx: dict = None) -> str:
    """
    docker-compose.yml YAML 생성 — Rust generate_compose_yaml() 정밀 포팅
    
    docker_config: module.toml [docker] 섹션
    instance: ServerInstance 데이터 (id, name, module_name, port, ext_data 등)
    extra_ctx: 추가 템플릿 변수 (java_version 등)
    """
    ctx = {
        "instance_id": instance.get("instance_id", instance.get("id", "")),
        "instance_name": instance.get("instance_name", instance.get("name", "")),
        "module_name": instance.get("module_name") or instance.get("module", ""),
        "port": instance.get("port"),
        "rcon_port": instance.get("rcon_port"),
        "rest_port": instance.get("rest_port"),
        "rest_password": instance.get("rest_password"),
        "module_settings": instance.get("module_settings", {}),
    }

    # 추가 컨텍스트 변수 병합 (java_version 등)
    if extra_ctx:
        ctx.update(extra_ctx)

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

    # Network mode — WSL2 mirrored에서는 host 네트워크 사용
    # (bridge 네트워크 + mirrored = UDP 포워딩 불가)
    use_host_network = _is_wsl2_mode() or docker_config.get("network_mode") == "host"

    if use_host_network:
        lines.append("    network_mode: host")

    # Ports (host 네트워크에서는 ports 매핑 불필요 — Docker가 무시함)
    if not use_host_network:
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
        if entrypoint:
            # entrypoint가 있으면 single-string exec form 유지
            # (e.g., entrypoint: ["/bin/bash", "-c"], command: ["exec server.sh ..."])
            escaped = resolved.replace('"', '\\"')
            lines.append(f'    command: ["{escaped}"]')
        else:
            # entrypoint가 없으면 shell form 사용
            # Docker가 자동으로 /bin/sh -c 를 앞에 붙여줌
            lines.append(f'    command: {resolved}')

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

    # stdin 지원 (tty 사용 안 함 — ManagedProcess의 piped stdin과 호환성을 위해)
    lines.append("    stdin_open: true")

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
