; Musicerer Windows installer
; Build with build.bat setup. The version is passed from Cargo.toml through /DMyAppVersion=...
#ifndef MyAppVersion
  #define MyAppVersion "0.3.2"
#endif
#define MyAppName "Musicerer"
#define MyAppPublisher "Asper"
#define MyAppURL "https://github.com/Asper0000/musicerer"
#define MyAppExeName "musicerer.exe"

[Setup]
AppId={{7C9C48A1-3B49-4B9E-BFA2-86A1B2C3D4E5}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
CloseApplications=yes
RestartApplications=no
OutputDir=..\dist\installer
OutputBaseFilename=Musicerer-v{#MyAppVersion}-windows-x64-setup
SetupIconFile=..\assets\icon.ico
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\{#MyAppExeName}
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; Install only Musicerer. Runtime media tools are downloaded after launch.
Source: "..\target\portable\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[UninstallDelete]
; Leftovers of self-updates and of older installers that shipped a tools folder.
Type: files; Name: "{app}\{#MyAppExeName}.old"
Type: files; Name: "{app}\{#MyAppExeName}.new"
Type: filesandordirs; Name: "{app}\tools"

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Code]
function IsAppRunning(): Boolean;
var
  Code: Integer;
begin
  Result := Exec(ExpandConstant('{cmd}'),
    '/C tasklist /FI "IMAGENAME eq {#MyAppExeName}" | find /I "{#MyAppExeName}"',
    '', SW_HIDE, ewWaitUntilTerminated, Code) and (Code = 0);
end;

function StopApp(): Boolean;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /T /IM {#MyAppExeName}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(800);
  Result := not IsAppRunning();
end;

{ A running copy locks its own exe, which is what made removal fail silently. }
function InitializeUninstall(): Boolean;
begin
  Result := True;
  if not IsAppRunning() then Exit;
  if not UninstallSilent() then
  begin
    if MsgBox('Musicerer is still running and has to be closed to continue.' + #13#10#13#10 +
              'Close it and uninstall?', mbConfirmation, MB_YESNO) <> IDYES then
    begin
      Result := False;
      Exit;
    end;
  end;
  if not StopApp() then
  begin
    if not UninstallSilent() then
      MsgBox('Musicerer could not be closed. Close it manually and run the uninstaller again.', mbError, MB_OK);
    Result := False;
  end;
end;

{ Settings, downloaded tools and cache live outside the install folder, so Inno does not
  know about them. When Musicerer uninstalls itself (silent) its own dialog has already
  asked what to keep, so only a manual uninstall asks here. Downloaded music is never touched. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and (not UninstallSilent()) then
  begin
    if MsgBox('Also remove Musicerer settings, downloaded tools and cache?' + #13#10#13#10 +
              'Your downloaded music is not affected.', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      DelTree(ExpandConstant('{userappdata}\Musicerer'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\Musicerer'), True, True, True);
      DelTree(ExpandConstant('{%TEMP}\Musicerer'), True, True, True);
    end;
  end;
end;
