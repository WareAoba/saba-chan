@echo off
REM All-in-one script to run the entire development environment
REM Usage: start-all.bat [option]
REM Options:
REM   daemon    - Run only Core Daemon
REM   bot       - Run only Discord Bot
REM   gui       - Run only Electron GUI
REM   all       - Run all components (default)

setlocal enabledelayedexpansion

set "SCRIPT_DIR=%~dp0"
set "MODE=%1"
if "%MODE%"=="" set "MODE=all"

cls
echo.
echo  ╔════════════════════════════════════════════════════════════╗
echo  ║  Game Server Management Platform - Development Mode       ║
echo  ║  모듈형 통합 디스코드 봇 ^& 게임 서버 관리 플랫폼            ║
echo  ╚════════════════════════════════════════════════════════════╝
echo.

echo  Mode: %MODE%
echo.

REM Create modules directory if it doesn't exist
if not exist "%SCRIPT_DIR%modules" mkdir "%SCRIPT_DIR%modules"

goto process_%MODE%

:process_daemon
echo  [1/3] Starting Core Daemon (Rust)
echo  ════════════════════════════════════════════════════
if not exist "%SCRIPT_DIR%Cargo.toml" (
    echo  ✗ Error: Cargo.toml not found
    exit /b 1
)

cd /d "%SCRIPT_DIR%"
start "Core Daemon" cargo run
timeout /t 2 /nobreak
goto started

:process_bot
echo  [2/3] Starting Discord Bot (Node.js)
echo  ════════════════════════════════════════════════════
if not exist "%SCRIPT_DIR%discord_bot\package.json" (
    echo  ✗ Error: discord_bot/package.json not found
    exit /b 1
)

cd /d "%SCRIPT_DIR%discord_bot"

if not exist ".env" (
    if exist ".env.example" (
        copy .env.example .env
        echo  ⚠ .env created from .env.example
        echo  ⚠ Please set DISCORD_TOKEN in .env
        echo  Get token from: https://discord.com/developers/applications
    ) else (
        echo  ✗ .env file not found
        exit /b 1
    )
)

if not exist "node_modules" (
    echo  Installing npm dependencies...
    call npm install --silent
)

start "Discord Bot" npm start
timeout /t 1 /nobreak
goto started

:process_gui
echo  [3/3] Starting Electron GUI (React)
echo  ════════════════════════════════════════════════════
if not exist "%SCRIPT_DIR%electron_gui\package.json" (
    echo  ✗ Error: electron_gui/package.json not found
    exit /b 1
)

cd /d "%SCRIPT_DIR%electron_gui"

if not exist "node_modules" (
    echo  Installing npm dependencies...
    call npm install --silent
)

start "Electron GUI" npm start
timeout /t 1 /nobreak
goto started

:process_all
echo  [1/3] Starting Core Daemon (Rust)
echo  ════════════════════════════════════════════════════
if not exist "%SCRIPT_DIR%Cargo.toml" (
    echo  ✗ Error: Cargo.toml not found
    exit /b 1
)

cd /d "%SCRIPT_DIR%"
start "Core Daemon" cargo run
timeout /t 3 /nobreak

echo.
echo  [2/3] Starting Discord Bot (Node.js)
echo  ════════════════════════════════════════════════════
if not exist "%SCRIPT_DIR%discord_bot\package.json" (
    echo  ✗ Error: discord_bot/package.json not found
    exit /b 1
)

cd /d "%SCRIPT_DIR%discord_bot"

if not exist ".env" (
    if exist ".env.example" (
        copy .env.example .env
        echo  ⚠ .env created from .env.example
        echo  ⚠ Please set DISCORD_TOKEN in .env
    )
)

if not exist "node_modules" (
    echo  Installing npm dependencies...
    call npm install --silent
)

start "Discord Bot" npm start
timeout /t 2 /nobreak

echo.
echo  [3/3] Starting Electron GUI (React)
echo  ════════════════════════════════════════════════════
if not exist "%SCRIPT_DIR%electron_gui\package.json" (
    echo  ✗ Error: electron_gui/package.json not found
    exit /b 1
)

cd /d "%SCRIPT_DIR%electron_gui"

if not exist "node_modules" (
    echo  Installing npm dependencies...
    call npm install --silent
)

start "Electron GUI" npm start
timeout /t 1 /nobreak

goto started

:started
echo.
echo  ╔════════════════════════════════════════════════════════════╗
echo  ║           All services are running!                       ║
echo  ╠════════════════════════════════════════════════════════════╣
echo  ║  Core Daemon:   http://localhost:57474                   ║
echo  ║  React Dev:     http://localhost:3000                    ║
echo  ║  Discord Bot:   Connected (check terminal)               ║
echo  ╠════════════════════════════════════════════════════════════╣
echo  ║  Press Ctrl+C in each terminal to stop services          ║
echo  ╚════════════════════════════════════════════════════════════╝
echo.
pause
exit /b 0

:process_%MODE%
echo  Unknown mode: %MODE%
echo  Usage: start-all.bat [daemon^|bot^|gui^|all]
exit /b 1
