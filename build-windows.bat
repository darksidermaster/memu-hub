@echo off
setlocal
cd /d "%~dp0"
where cargo >nul 2>&1
if errorlevel 1 (
 echo Instale Rust MSVC e o Visual Studio Build Tools com C++ primeiro.
 pause
 exit /b 1
)
call npm install
if errorlevel 1 exit /b 1
call npm run build
if errorlevel 1 exit /b 1
echo Executavel: %CD%\src-tauri\target\release\memu-hub.exe
pause
