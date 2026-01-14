#!/bin/bash
# Electron GUI 개발 모드 실행 스크립트 (Linux/macOS)

set -e

echo "🚀 Electron GUI - Development Mode"
echo "===================================="
echo ""

# 현재 디렉터리 확인
if [ ! -f "package.json" ]; then
    echo "❌ Error: package.json not found. Run this script from the electron_gui directory."
    exit 1
fi

# node_modules 확인
if [ ! -d "node_modules" ]; then
    echo "📦 Installing dependencies..."
    npm install
fi

echo "🔗 IPC Configuration:"
echo "  IPC_BASE: http://localhost:57474"
echo ""

echo "📱 Ports:"
echo "  React Dev Server: http://localhost:3000"
echo "  Electron App: (desktop window)"
echo ""

echo "Starting Electron GUI..."
echo "Press Ctrl+C to stop"
echo ""

npm start
