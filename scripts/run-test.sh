#!/usr/bin/env bash
# Unified test runner for saba-chan (Unix-like)

set -u

NO_INSTALL=0
VERBOSE=0

for arg in "$@"; do
  case "$arg" in
    --no-install) NO_INSTALL=1 ;;
    --verbose) VERBOSE=1 ;;
    *) ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ "$(basename "$SCRIPT_DIR")" == "scripts" ]]; then
  REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
else
  REPO_ROOT="$SCRIPT_DIR"
fi
cd "$REPO_ROOT" || exit 1

RESULT_NAMES=()
RESULT_STATUS=()
RESULT_CODES=()
RESULT_DURATIONS=()

print_section() {
  echo
  echo "=================================================="
  echo " $1"
  echo "=================================================="
}

ensure_npm_dependencies() {
  local dir="$1"
  local label="$2"

  if [[ "$NO_INSTALL" -eq 1 ]]; then
    return 0
  fi

  if [[ ! -d "$dir/node_modules" ]]; then
    echo "[$label] node_modules not found. Installing dependencies..."
    (cd "$dir" && npm install --silent)
    local code=$?
    if [[ $code -ne 0 ]]; then
      echo "[$label] npm install failed (exit $code)"
      return $code
    fi
  fi

  return 0
}

run_step() {
  local name="$1"
  local workdir="$2"
  local cmd="$3"

  echo "[$name] $cmd"

  local start end duration code
  start=$(date +%s)

  if [[ "$VERBOSE" -eq 1 ]]; then
    (cd "$workdir" && bash -lc "$cmd")
  else
    (cd "$workdir" && bash -lc "$cmd")
  fi

  code=$?
  end=$(date +%s)
  duration=$((end - start))

  RESULT_NAMES+=("$name")
  RESULT_CODES+=("$code")
  RESULT_DURATIONS+=("$duration")

  if [[ $code -eq 0 ]]; then
    RESULT_STATUS+=("PASS")
    echo "[$name] PASS (${duration}s)"
  else
    RESULT_STATUS+=("FAIL")
    echo "[$name] FAIL (exit $code, ${duration}s)"
  fi
}

print_section "Saba-chan Unified Test Runner"
echo "Repository: $REPO_ROOT"
echo "NoInstall : $NO_INSTALL"
echo "Verbose   : $VERBOSE"

ensure_npm_dependencies "$REPO_ROOT/saba-chan-gui" "GUI" || exit 1
ensure_npm_dependencies "$REPO_ROOT/discord_bot" "Discord" || exit 1

print_section "Running Test Suites"
run_step "Rust-Daemon-Integration" "$REPO_ROOT" "cargo test --test daemon_integration"
run_step "Rust-Updater-Integration" "$REPO_ROOT" "cargo test --test updater_integration"
run_step "GUI-Vitest" "$REPO_ROOT/saba-chan-gui" "npm test -- --run"
run_step "Discord-Jest" "$REPO_ROOT/discord_bot" "npm test"

print_section "Summary"
printf "%-28s %-6s %-7s %s\n" "Suite" "Status" "Exit" "Sec"
printf "%-28s %-6s %-7s %s\n" "-----" "------" "----" "---"

failed_count=0
for i in "${!RESULT_NAMES[@]}"; do
  printf "%-28s %-6s %-7s %s\n" \
    "${RESULT_NAMES[$i]}" \
    "${RESULT_STATUS[$i]}" \
    "${RESULT_CODES[$i]}" \
    "${RESULT_DURATIONS[$i]}"

  if [[ "${RESULT_STATUS[$i]}" == "FAIL" ]]; then
    failed_count=$((failed_count + 1))
  fi
done

if [[ $failed_count -gt 0 ]]; then
  echo
  echo "Failed suites:"
  for i in "${!RESULT_NAMES[@]}"; do
    if [[ "${RESULT_STATUS[$i]}" == "FAIL" ]]; then
      echo " - ${RESULT_NAMES[$i]} (exit ${RESULT_CODES[$i]})"
    fi
  done
  exit 1
fi

echo
echo "All test suites passed."
exit 0
