#!/usr/bin/env bash
set -eu
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "${SABA_PYTHON:-python3}" "$SCRIPT_DIR/run-tests.py" "$@"
