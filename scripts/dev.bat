@echo off
setlocal
for %%R in ("%~dp0..") do set "ROOT=%%~fR\"
set "MSVC_ROOT=%PORTABLE_MSVC_ROOT%"
if "%MSVC_ROOT%"=="" set "MSVC_ROOT=D:\Applications\msvc-portable\msvc"
where /q cl.exe
if errorlevel 1 (
  if not exist "%MSVC_ROOT%\setup_x64.bat" exit /b 1
  call "%MSVC_ROOT%\setup_x64.bat" >nul
  where /q cl.exe || exit /b 1
)
set "CARGO_TARGET_DIR=%ROOT%target"
call pnpm --dir "%ROOT%." tauri dev
exit /b %ERRORLEVEL%
