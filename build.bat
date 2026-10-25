@echo off
setlocal EnableExtensions EnableDelayedExpansion
cd /d "%~dp0"

rem ---------------------------------------------------------------------------
rem  Musicerer build script  (run  build.bat help  for the target list)
rem
rem  Windows (native cargo)
rem    bundled    all-in-one exe           -> dist\bundled\musicerer.exe
rem    portable   slim exe + tools folder  -> dist\portable\musicerer.exe
rem    both       bundled + portable       (default when no target is given)
rem
rem  Linux (built inside WSL by packaging\linux\build.sh)
rem    appimage   -> dist\linux\appimage\Musicerer-<version>-<arch>.AppImage
rem    deb        -> dist\linux\deb\musicerer_<version>_<arch>.deb
rem    rpm        -> dist\linux\rpm\musicerer-<version>-1.<arch>.rpm
rem    linux      appimage + deb + rpm
rem
rem    all        both + linux
rem
rem  Targets can be combined:  build.bat portable deb rpm
rem  "linux-deb", "linux-rpm" and "linux-appimage" work as aliases.
rem
rem  Windows tools : .\tools        (yt-dlp.exe ffmpeg.exe ffprobe.exe deno.exe soundcloud.exe)
rem  Linux tools   : .\linux_tools  (yt-dlp ffmpeg ffprobe deno soundcloud, Linux binaries)
rem ---------------------------------------------------------------------------

set "DO_BUNDLED=0"
set "DO_PORTABLE=0"
set "DO_APPIMAGE=0"
set "DO_DEB=0"
set "DO_RPM=0"

if "%~1"=="" ( set "DO_BUNDLED=1" & set "DO_PORTABLE=1" )

:parse
if "%~1"=="" goto parsed
set "ARG=%~1"
if /i "!ARG!"=="both"           ( set "DO_BUNDLED=1" & set "DO_PORTABLE=1" & goto next )
if /i "!ARG!"=="bundled"        ( set "DO_BUNDLED=1" & goto next )
if /i "!ARG!"=="portable"       ( set "DO_PORTABLE=1" & goto next )
if /i "!ARG!"=="appimage"       ( set "DO_APPIMAGE=1" & goto next )
if /i "!ARG!"=="linux-appimage" ( set "DO_APPIMAGE=1" & goto next )
if /i "!ARG!"=="deb"            ( set "DO_DEB=1" & goto next )
if /i "!ARG!"=="linux-deb"      ( set "DO_DEB=1" & goto next )
if /i "!ARG!"=="rpm"            ( set "DO_RPM=1" & goto next )
if /i "!ARG!"=="linux-rpm"      ( set "DO_RPM=1" & goto next )
if /i "!ARG!"=="linux"          ( set "DO_APPIMAGE=1" & set "DO_DEB=1" & set "DO_RPM=1" & goto next )
if /i "!ARG!"=="all"            ( set "DO_BUNDLED=1" & set "DO_PORTABLE=1" & set "DO_APPIMAGE=1" & set "DO_DEB=1" & set "DO_RPM=1" & goto next )
if /i "!ARG!"=="help"           goto usage
if /i "!ARG!"=="-h"             goto usage
if /i "!ARG!"=="/?"             goto usage
echo Unknown target: !ARG!
echo.
goto usage
:next
shift
goto parse

:usage
echo Usage: build.bat [target ...]
echo.
echo   Windows   bundled ^| portable ^| both          (default: both)
echo   Linux     appimage ^| deb ^| rpm ^| linux        (built through WSL)
echo   Everything: all
echo.
echo   Example:  build.bat portable deb rpm
echo   Output:   dist\bundled, dist\portable, dist\linux\{appimage,deb,rpm}
exit /b 1

:parsed
set "LINUX_ARGS="
if "%DO_APPIMAGE%"=="1" set "LINUX_ARGS=!LINUX_ARGS! appimage"
if "%DO_DEB%"=="1"      set "LINUX_ARGS=!LINUX_ARGS! deb"
if "%DO_RPM%"=="1"      set "LINUX_ARGS=!LINUX_ARGS! rpm"

rem ---- Windows preflight ----------------------------------------------------
if "%DO_BUNDLED%%DO_PORTABLE%"=="00" goto windows_done

where cargo >nul 2>&1
if errorlevel 1 ( echo cargo was not found. Install the Rust toolchain first. & exit /b 1 )

if not exist "assets\icon.ico" ( echo assets\icon.ico is missing. & exit /b 1 )

if "%DO_BUNDLED%"=="1" (
    set "MISSING="
    for %%T in (yt-dlp ffmpeg ffprobe deno soundcloud) do (
        if not exist "tools\%%T.exe" set "MISSING=!MISSING! %%T.exe"
    )
    if defined MISSING (
        echo The bundled build needs these files in the tools folder:!MISSING!
        exit /b 1
    )
)

if not exist "dist" mkdir "dist"

if "%DO_PORTABLE%"=="1" (
    echo.
    echo === Building portable ===
    cargo build --release --target-dir "target\portable"
    if errorlevel 1 exit /b 1

    if exist "dist\portable\musicerer.exe" del /q "dist\portable\musicerer.exe"
    if not exist "dist\portable" mkdir "dist\portable"
    if not exist "dist\portable\tools" mkdir "dist\portable\tools"
    copy /Y "target\portable\release\musicerer.exe" "dist\portable\musicerer.exe" >nul
    if errorlevel 1 ( echo Could not copy the portable exe. & exit /b 1 )
    > "dist\portable\tools\README.txt" echo Put yt-dlp.exe, ffmpeg.exe, ffprobe.exe, deno.exe and soundcloud.exe ^(optional^) in this folder.
    >> "dist\portable\tools\README.txt" echo Anything missing here is looked up on PATH instead.
)

if "%DO_BUNDLED%"=="1" (
    echo.
    echo === Building bundled ===
    cargo build --release --features bundled --target-dir "target\bundled"
    if errorlevel 1 exit /b 1

    if not exist "dist\bundled" mkdir "dist\bundled"
    copy /Y "target\bundled\release\musicerer.exe" "dist\bundled\musicerer.exe" >nul
    if errorlevel 1 ( echo Could not copy the bundled exe. & exit /b 1 )
)
:windows_done

rem ---- Linux (via WSL) --------------------------------------------------------
if not defined LINUX_ARGS goto linux_done

where wsl >nul 2>&1
if errorlevel 1 (
    echo WSL was not found. Install it ^(wsl --install^) to build the Linux packages.
    exit /b 1
)
wsl -e true >nul 2>&1
if errorlevel 1 (
    echo WSL is installed but has no working Linux distribution. Run: wsl --install -d Ubuntu
    exit /b 1
)
if not exist "linux_tools" (
    echo The linux_tools folder is missing. Put the Linux binaries yt-dlp, ffmpeg, ffprobe, deno
    echo and soundcloud in it.
    exit /b 1
)

echo.
echo === Building Linux packages in WSL:!LINUX_ARGS! ===
wsl --cd "%CD%" -e bash packaging/linux/build.sh!LINUX_ARGS!
if errorlevel 1 ( echo Linux build failed. & exit /b 1 )
:linux_done

echo.
echo Done.
if "%DO_BUNDLED%"=="1"  echo   dist\bundled\musicerer.exe
if "%DO_PORTABLE%"=="1" echo   dist\portable\musicerer.exe
if "%DO_APPIMAGE%"=="1" echo   dist\linux\appimage\
if "%DO_DEB%"=="1"      echo   dist\linux\deb\
if "%DO_RPM%"=="1"      echo   dist\linux\rpm\
endlocal
