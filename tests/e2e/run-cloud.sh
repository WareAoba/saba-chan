#!/usr/bin/env bash
# saba-chan E2E를 리눅스(클라우드)에서 한 번에. 외부망이 필요한 시험(network·steam)은 빼고 돈다.
# 사용: tests/e2e/run-cloud.sh [-k 식] [--slow]
set -euo pipefail
E2E="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$E2E/../.." && pwd)"
MARK="not network and not steam and not slow"
ARGS=()
for a in "$@"; do
  if [[ "$a" == "--slow" ]]; then MARK="not network and not steam"; else ARGS+=("$a"); fi
done

cd "$REPO"
cargo build -p saba-core -p saba-chan-cli -p saba-runner
CARGO_TARGET_DIR="$REPO/target/e2e-mock" cargo build -p saba-core --features mock-relay
# GUI: 화면(build/)이 실행 파일에 들어가므로 vite build 다음 Tauri 빌드. tauri-driver·WebKitWebDriver가 없으면 GUI 시험은 건너뛴다
if [[ -d saba-chan-gui/node_modules/.bin ]] && command -v tauri-driver >/dev/null; then
  (cd saba-chan-gui && npx vite build >/dev/null) && cargo build -p saba-chan-gui
fi
python3 -m pip install -q -r "$E2E/requirements.txt"

STAMP="$(date +%Y%m%d-%H%M%S)"
ROOT="${SABA_E2E_ROOT:-${TMPDIR:-/tmp}/saba-e2e}"
mkdir -p "$ROOT"
cd "$E2E"
set +e
python3 -m pytest -p no:cacheprovider -m "$MARK" --junitxml "$ROOT/junit-$STAMP.xml" "${ARGS[@]}"
CODE=$?
python3 -m harness.report "$ROOT/junit-$STAMP.xml" "$ROOT/report-$STAMP.md" "saba-chan E2E (cloud $STAMP)"
echo "결과: $ROOT/report-$STAMP.md"
exit $CODE
