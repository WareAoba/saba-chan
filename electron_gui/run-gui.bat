@echo off
REM Electron GUI 개발 모드 실행 스크립트 (Windows)

echo.
echo  🚀 Electron GUI - Development Mode
echo  ====================================
echo.

REM 현재 디렉터리 확인
if not exist "package.json" (
    echo  ❌ Error: package.json not found. Run from electron_gui directory.
    exit /b 1
)

REM node_modules 확인
if not exist "node_modules" (
    echo  📦 Installing dependencies...
    call npm install
)

echo  🔗 IPC Configuration:
echo    IPC_BASE: http://localhost:57474
echo.

echo  📱 Ports:
echo    React Dev Server: http://localhost:3000
echo    Electron App: (desktop window)
echo.

echo  Starting Electron GUI...
echo  Press Ctrl+C to stop
echo.

npm start
