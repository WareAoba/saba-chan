@echo off
REM Discord Bot 개발 모드 실행 스크립트 (Windows)

echo.
echo  🚀 Discord Bot - Development Mode
echo  ==================================
echo.

REM 현재 디렉터리 확인
if not exist "package.json" (
    echo  ❌ Error: package.json not found. Run from discord_bot directory.
    exit /b 1
)

REM .env 파일 확인
if not exist ".env" (
    echo  ⚠️  .env file not found!
    echo  Creating from .env.example...
    
    if exist ".env.example" (
        copy .env.example .env
        echo  ✓ .env created
        echo.
        echo  ⚠️  IMPORTANT: Set DISCORD_TOKEN in .env file
        echo  Get token from: https://discord.com/developers/applications
        echo.
        exit /b 1
    ) else (
        echo  ❌ .env.example not found either
        exit /b 1
    )
)

REM node_modules 확인
if not exist "node_modules" (
    echo  📦 Installing dependencies...
    call npm install
)

echo  📝 Configuration:
for /f "tokens=*" %%A in ('findstr IPC_BASE .env') do echo    %%A

echo.
echo  Starting Discord Bot...
echo  Press Ctrl+C to stop
echo.

npm start
