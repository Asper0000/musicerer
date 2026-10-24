@echo off
setlocal
cd /d "%~dp0"
set "PATH=%CD%\tools;%PATH%"

if not exist "tools\yt-dlp.exe"  ( echo tools\yt-dlp.exe missing. Add yt-dlp.exe to the tools folder before running. & exit /b 1 )
if not exist "tools\ffmpeg.exe"  ( echo tools\ffmpeg.exe missing. Add ffmpeg.exe to the tools folder before building. & exit /b 1 )
if not exist "tools\ffprobe.exe" ( echo tools\ffprobe.exe missing. Add ffprobe.exe to the tools folder before building. & exit /b 1 )
if not exist "assets\icon.ico" ( echo assets\icon.ico is missing. & exit /b 1 )

echo Building release...
cargo build --release
if errorlevel 1 exit /b 1

echo Staging tools next to exe...
if not exist "target\release\tools" mkdir "target\release\tools"
robocopy "tools" "target\release\tools" /E /NJH /NJS /NDL /NFL /NC /NS /NP >nul
if errorlevel 8 (
    echo robocopy failed while staging tools.
    exit /b 1
)

if not exist "target\release\assets" mkdir "target\release\assets"
copy /Y "assets\icon.ico" "target\release\assets\icon.ico" >nul
if errorlevel 1 (
    echo Could not stage the window icon.
    exit /b 1
)

echo Launching Musicerer...
"target\release\musicerer.exe"
