#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════
# Saba-chan Unified Test Runner (Unix / macOS / WSL)
# ═══════════════════════════════════════════════════════════
# 전체 코드베이스 테스트를 한 번에 실행:
#   1. Rust 데몬 통합 테스트
#   2. Rust 업데이터 통합 테스트
#   3. 릴레이 서버 E2E (Vitest + PostgreSQL)
#   4. GUI E2E (Vitest + jsdom)
#   5. Discord 봇 통합 (Jest)
#
# 사용법:
#   ./run-test.sh                    # 전체
#   ./run-test.sh --suite gui        # 특정 스위트만
#   ./run-test.sh --no-install       # npm install 건너뛰기
#   ./run-test.sh --verbose          # 상세 출력

set -u

NO_INSTALL=0
VERBOSE=0
SUITE="all"

for arg in "$@"; do
  case "$arg" in
    --no-install) NO_INSTALL=1 ;;
    --verbose)    VERBOSE=1 ;;
    --suite)      shift_next=1 ;;
    *)
      if [[ "${shift_next:-0}" == "1" ]]; then
        SUITE="$arg"; shift_next=0
      fi
      ;;
  esac
done

# ── 경로 해석 ──────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ "$(basename "$SCRIPT_DIR")" == "scripts" ]]; then
  REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
else
  REPO_ROOT="$SCRIPT_DIR"
fi
cd "$REPO_ROOT" || exit 1

# server-chan (형제 또는 하위)
RELAY_DIR=""
for candidate in \
    "$(cd "$REPO_ROOT/.." && pwd)/server-chan/relay-server" \
    "$REPO_ROOT/server-chan/relay-server"; do
  if [[ -d "$candidate" ]]; then RELAY_DIR="$candidate"; break; fi
done

# ── 결과 배열 ──────────────────────────────────────────────
RESULT_NAMES=()
RESULT_STATUS=()
RESULT_CODES=()
RESULT_DURATIONS=()

# ── 색상 ────────────────────────────────────────────────────
GREEN='\033[0;32m'; RED='\033[0;31m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; GRAY='\033[0;37m'; NC='\033[0m'

print_section() {
  echo
  echo -e "${CYAN}==========================================================${NC}"
  echo -e "${CYAN} $1${NC}"
  echo -e "${CYAN}==========================================================${NC}"
}

ensure_npm() {
  local dir="$1" label="$2"
  [[ "$NO_INSTALL" -eq 1 ]] && return 0
  if [[ ! -d "$dir/node_modules" ]]; then
    echo -e "[${label}] Installing dependencies..."
    (cd "$dir" && npm install --silent) || { echo "[$label] npm install failed"; return 1; }
  fi
}

run_step() {
  local name="$1" workdir="$2" cmd="$3"
  echo -e "\n${YELLOW}[${name}]${NC} $cmd"

  local start end duration code
  start=$(date +%s)
  (cd "$workdir" && bash -lc "$cmd")
  code=$?
  end=$(date +%s)
  duration=$((end - start))

  RESULT_NAMES+=("$name")
  RESULT_CODES+=("$code")
  RESULT_DURATIONS+=("$duration")

  if [[ $code -eq 0 ]]; then
    RESULT_STATUS+=("PASS")
    echo -e "  -> ${GREEN}PASS${NC} (${duration}s)"
  else
    RESULT_STATUS+=("FAIL")
    echo -e "  -> ${RED}FAIL${NC} (exit $code, ${duration}s)"
  fi
}

skip_step() {
  local name="$1" reason="$2"
  echo -e "\n${YELLOW}[${name}]${NC} SKIP: $reason"
  RESULT_NAMES+=("$name"); RESULT_STATUS+=("SKIP"); RESULT_CODES+=(0); RESULT_DURATIONS+=(0)
}

# ── 헤더 ────────────────────────────────────────────────────
print_section "Saba-chan Unified Test Runner"
echo -e "${GRAY}Repository  : $REPO_ROOT${NC}"
echo -e "${GRAY}RelayServer : ${RELAY_DIR:-(not found)}${NC}"
echo -e "${GRAY}Suite       : $SUITE${NC}"
echo -e "${GRAY}NoInstall   : $NO_INSTALL${NC}"

# ── 의존성 ─────────────────────────────────────────────────
if [[ "$SUITE" == "all" || "$SUITE" == "gui" ]]; then
  ensure_npm "$REPO_ROOT/saba-chan-gui" "GUI" || exit 1
fi
if [[ "$SUITE" == "all" || "$SUITE" == "discord" ]]; then
  ensure_npm "$REPO_ROOT/discord_bot" "Discord" || exit 1
fi
if [[ "$SUITE" == "all" || "$SUITE" == "relay" ]]; then
  if [[ -n "$RELAY_DIR" ]]; then ensure_npm "$RELAY_DIR" "Relay" || exit 1; fi
fi

# ── 테스트 실행 ─────────────────────────────────────────────
print_section "Running Test Suites"

# 1) Rust
if [[ "$SUITE" == "all" || "$SUITE" == "rust" ]]; then
  run_step "Rust-Daemon"  "$REPO_ROOT" "cargo test --test daemon_integration"
  run_step "Rust-Updater" "$REPO_ROOT" "cargo test --test updater_integration"
fi

# 2) Relay Server E2E
if [[ "$SUITE" == "all" || "$SUITE" == "relay" ]]; then
  if [[ -n "$RELAY_DIR" ]]; then
    run_step "Relay-E2E" "$RELAY_DIR" "npx vitest run"
  else
    skip_step "Relay-E2E" "relay-server directory not found"
  fi
fi

# 3) GUI E2E
if [[ "$SUITE" == "all" || "$SUITE" == "gui" ]]; then
  run_step "GUI-E2E" "$REPO_ROOT/saba-chan-gui" "npx vitest run"
fi

# 4) Discord Bot Integration
if [[ "$SUITE" == "all" || "$SUITE" == "discord" ]]; then
  run_step "Discord-Integration" "$REPO_ROOT/discord_bot" "npm test"
fi

# ── 요약 ────────────────────────────────────────────────────
print_section "Summary"
printf "%-28s %-6s %-7s %s\n" "Suite" "Status" "Exit" "Sec"
printf "%-28s %-6s %-7s %s\n" "-----" "------" "----" "---"

failed_count=0
for i in "${!RESULT_NAMES[@]}"; do
  status="${RESULT_STATUS[$i]}"
  color="$NC"
  [[ "$status" == "PASS" ]] && color="$GREEN"
  [[ "$status" == "FAIL" ]] && color="$RED"
  [[ "$status" == "SKIP" ]] && color="$YELLOW"

  printf "${color}%-28s %-6s %-7s %s${NC}\n" \
    "${RESULT_NAMES[$i]}" "$status" "${RESULT_CODES[$i]}" "${RESULT_DURATIONS[$i]}"

  [[ "$status" == "FAIL" ]] && failed_count=$((failed_count + 1))
done

echo
if [[ $failed_count -gt 0 ]]; then
  echo -e "${RED}Failed suites:${NC}"
  for i in "${!RESULT_NAMES[@]}"; do
    [[ "${RESULT_STATUS[$i]}" == "FAIL" ]] && echo -e "  ${RED}- ${RESULT_NAMES[$i]} (exit ${RESULT_CODES[$i]})${NC}"
  done
  exit 1
fi

echo -e "${GREEN}All test suites passed.${NC}"
exit 0
