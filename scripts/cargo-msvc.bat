@echo off
setlocal
for %%R in ("%~dp0..") do set "ROOT=%%~fR\"
set "MANIFEST=%ROOT%Cargo.toml"
set "MSVC_ROOT=%PORTABLE_MSVC_ROOT%"
if "%MSVC_ROOT%"=="" set "MSVC_ROOT=D:\Applications\msvc-portable\msvc"
where /q cl.exe
if errorlevel 1 (
  if not exist "%MSVC_ROOT%\setup_x64.bat" (
    echo ERROR: MSVC toolchain not found at %MSVC_ROOT%\setup_x64.bat 1>&2
    exit /b 1
  )
  call "%MSVC_ROOT%\setup_x64.bat" >nul
  where /q cl.exe || exit /b 1
)
set "CARGO_TARGET_DIR=%ROOT%target"
set "SUBCOMMAND=%1"
shift
set "ARGS="
:collect_args
if "%~1"=="" goto run_cargo
set "ARGS=%ARGS% "%~1""
shift
goto collect_args
:run_cargo
cargo %SUBCOMMAND% --manifest-path "%MANIFEST%"%ARGS%
exit /b %ERRORLEVEL%
