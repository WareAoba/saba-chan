#!/bin/bash
# Discord Bot 개발 모드 실행 스크립트 (Linux/macOS)

set -e

echo "🚀 Discord Bot - Development Mode"
echo "=================================="
echo ""

# 현재 디렉터리 확인
if [ ! -f "package.json" ]; then
    echo "❌ Error: package.json not found. Run this script from the discord_bot directory."
    exit 1
fi

# .env 파일 확인
if [ ! -f ".env" ]; then
    echo "⚠️  .env file not found!"
    echo "Creating from .env.example..."
    if [ -f ".env.example" ]; then
        cp .env.example .env
        echo "✓ .env created"
        echo ""
        echo "⚠️  IMPORTANT: Set DISCORD_TOKEN in .env file"
        echo "Get token from: https://discord.com/developers/applications"
        echo ""
        exit 1
    else
        echo "❌ .env.example not found either"
        exit 1
    fi
fi

# node_modules 확인
if [ ! -d "node_modules" ]; then
    echo "📦 Installing dependencies..."
    npm install
fi

echo "📝 Configuration:"
echo "  IPC_BASE: $(grep IPC_BASE .env || echo 'not set')"
echo ""

echo "Starting Discord Bot..."
echo "Press Ctrl+C to stop"
echo ""

npm start
