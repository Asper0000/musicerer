@echo off
setlocal EnableExtensions EnableDelayedExpansion
cd /d "%~dp0"

rem Musicerer build targets:
rem   portable  -> dist\portable\musicerer.exe
rem   setup     -> dist\installer\Musicerer_Setup_v<version>.exe
rem   both      -> portable + setup (default)
rem   Linux: appimage, deb, rpm, linux; all = both + Linux packages

set "DO_PORTABLE=0"
set "DO_SETUP=0"
set "DO_APPIMAGE=0"
set "DO_DEB=0"
set "DO_RPM=0"

if "%~1"=="" (
    set "DO_PORTABLE=1"
    set "DO_SETUP=1"
)

:parse
if "%~1"=="" goto parsed
set "ARG=%~1"
if /i "!ARG!"=="both"           ( set "DO_PORTABLE=1" & set "DO_SETUP=1" & goto next )
if /i "!ARG!"=="portable"       ( set "DO_PORTABLE=1" & goto next )
if /i "!ARG!"=="setup"          ( set "DO_SETUP=1" & goto next )
if /i "!ARG!"=="appimage"       ( set "DO_APPIMAGE=1" & goto next )
if /i "!ARG!"=="linux-appimage" ( set "DO_APPIMAGE=1" & goto next )
if /i "!ARG!"=="deb"            ( set "DO_DEB=1" & goto next )
if /i "!ARG!"=="linux-deb"      ( set "DO_DEB=1" & goto next )
if /i "!ARG!"=="rpm"            ( set "DO_RPM=1" & goto next )
if /i "!ARG!"=="linux-rpm"      ( set "DO_RPM=1" & goto next )
if /i "!ARG!"=="linux"          ( set "DO_APPIMAGE=1" & set "DO_DEB=1" & set "DO_RPM=1" & goto next )
if /i "!ARG!"=="all"            ( set "DO_PORTABLE=1" & set "DO_SETUP=1" & set "DO_APPIMAGE=1" & set "DO_DEB=1" & set "DO_RPM=1" & goto next )
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
echo   Windows   portable ^| setup ^| both       (default: both)
echo   Linux     appimage ^| deb ^| rpm ^| linux
echo   Everything: all
echo.
echo   Example:  build.bat portable deb rpm
echo   Output:   dist\portable, dist\installer, dist\linux\{appimage,deb,rpm}
exit /b 1

:parsed
set "LINUX_ARGS="
if "%DO_APPIMAGE%"=="1" set "LINUX_ARGS=!LINUX_ARGS! appimage"
if "%DO_DEB%"=="1"      set "LINUX_ARGS=!LINUX_ARGS! deb"
if "%DO_RPM%"=="1"      set "LINUX_ARGS=!LINUX_ARGS! rpm"

rem A setup build always builds the portable executable first.
if "%DO_SETUP%"=="1" set "DO_PORTABLE=1"

if "%DO_PORTABLE%"=="1" (
    where cargo >nul 2>&1
    if errorlevel 1 ( echo cargo was not found. Install the Rust toolchain first. & exit /b 1 )
    if not exist "assets\icon.ico" ( echo assets\icon.ico is missing. & exit /b 1 )
    echo.
    echo === Building portable ===
    cargo build --release --target-dir "target\portable"
    if errorlevel 1 exit /b 1
    if not exist "dist\portable" mkdir "dist\portable"
    copy /Y "target\portable\release\musicerer.exe" "dist\portable\musicerer.exe" >nul
    if errorlevel 1 ( echo Could not copy the portable executable. & exit /b 1 )
)

if "%DO_SETUP%"=="1" (
    echo.
    echo === Building Windows setup ===
    call :build_setup
    if errorlevel 1 exit /b 1
)

if not defined LINUX_ARGS goto linux_done
where wsl >nul 2>&1
if errorlevel 1 ( echo WSL was not found. Install it ^(wsl --install^) to build Linux packages. & exit /b 1 )
wsl -e true >nul 2>&1
if errorlevel 1 ( echo WSL has no working Linux distribution. Run: wsl --install -d Ubuntu & exit /b 1 )
echo.
echo === Building Linux packages in WSL:!LINUX_ARGS! ===
wsl --cd "%CD%" -e bash packaging/linux/build.sh!LINUX_ARGS!
if errorlevel 1 ( echo Linux build failed. & exit /b 1 )

:linux_done
echo.
echo Done.
if "%DO_PORTABLE%"=="1" echo   dist\portable\musicerer.exe
if "%DO_SETUP%"=="1"    echo   dist\installer\Musicerer_Setup_v!APP_VERSION!.exe
if "%DO_APPIMAGE%"=="1" echo   dist\linux\appimage\
if "%DO_DEB%"=="1"      echo   dist\linux\deb\
if "%DO_RPM%"=="1"      echo   dist\linux\rpm\
endlocal
goto :eof

:build_setup
if not exist "%~dp0scripts\musicercer_portable_setup.iss" (
    echo Installer script is missing: scripts\musicercer_portable_setup.iss
    exit /b 1
)

set "ISCC_EXE="
for /f "delims=" %%I in ('where ISCC.exe 2^>nul') do if not defined ISCC_EXE set "ISCC_EXE=%%~I"

if not defined ISCC_EXE if exist "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" set "ISCC_EXE=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not defined ISCC_EXE if exist "%ProgramFiles%\Inno Setup 6\ISCC.exe" set "ISCC_EXE=%ProgramFiles%\Inno Setup 6\ISCC.exe"
if not defined ISCC_EXE if exist "%ProgramFiles(x86)%\Inno Setup 7\ISCC.exe" set "ISCC_EXE=%ProgramFiles(x86)%\Inno Setup 7\ISCC.exe"
if not defined ISCC_EXE if exist "%ProgramFiles%\Inno Setup 7\ISCC.exe" set "ISCC_EXE=%ProgramFiles%\Inno Setup 7\ISCC.exe"

if not defined ISCC_EXE (
    echo Inno Setup Compiler was not found.
    echo Install Inno Setup 6 or 7, or add ISCC.exe to PATH.
    exit /b 1
)
if not exist "!ISCC_EXE!" (
    echo Inno Setup Compiler path does not exist: !ISCC_EXE!
    exit /b 1
)

set "APP_VERSION="
for /f "tokens=3" %%V in ('findstr /b /c:"version =" "%~dp0Cargo.toml"') do set "APP_VERSION=%%~V"
if not defined APP_VERSION (
    echo Could not read version from Cargo.toml.
    exit /b 1
)

if not exist "%~dp0dist\installer" mkdir "%~dp0dist\installer"
echo Using Inno Setup: !ISCC_EXE!
echo Building version: !APP_VERSION!
"!ISCC_EXE!" "/DMyAppVersion=!APP_VERSION!" "%~dp0scripts\musicercer_portable_setup.iss"
if errorlevel 1 (
    echo Inno Setup compilation failed.
    exit /b 1
)
exit /b 0
