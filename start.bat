@echo off
setlocal EnableExtensions
rem ------------------------------------------------------------------
rem  mouse-kb-remap launcher
rem
rem    start.bat               run the app (builds it first if no exe exists)
rem    start.bat build         rebuild from source into dist\, then run
rem    start.bat admin         run as administrator (needed to remap games
rem                            that themselves run as admin, e.g. Genshin)
rem    start.bat build admin   both
rem ------------------------------------------------------------------

cd /d "%~dp0"

set "BUILD="
set "ADMIN="
:parse_args
if "%~1"=="" goto args_done
if /i "%~1"=="build"   set "BUILD=1"
if /i "%~1"=="--build" set "BUILD=1"
if /i "%~1"=="admin"   set "ADMIN=1"
if /i "%~1"=="--admin" set "ADMIN=1"
shift
goto parse_args
:args_done

rem Prefer an exe next to this script (release zip layout), then dist\.
set "EXE="
if exist "%~dp0mouse-kb-remap.exe" set "EXE=%~dp0mouse-kb-remap.exe"
if not defined EXE if exist "%~dp0dist\mouse-kb-remap.exe" set "EXE=%~dp0dist\mouse-kb-remap.exe"

if defined BUILD goto build
if defined EXE goto run

:build
if not exist "%~dp0Cargo.toml" (
    echo No source code here to build. Clone the full repository to build from source.
    pause
    exit /b 1
)
where cargo >nul 2>nul
if errorlevel 1 (
    echo Rust is not installed. Install it from https://rustup.rs and run this again.
    pause
    exit /b 1
)
echo Building release build, this takes a few minutes the first time...
cargo build --release
if errorlevel 1 (
    echo.
    echo Build failed. See the errors above.
    pause
    exit /b 1
)
if not exist "%~dp0dist" mkdir "%~dp0dist"
copy /y "%~dp0target\release\mouse-kb-remap.exe" "%~dp0dist\mouse-kb-remap.exe" >nul
if errorlevel 1 (
    echo Could not copy the new exe into dist\. Close mouse-kb-remap if it is running and try again.
    pause
    exit /b 1
)
set "EXE=%~dp0dist\mouse-kb-remap.exe"

:run
if defined ADMIN (
    powershell -NoProfile -Command "Start-Process -FilePath '%EXE%' -Verb RunAs"
) else (
    start "" "%EXE%"
)
endlocal
