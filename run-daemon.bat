@echo off
REM Core Daemon 개발 모드 실행 스크립트 (Windows)

echo.
echo  🚀 Game Server Management Platform - Development Mode
echo  ==================================================
echo.

REM 현재 디렉터리 확인
if not exist "Cargo.toml" (
    echo  ❌ Error: Cargo.toml not found. Run this script from the Bot directory.
    exit /b 1
)

echo  1️⃣  Core Daemon (Rust)
echo  Starting with debug logging...
echo.

REM 로그 레벨 설정
if not defined RUST_LOG (
    set RUST_LOG=debug
)

REM modules 디렉터리가 없으면 생성
if not exist "modules" (
    echo  📁 Creating modules directory...
    mkdir modules
)

REM 모듈 발견 확인
echo  📦 Available modules:
if exist "modules" (
    for /d %%D in (modules\*) do (
        if exist "%%D\module.toml" (
            echo  ✓ %%~nD
        )
    )
) else (
    echo   (none - add modules to modules\ directory)
)

echo.
echo  Starting Core Daemon...
echo  Press Ctrl+C to stop
echo.

cargo run
