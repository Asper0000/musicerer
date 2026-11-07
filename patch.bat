@echo off
cd /d "%~dp0"
if not exist "src" ( echo ERROR: Run from project root. & pause & exit /b 1 )

powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = '%~f0'; $l = [IO.File]::ReadAllLines($f); $i = 0; for ($j = 0; $j -lt $l.Count; $j++) { if ($l[$j] -eq '#__PS_START__#') { $i = $j + 1; break } }; if ($i -eq 0) { Write-Error 'PS marker missing'; exit 1 }; $c = ($l[$i..($l.Count-1)] -join [char]10); Invoke-Expression $c"

if errorlevel 1 ( echo. & echo Patch failed. & pause & exit /b 1 )
echo. & echo Done. Rebuild with: cargo build & pause
exit /b

#__PS_START__#
$ErrorActionPreference = 'Stop'
if (-not (Test-Path 'src')) { throw "Not in project root. Working dir: $PWD" }

Write-Host ''
Write-Host 'Fixing Rust string literal for YouTube Music SVG'
Write-Host ''

$iconsPath = 'src\ui\icons.rs'
$raw = [IO.File]::ReadAllText($iconsPath)

# Replace the broken raw string with a double-hash raw string
$newLine = 'pub const ICON_SRC_YT_MUSIC: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" xml:space="preserve" viewBox="0 0 192 192"><path fill="none" d="M0 0h192v192H0z"/><circle cx="96" cy="96" r="88" fill="red"/><path fill="#FFF" d="M96 50.32c25.19 0 45.68 20.49 45.68 45.68S121.19 141.68 96 141.68 50.32 121.19 50.32 96 70.81 50.32 96 50.32m0-6.4c-28.76 0-52.08 23.32-52.08 52.08 0 28.76 23.32 52.08 52.08 52.08s52.08-23.32 52.08-52.08c0-28.76-23.32-52.08-52.08-52.08z"/><path fill="#FFF" d="m79 122 45-26-45-26z"/></svg>"##;'

# Regex to match the entire ICON_SRC_YT_MUSIC line
$pattern = '(?m)^pub const ICON_SRC_YT_MUSIC.*$'
$raw = [regex]::Replace($raw, $pattern, $newLine)

[IO.File]::WriteAllText($iconsPath, $raw, [Text.UTF8Encoding]::new($false))
Write-Host "  Fixed ICON_SRC_YT_MUSIC in src\ui\icons.rs"

Write-Host ''
Write-Host 'Done. Rebuild with: cargo build'