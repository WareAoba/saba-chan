#!/usr/bin/env python3
"""
E2E 테스트용 mock ext-process.
데몬의 ext-process 관리 (start/stop/console/stdin)를 검증합니다.

- 시작 시 stdout으로 READY 메시지 출력
- stdin에서 명령을 읽어 echo back
- shutdown 명령 수신 시 graceful 종료
"""

import sys
import json
import time
import threading

def main():
    print("E2E_MOCK_EXT_STARTED", flush=True)
    print(f"PID={__import__('os').getpid()}", flush=True)
    print("READY", flush=True)

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            msg = json.loads(line)
            if msg.get("type") == "shutdown":
                print("SHUTDOWN_ACK: graceful exit", flush=True)
                break
            elif msg.get("type") == "ping":
                print(json.dumps({"type": "pong", "timestamp": time.time()}), flush=True)
            elif msg.get("type") == "echo":
                print(json.dumps({"type": "echo_reply", "data": msg.get("data", "")}), flush=True)
            else:
                print(json.dumps({"type": "unknown_command", "received": msg}), flush=True)
        except json.JSONDecodeError:
            # Plain text commands
            if line.lower() == "status":
                print("STATUS: running", flush=True)
            elif line.lower() == "quit":
                print("QUIT_ACK", flush=True)
                break
            else:
                print(f"ECHO: {line}", flush=True)

    print("E2E_MOCK_EXT_EXITING", flush=True)
    sys.exit(0)


if __name__ == "__main__":
    main()
