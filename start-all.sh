#!/bin/bash
# All-in-one script to run the entire development environment
# Usage: ./start-all.sh [option]
# Options:
#   daemon    - Run only Core Daemon
#   bot       - Run only Discord Bot
#   gui       - Run only Electron GUI
#   all       - Run all components (default)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODE="${1:-all}"

echo ""
echo "╔════════════════════════════════════════════════════════════╗"
echo "║  Game Server Management Platform - Development Mode       ║"
echo "║  모듈형 통합 디스코드 봇 & 게임 서버 관리 플랫폼            ║"
echo "╚════════════════════════════════════════════════════════════╝"
echo ""

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

cleanup() {
    echo ""
    echo -e "${YELLOW}Shutting down all processes...${NC}"
    # Kill all background jobs
    jobs -p | xargs -r kill 2>/dev/null || true
    echo -e "${GREEN}✓ All processes stopped${NC}"
    exit 0
}

trap cleanup SIGINT SIGTERM

run_daemon() {
    echo -e "${BLUE}[1/3] Starting Core Daemon (Rust)${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    
    if [ ! -f "$SCRIPT_DIR/Cargo.toml" ]; then
        echo -e "${RED}✗ Error: Cargo.toml not found in $SCRIPT_DIR${NC}"
        return 1
    fi
    
    if [ ! -d "$SCRIPT_DIR/modules" ]; then
        mkdir -p "$SCRIPT_DIR/modules"
    fi
    
    export RUST_LOG=info
    cd "$SCRIPT_DIR"
    
    if ! command -v cargo &> /dev/null; then
        echo -e "${RED}✗ Rust/cargo not found. Install from https://rustup.rs${NC}"
        return 1
    fi
    
    cargo build --quiet 2>/dev/null || true
    cargo run &
    DAEMON_PID=$!
    echo -e "${GREEN}✓ Core Daemon started (PID: $DAEMON_PID)${NC}"
    echo ""
    return 0
}

run_bot() {
    echo -e "${BLUE}[2/3] Starting Discord Bot (Node.js)${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    
    BOT_DIR="$SCRIPT_DIR/discord_bot"
    
    if [ ! -f "$BOT_DIR/package.json" ]; then
        echo -e "${RED}✗ Error: discord_bot/package.json not found${NC}"
        return 1
    fi
    
    cd "$BOT_DIR"
    
    # Check .env
    if [ ! -f ".env" ]; then
        if [ -f ".env.example" ]; then
            cp .env.example .env
            echo -e "${YELLOW}⚠ .env created from .env.example${NC}"
            echo -e "${YELLOW}  ⚠ Please set DISCORD_TOKEN in .env${NC}"
            echo -e "${YELLOW}  Get token from: https://discord.com/developers/applications${NC}"
        else
            echo -e "${RED}✗ .env file not found${NC}"
            return 1
        fi
    fi
    
    if ! command -v npm &> /dev/null; then
        echo -e "${RED}✗ Node.js/npm not found${NC}"
        return 1
    fi
    
    # Install dependencies if needed
    if [ ! -d "node_modules" ]; then
        echo "Installing npm dependencies..."
        npm install --silent
    fi
    
    npm start &
    BOT_PID=$!
    echo -e "${GREEN}✓ Discord Bot started (PID: $BOT_PID)${NC}"
    echo ""
    return 0
}

run_gui() {
    echo -e "${BLUE}[3/3] Starting Electron GUI (React)${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    
    GUI_DIR="$SCRIPT_DIR/electron_gui"
    
    if [ ! -f "$GUI_DIR/package.json" ]; then
        echo -e "${RED}✗ Error: electron_gui/package.json not found${NC}"
        return 1
    fi
    
    cd "$GUI_DIR"
    
    if ! command -v npm &> /dev/null; then
        echo -e "${RED}✗ Node.js/npm not found${NC}"
        return 1
    fi
    
    # Install dependencies if needed
    if [ ! -d "node_modules" ]; then
        echo "Installing npm dependencies..."
        npm install --silent
    fi
    
    npm start &
    GUI_PID=$!
    echo -e "${GREEN}✓ Electron GUI started (PID: $GUI_PID)${NC}"
    echo ""
    return 0
}

echo "Mode: $MODE"
echo ""

case $MODE in
    daemon)
        run_daemon || exit 1
        ;;
    bot)
        run_bot || exit 1
        ;;
    gui)
        run_gui || exit 1
        ;;
    all)
        run_daemon || exit 1
        sleep 2
        run_bot || exit 1
        sleep 1
        run_gui || exit 1
        ;;
    *)
        echo -e "${RED}Unknown mode: $MODE${NC}"
        echo "Usage: $0 [daemon|bot|gui|all]"
        exit 1
        ;;
esac

echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║           All services are running!                       ║${NC}"
echo -e "${GREEN}╠════════════════════════════════════════════════════════════╣${NC}"
echo -e "${GREEN}║  Core Daemon:   http://localhost:57474                   ║${NC}"
echo -e "${GREEN}║  React Dev:     http://localhost:3000                    ║${NC}"
echo -e "${GREEN}║  Discord Bot:   Connected (check terminal)               ║${NC}"
echo -e "${GREEN}╠════════════════════════════════════════════════════════════╣${NC}"
echo -e "${GREEN}║  Press Ctrl+C to stop all services                       ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
echo ""

# Keep script running
wait
