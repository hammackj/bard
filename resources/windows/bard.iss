; Inno Setup script for Bard's Windows installer. The release workflow runs:
;
;   ISCC /DAppVersion=0.2.16 /DStage=<folder with bard.exe and docs> /O<out> bard.iss
;
; Installs per user by default (no admin prompt), into
; %LOCALAPPDATA%\Programs\Bard, with an option to install for all users. Adds
; a Start menu entry, an optional desktop shortcut, and an uninstaller listed
; in Settings > Apps.

#ifndef AppVersion
  #error Pass /DAppVersion=X.Y.Z
#endif
#ifndef Stage
  #error Pass /DStage=<folder with bard.exe>
#endif

[Setup]
; Never change AppId: upgrades and the uninstaller find Bard by it.
AppId={{5B0C2E57-5D8F-4C46-9F0B-6A1E7C3D2B91}
AppName=Bard
AppVersion={#AppVersion}
AppVerName=Bard {#AppVersion}
AppPublisher=hammackj
AppPublisherURL=https://github.com/hammackj/bard
AppSupportURL=https://github.com/hammackj/bard/issues
AppUpdatesURL=https://github.com/hammackj/bard/releases
DefaultDirName={autopf}\Bard
DefaultGroupName=Bard
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputBaseFilename=bard-{#AppVersion}-x86_64-windows-setup
SetupIconFile=bard.ico
UninstallDisplayIcon={app}\bard.exe
UninstallDisplayName=Bard
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; Closes a running Bard (via Restart Manager) before replacing or removing it.
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Stage}\bard.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\CHANGELOG.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\THIRD-PARTY.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Bard"; Filename: "{app}\bard.exe"
Name: "{autodesktop}\Bard"; Filename: "{app}\bard.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\bard.exe"; Description: "{cm:LaunchProgram,Bard}"; Flags: nowait postinstall skipifsilent

[Code]
// Bard's own data lives outside the install folder: settings, play counts,
// loved tracks and the artwork cache in %APPDATA%\bard, and the OBS password
// in Credential Manager. Uninstalling keeps them unless the user says
// otherwise, so a reinstall picks up where it left off. Music files are
// never touched.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
  ResultCode: Integer;
begin
  if (CurUninstallStep <> usPostUninstall) or UninstallSilent then
    exit;
  DataDir := ExpandConstant('{userappdata}\bard');
  if MsgBox('Also delete your Bard settings, play counts, loved tracks and artwork cache, and the saved OBS password?' + #13#10#13#10 +
            'They are in ' + DataDir + ' and Credential Manager. Your music files are not touched.' + #13#10#13#10 +
            'Choose No to keep them for a later reinstall.',
            mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
  begin
    DelTree(DataDir, True, True, True);
    Exec(ExpandConstant('{sys}\cmdkey.exe'), '/delete:obs-password.bard', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  end;
end;
