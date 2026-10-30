@echo off
setlocal
REM ============================================================
REM  dump.bat [root_dir] [output_file]
REM  Example:  dump.bat C:\projects\myapp dump.txt
REM  Defaults: root = current dir, output = codebase_dump.txt
REM ============================================================

set "ROOT=%~1"
if "%ROOT%"=="" set "ROOT=%~dp0.."
set "OUT=%~2"
if "%OUT%"=="" set "OUT=%CD%\codebase_dump.txt"

echo Dumping: %ROOT%
echo Output : %OUT%
echo.

powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "$ErrorActionPreference='SilentlyContinue';" ^
  "$root='%ROOT%'; $out='%OUT%';" ^
  "$skipDirs='\\\.git\\|\\node_modules\\|\\linux_tools\\|\\bin\\|\\obj\\|\\\.vs\\|\\dist\\|\\build\\|\\\.next\\|\\__pycache__\\|\\\.venv\\|\\venv\\|\\\.idea\\|\\\.vscode\\|\\coverage\\|\\target\\|\\out\\|\\Detours\\';" ^
  "$skipExt='\.(exe|dll|pdb|so|dylib|class|jar|pyc|o|a|lib|bin|zip|tar|gz|7z|rar|png|jpg|jpeg|gif|bmp|ico|pdf|mp3|mp4|avi|mov|wav|flac|woff|woff2|ttf|eot|otf|db|sqlite|lock|map|min\.js)$';" ^
  "if(Test-Path $out){Remove-Item $out -Force};" ^
  "$files = Get-ChildItem -Path $root -Recurse -File -Force | Where-Object { $p=$_.FullName; -not ($p -match $skipDirs) -and -not ($p -match $skipExt) } | Sort-Object FullName;" ^
  "'===== CODEBASE DUMP =====' | Set-Content $out;" ^
  "('Root : '+$root)  | Add-Content $out;" ^
  "('Date : '+(Get-Date)) | Add-Content $out;" ^
  "('Files: '+$files.Count) | Add-Content $out;" ^
  "'' | Add-Content $out;" ^
  "foreach($f in $files){" ^
  "  $rel = $f.FullName.Substring($root.Length).TrimStart('\');" ^
  "  Add-Content $out '';" ^
  "  Add-Content $out ('===== FILE: '+$rel+' =====');" ^
  "  try { Add-Content $out -Value (Get-Content -LiteralPath $f.FullName -Raw -ErrorAction Stop) }" ^
  "  catch { Add-Content $out '[unreadable / binary]' }" ^
  "};" ^
  "Write-Host ('Done. '+$files.Count+' files written to '+$out)"

endlocal
pause