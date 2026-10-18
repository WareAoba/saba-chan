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
if [[ -d saba-chan-gui/node_modules/electron ]]; then (cd saba-chan-gui && npx vite build >/dev/null); fi
if [[ ! -d "$E2E/gui/node_modules" ]]; then (cd "$E2E/gui" && npm install --no-audit --no-fund >/dev/null); fi
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
