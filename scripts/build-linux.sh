#!/usr/bin/env bash
set -euo pipefail

# ────────────────────────────────────────────────────────────
# Saba-chan Linux Release Build Script
#
# 배포물:
#   saba-core              - Rust IPC 데몬
#   saba-chan-cli           - Rust TUI 클라이언트
#   saba-chan-gui           - Electron GUI (AppImage / tar.gz)
#   saba-chan-updater       - 업데이터
#   locales/                - 다국어 리소스
#
# Usage:
#   ./build-linux.sh
#   ./build-linux.sh my-build
# ────────────────────────────────────────────────────────────

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
OUTPUT_DIR="${1:-release-build}"
RELEASE_DIR="$PROJECT_ROOT/$OUTPUT_DIR"
VERSION="$(grep -m1 -E '^version *=' "$PROJECT_ROOT/Cargo.toml" | cut -d'"' -f2)"
DIST_DIR="$RELEASE_DIR/saba-chan-v$VERSION"

SECONDS=0

echo "================================"
echo " Saba-chan Linux Release Build"
echo "================================"
echo ""

# --- 1. Clean ---
echo "[1/6] Cleaning..."
if [ -d "$RELEASE_DIR" ]; then
    rm -rf "$RELEASE_DIR"
fi
mkdir -p "$DIST_DIR"
echo "  [OK]"

# --- 2. Rust Workspace Build ---
echo "[2/6] Building Rust binaries (release)..."
cd "$PROJECT_ROOT"
cargo build --release --workspace

RUST_TARGETS=("saba-core" "saba-chan-cli" "saba-chan-updater")
for t in "${RUST_TARGETS[@]}"; do
    BIN="$PROJECT_ROOT/target/release/$t"
    if [ ! -f "$BIN" ]; then
        echo "  [ERROR] $t not found at $BIN" >&2
        exit 1
    fi
    SIZE=$(du -h "$BIN" | cut -f1)
    echo "  $t ($SIZE)"
done
echo "  [OK]"

# --- 3. Electron GUI Build ---
echo "[3/6] Building Electron GUI..."
cd "$PROJECT_ROOT/saba-chan-gui"

# Clean previous builds
rm -rf dist build electron-dist 2>/dev/null || true

if [ ! -d "node_modules" ]; then
    npm ci --quiet
fi

npm run build --silent
npx electron-builder --linux

echo "  [OK]"

# --- 4. Collect Artifacts ---
echo "[4/6] Collecting artifacts..."
cd "$PROJECT_ROOT"

# Rust binaries
for t in "${RUST_TARGETS[@]}"; do
    cp "target/release/$t" "$DIST_DIR/"
    chmod +x "$DIST_DIR/$t"
done

# GUI portable (AppImage)
cp "$PROJECT_ROOT/saba-chan-gui/electron-dist/"*.AppImage "$DIST_DIR/" 2>/dev/null || true
chmod +x "$DIST_DIR/"*.AppImage 2>/dev/null || true

echo "  [OK]"

# --- 5. Resources ---
echo "[5/6] Preparing resources..."

# Locales
if [ -d "$PROJECT_ROOT/locales" ]; then
    cp -r "$PROJECT_ROOT/locales" "$DIST_DIR/locales"
fi

# Shared modules (i18n.py, daemon_rcon.py)
SHARED_MODULES=("i18n.py" "daemon_rcon.py")
for mod in "${SHARED_MODULES[@]}"; do
    SRC="$PROJECT_ROOT/modules/$mod"
    if [ -f "$SRC" ]; then
        cp "$SRC" "$DIST_DIR/"
    else
        echo "  [WARN] $mod not found at $SRC"
    fi
done

echo "  [OK]"

# --- 6. Create Archives ---
echo "[6/6] Creating archives..."
cd "$RELEASE_DIR"

# Individual component archives
tar -czf saba-core-linux-x64.tar.gz     -C "$DIST_DIR" saba-core
tar -czf saba-chan-cli-linux-x64.tar.gz  -C "$DIST_DIR" saba-chan-cli
tar -czf saba-chan-updater-linux-x64.tar.gz -C "$DIST_DIR" saba-chan-updater

# GUI: electron-builder output
cp "$PROJECT_ROOT/saba-chan-gui/electron-dist/"*.AppImage "$RELEASE_DIR/" 2>/dev/null || true
cp "$PROJECT_ROOT/saba-chan-gui/electron-dist/"*.tar.gz   "$RELEASE_DIR/" 2>/dev/null || true

echo "  [OK]"

# --- Summary ---
ELAPSED=$SECONDS
MINS=$((ELAPSED / 60))
SECS=$((ELAPSED % 60))

TOTAL_SIZE=$(du -sh "$DIST_DIR" | cut -f1)

echo ""
echo "================================"
echo " BUILD COMPLETE"
echo "================================"
echo "Output: $DIST_DIR"
echo "Time:   ${MINS}m ${SECS}s"
echo ""
echo "  Contents:"
for f in "$DIST_DIR"/*; do
    NAME=$(basename "$f")
    if [ -d "$f" ]; then
        SIZE=$(du -sh "$f" | cut -f1)
        echo "    $NAME/  ($SIZE)"
    else
        SIZE=$(du -h "$f" | cut -f1)
        echo "    $NAME  ($SIZE)"
    fi
done
echo ""
echo "  Total: $TOTAL_SIZE"
echo ""

# Archives
echo "  Archives:"
ls -lh "$RELEASE_DIR"/*.tar.gz "$RELEASE_DIR"/*.AppImage 2>/dev/null | awk '{print "    " $NF " (" $5 ")"}'
echo ""
