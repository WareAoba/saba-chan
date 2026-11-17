#!/usr/bin/env bash
# 모듈·익스텐션 저장소 검사를 이 데몬의 공용 런타임(modules/*.py)으로 돌린다.
# ci.yml(공용 런타임을 바꾼 PR)과 publish-addons.yml(발행 관문)이 같이 쓴다.
# 사용: addon-tests.sh <공용 런타임 폴더> <모듈 저장소> <익스텐션 저장소>
set -euo pipefail
# Windows Git Bash에서는 Python이 읽는 D:/ 꼴 경로로 (pwd -W)
abs() { (cd "$1" && { pwd -W 2>/dev/null || pwd; }); }
runtime="$(abs "$1")"
modules="$(abs "$2")"
extensions="$(abs "$3")"

python -m pip install --quiet pytest ruff -r "$modules/requirements-dev.txt"

echo "::group::Extensions (ruff 치명 규칙, pytest)"
(cd "$extensions" && python -m ruff check . --select E9,F63,F7,F82 && SABA_RUNTIME_DIR="$runtime" python -m pytest -q)
echo "::endgroup::"

echo "::group::Modules (conformance, pytest)"
(cd "$modules" && export SABA_RUNTIME_DIR="$runtime" SABA_EXTENSIONS_DIR="$extensions" &&
  python -m pytest conformance -q --shared-libs "$runtime" && python -m pytest -q)
echo "::endgroup::"
