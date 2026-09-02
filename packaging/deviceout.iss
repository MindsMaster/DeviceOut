#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName    "DeviceOut"
#define MyBundleName "DeviceOut.vst3"
#define MyPublisher  "DeviceOut"

[Setup]
AppId={{8F3C1A20-6B4D-4E91-9C2A-7D5E1B0F4A33}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyPublisher}
AppPublisherURL=https://github.com/MindsMaster/DeviceOut
AppSupportURL=https://github.com/MindsMaster/DeviceOut/issues
VersionInfoVersion={#MyAppVersion}

ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Common\VST3\{#MyBundleName}

DisableDirPage=yes
DisableProgramGroupPage=yes

UninstallFilesDir={localappdata}\DeviceOut\uninstall

DisableFinishedPage=no
UninstallDisplayName={#MyAppName} {#MyAppVersion}

CloseApplications=no
RestartApplications=no

SetupMutex={#MyAppName}Setup

OutputDir=out
OutputBaseFilename={#MyAppName}-Setup-{#MyAppVersion}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "chinese"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "..\target\bundled\{#MyBundleName}\*"; \
    DestDir: "{app}"; \
    Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\target\release\deviceout-updater.exe"; \
    DestDir: "{localappdata}\DeviceOut"; \
    DestName: "deviceout-updater.exe"; \
    Flags: ignoreversion

[Dirs]
Name: "{app}"; Flags: uninsalwaysuninstall
Name: "{app}\Contents"; Flags: uninsalwaysuninstall
Name: "{app}\Contents\x86_64-win"; Flags: uninsalwaysuninstall
Name: "{app}\Contents\Resources"; Flags: uninsalwaysuninstall

[UninstallDelete]
Type: dirifempty; Name: "{localappdata}\DeviceOut"

[Code]
const
  GENERIC_WRITE = $40000000;
  FILE_SHARE_READ = $00000001;
  FILE_SHARE_WRITE = $00000002;
  FILE_SHARE_DELETE = $00000004;
  OPEN_EXISTING = 3;
  INVALID_HANDLE_VALUE = -1;
  BINARY_REL = 'Contents\x86_64-win\{#MyBundleName}';

function CreateFileW(lpFileName: String; dwDesiredAccess, dwShareMode: Cardinal;
  lpSecurityAttributes: Cardinal; dwCreationDisposition, dwFlagsAndAttributes: Cardinal;
  hTemplateFile: Cardinal): Integer;
  external 'CreateFileW@kernel32.dll stdcall';

function CloseHandle(hObject: Integer): Integer;
  external 'CloseHandle@kernel32.dll stdcall';

function IsFileLocked(const FileName: String): Boolean;
var
  Handle: Integer;
begin
  if not FileExists(FileName) then begin
    Result := False;
    Exit;
  end;
  Handle := CreateFileW(FileName, GENERIC_WRITE,
    FILE_SHARE_READ or FILE_SHARE_WRITE or FILE_SHARE_DELETE,
    0, OPEN_EXISTING, 0, 0);
  if Handle = INVALID_HANDLE_VALUE then begin
    Result := True;
  end else begin
    CloseHandle(Handle);
    Result := False;
  end;
end;

function UserBundlePath(): String;
begin
  Result := ExpandConstant('{localappdata}') + '\Programs\Common\VST3\{#MyBundleName}';
end;

function MachineBundlePath(): String;
begin
  Result := ExpandConstant('{commoncf64}') + '\VST3\{#MyBundleName}';
end;

function InitializeSetup(): Boolean;
var
  Other: String;
  Msg: String;
begin
  Result := True;

  if IsAdminInstallMode() then
    Other := UserBundlePath()
  else
    Other := MachineBundlePath();

  if DirExists(Other) then
  begin
    Msg := '另一处已安装 DeviceOut：' + #13#10
         + Other + #13#10
         + '类 ID 相同，继续可能看不到本次更新。仍要继续？';
    Result := (SuppressibleMsgBox(Msg, mbConfirmation, MB_YESNO, IDYES) = IDYES);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    WizardForm.FinishedLabel.Caption :=
        'DeviceOut 已安装到：' + ExpandConstant('{app}') + #13#10
      + '重启 DAW -> 总线挂 DeviceOut -> 选输出设备';
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if IsFileLocked(ExpandConstant('{app}\' + BINARY_REL)) then
    Result := '插件正被宿主占用，无法覆盖。请关闭 DAW 或解除占用后重试。';
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  if IsFileLocked(ExpandConstant('{app}\' + BINARY_REL)) then
  begin
    SuppressibleMsgBox('插件正被宿主占用。请关闭 DAW 或解除占用后重新卸载。本次未改动任何文件。',
         mbError, MB_OK, IDOK);
    Result := False;
  end;
end;
