@echo off
REM ---------------------------------------------------------------------
REM  Build the Sonor machine.
REM    1. sonor-core     -> WASM (public\sonor.wasm)  : voice + analysis
REM    2. sonor-listener -> EXE (native)             : WASAPI loopback
REM ---------------------------------------------------------------------
setlocal
set CARGO=%USERPROFILE%\.cargo\bin\cargo.exe
if not exist "%CARGO%" (
  echo [!] cargo not found - install Rust: https://rustup.rs
  exit /b 1
)
cd /d "%~dp0"

REM the running listener holds its own binary: stop it before rebuilding
taskkill /F /IM sonor-listener.exe >nul 2>&1
ping -n 2 127.0.0.1 >nul

echo [1/2] wasm core ...
"%CARGO%" build --release --target wasm32-unknown-unknown -p sonor-core
if errorlevel 1 exit /b 1
copy /y target\wasm32-unknown-unknown\release\sonor_core.wasm ..\public\sonor.wasm >nul
echo       ..\public\sonor.wasm

echo [2/2] native listener (windows-gnu) ...
REM use the toolchain's own lld instead of an external MinGW
if defined RUSTUP_HOME (set "RUSTUP=%RUSTUP_HOME%") else (set "RUSTUP=%USERPROFILE%\.rustup")
set "TOOLCHAIN=%RUSTUP%\toolchains\stable-x86_64-pc-windows-gnu"
set "LDDIR=%TOOLCHAIN%\lib\rustlib\x86_64-pc-windows-gnu\bin"
if not exist "%LDDIR%\rust-lld.exe" (
  echo [!] the stable-x86_64-pc-windows-gnu toolchain is missing:
  echo     rustup toolchain install stable-x86_64-pc-windows-gnu
  exit /b 1
)
set "PATH=%LDDIR%;%PATH%"
echo       linker: %LDDIR%\rust-lld.exe
"%CARGO%" +stable-x86_64-pc-windows-gnu build --release --target x86_64-pc-windows-gnu -p sonor-listener
if errorlevel 1 exit /b 1
copy /y target\x86_64-pc-windows-gnu\release\sonor-listener.exe ..\public\sonor-listener.exe >nul
echo       ..\public\sonor-listener.exe

echo.
echo [ok] the machine is assembled.
echo      start it with:  node server.mjs
endlocal
