#!/bin/bash
# Core Daemon 개발 모드 실행 스크립트 (Linux/macOS)

set -e

echo "🚀 Game Server Management Platform - Development Mode"
echo "=================================================="
echo ""

# 현재 디렉터리 확인
if [ ! -f "Cargo.toml" ]; then
    echo "❌ Error: Cargo.toml not found. Run this script from the Bot directory."
    exit 1
fi

echo "1️⃣  Core Daemon (Rust)"
echo "Starting with debug logging..."
echo ""

# 로그 레벨 설정
export RUST_LOG=${RUST_LOG:-debug}

# modules 디렉터리가 없으면 생성
if [ ! -d "modules" ]; then
    echo "📁 Creating modules directory..."
    mkdir -p modules
fi

# 모듈 발견 확인
echo "📦 Available modules:"
if [ -d "modules" ]; then
    for module_dir in modules/*/; do
        if [ -f "${module_dir}module.toml" ]; then
            module_name=$(basename "$module_dir")
            echo "  ✓ $module_name"
        fi
    done
else
    echo "  (none - add modules to modules/ directory)"
fi

echo ""
echo "Starting Core Daemon..."
echo "Press Ctrl+C to stop"
echo ""

cargo run
