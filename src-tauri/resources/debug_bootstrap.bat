@echo off
REM GlideKit — Debug Bootstrap Launcher
REM Double-click this to diagnose bootstrap issues.
REM Keeps the terminal open so you can see what happened.

echo ========================================
echo   GlideKit — Debug Bootstrap
echo ========================================
echo.

set CLIPPER_HOME=%LOCALAPPDATA%\glidekit
set LOG_FILE=%CLIPPER_HOME%\debug-bootstrap.log

echo Creating log directory...
if not exist "%CLIPPER_HOME%" mkdir "%CLIPPER_HOME%"

echo Debug log: %LOG_FILE%
echo.

(
    echo ========================================
    echo Debug Bootstrap Log
    echo Date: %DATE% %TIME%
    echo ========================================
    echo.
    echo --- System Info ---
    echo OS: %OS%
    echo PROCESSOR_ARCHITECTURE: %PROCESSOR_ARCHITECTURE%
    echo USERNAME: %USERNAME%
    echo LOCALAPPDATA: %LOCALAPPDATA%
    echo.
    echo --- PATH ---
    echo %PATH%
    echo.
    echo --- PowerShell Version ---
) > "%LOG_FILE%"

powershell -NoProfile -Command "$PSVersionTable | Out-String" >> "%LOG_FILE%" 2>&1

(
    echo.
    echo --- Checking for bootstrap.ps1 ---
) >> "%LOG_FILE%"

REM Check common locations for bootstrap.ps1
set FOUND=0

if exist "%~dp0bootstrap.ps1" (
    echo [FOUND] %~dp0bootstrap.ps1 >> "%LOG_FILE%"
    set SCRIPT_PATH=%~dp0bootstrap.ps1
    set FOUND=1
) else (
    echo [NOT FOUND] %~dp0bootstrap.ps1 >> "%LOG_FILE%"
)

if exist "%~dp0..\resources\bootstrap.ps1" (
    echo [FOUND] %~dp0..\resources\bootstrap.ps1 >> "%LOG_FILE%"
    if %FOUND%==0 (
        set SCRIPT_PATH=%~dp0..\resources\bootstrap.ps1
        set FOUND=1
    )
) else (
    echo [NOT FOUND] %~dp0..\resources\bootstrap.ps1 >> "%LOG_FILE%"
)

REM List what's in the current directory
echo. >> "%LOG_FILE%"
echo --- Contents of %~dp0 --- >> "%LOG_FILE%"
dir /b "%~dp0" >> "%LOG_FILE%" 2>&1

echo. >> "%LOG_FILE%"
echo --- Contents of %~dp0.. --- >> "%LOG_FILE%"
dir /b "%~dp0.." >> "%LOG_FILE%" 2>&1

if %FOUND%==0 (
    echo. >> "%LOG_FILE%"
    echo [ERROR] bootstrap.ps1 not found in any expected location >> "%LOG_FILE%"
    echo.
    echo ERROR: bootstrap.ps1 not found!
    echo Check %LOG_FILE% for details.
    echo.
    pause
    exit /b 1
)

echo.
echo Found bootstrap script: %SCRIPT_PATH%
echo Running bootstrap with full output...
echo.
echo ========================================

(
    echo.
    echo --- Running bootstrap.ps1 ---
    echo Script: %SCRIPT_PATH%
    echo.
) >> "%LOG_FILE%"

REM Run the bootstrap with output visible AND logged
powershell -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT_PATH%" 2>&1 | powershell -NoProfile -Command "$input | Tee-Object -FilePath '%LOG_FILE%' -Append"

echo.
echo ========================================
echo Exit code: %ERRORLEVEL%
echo Log saved to: %LOG_FILE%
echo ========================================
echo.
echo Press any key to close...
pause > nul
