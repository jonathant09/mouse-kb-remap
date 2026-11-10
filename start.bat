@echo off
setlocal EnableExtensions
rem ------------------------------------------------------------------
rem  mouse-kb-remap from source (needs Rust: https://rustup.rs)
rem
rem    start.bat               run the app from this checkout (builds it
rem                            first if there is no build yet)
rem    start.bat build         rebuild, then run
rem    start.bat admin         run as administrator (needed to remap games
rem                            that themselves run as admin, e.g. Genshin)
rem    start.bat install       build and install, or update in place, the
rem                            copy in Documents\mouse-kb-remap (restarts it
rem                            if it is running)
rem
rem  Just want to use the app? Download it from the GitHub Releases page
rem  instead: no Rust needed.
rem ------------------------------------------------------------------

cd /d "%~dp0"

set "BUILD="
set "ADMIN="
:parse_args
if "%~1"=="" goto args_done
if /i "%~1"=="install" goto install
if /i "%~1"=="build"   set "BUILD=1"
if /i "%~1"=="admin"   set "ADMIN=1"
shift
goto parse_args
:args_done

where cargo >nul 2>nul
if errorlevel 1 (
    echo Rust is not installed. Install it from https://rustup.rs and run this again,
    echo or download the ready-to-run app from the GitHub Releases page.
    pause
    exit /b 1
)

set "EXE=%~dp0target\release\mouse-kb-remap.exe"
if defined BUILD goto build
if exist "%EXE%" goto run

:build
echo Building, this takes a few minutes the first time...
cargo build --release
if errorlevel 1 (
    echo.
    echo Build failed. See the errors above.
    pause
    exit /b 1
)

:run
if defined ADMIN (
    powershell -NoProfile -Command "Start-Process -FilePath '%EXE%' -Verb RunAs"
) else (
    start "" "%EXE%"
)
exit /b 0

:install
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\install.ps1"
if errorlevel 1 pause
exit /b %errorlevel%
