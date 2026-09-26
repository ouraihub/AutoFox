@echo off
setlocal
for %%R in ("%~dp0..") do set "ROOT=%%~fR\"
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
set "SRC=%ROOT%target\release"
set "DIST=%ROOT%dist\autofox"
set "INSTALLER=%ROOT%dist\installer"
if not exist "%ROOT%yt-dlp.exe" (
  echo ERROR: missing release resource %ROOT%yt-dlp.exe 1>&2
  exit /b 1
)
if not exist "%ROOT%lib\ffmpeg.exe" (
  echo ERROR: missing release resource %ROOT%lib\ffmpeg.exe 1>&2
  exit /b 1
)
if not exist "%ROOT%lib\ffprobe.exe" (
  echo ERROR: missing release resource %ROOT%lib\ffprobe.exe 1>&2
  exit /b 1
)
call pnpm --dir "%ROOT%." generate:notice
if errorlevel 1 exit /b 1
call pnpm --dir "%ROOT%." tauri build --bundles nsis
if errorlevel 1 exit /b 1
if not exist "%SRC%\autofox.exe" (
  echo ERROR: release executable not found 1>&2
  exit /b 1
)
if not exist "%SRC%\bundle\nsis" (
  echo ERROR: NSIS output directory not found 1>&2
  exit /b 1
)

rd /s /q "%DIST%" 2>nul
if not exist "%INSTALLER%" mkdir "%INSTALLER%"
mkdir "%DIST%"
copy /y "%SRC%\autofox.exe" "%DIST%\autofox.exe" >nul || exit /b 1
if exist "%ROOT%yt-dlp.exe" copy /y "%ROOT%yt-dlp.exe" "%DIST%\yt-dlp.exe" >nul || exit /b 1
if not exist "%DIST%\lib" mkdir "%DIST%\lib"
if exist "%ROOT%lib\ffmpeg.exe" copy /y "%ROOT%lib\ffmpeg.exe" "%DIST%\lib\ffmpeg.exe" >nul || exit /b 1
if exist "%ROOT%lib\ffprobe.exe" copy /y "%ROOT%lib\ffprobe.exe" "%DIST%\lib\ffprobe.exe" >nul || exit /b 1
del /q "%INSTALLER%\*-setup.exe" 2>nul
for %%F in ("%SRC%\bundle\nsis\*-setup.exe") do copy /y "%%~F" "%INSTALLER%\" >nul || exit /b 1
for /f "tokens=3 delims= " %%V in ('findstr /r /c:"^version = " "%ROOT%src-tauri\Cargo.toml"') do set "APP_VER=%%~V"
if "%APP_VER%"=="" exit /b 1
set "ZIP=%INSTALLER%\AutoFox_%APP_VER%_x64-portable.zip"
del /q "%ZIP%" 2>nul
if exist "D:\Program Files\PowerShell\7\pwsh.exe" (
  "D:\Program Files\PowerShell\7\pwsh.exe" -NoProfile -NonInteractive -Command "Compress-Archive -Path '%DIST%' -DestinationPath '%ZIP%' -CompressionLevel Optimal -Force"
) else (
  "%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -Command "Compress-Archive -Path '%DIST%' -DestinationPath '%ZIP%' -CompressionLevel Optimal -Force"
)
if errorlevel 1 exit /b 1
exit /b 0
