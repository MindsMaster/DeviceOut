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
ShowLanguageDialog=no
LanguageDetectionMethod=uilanguage

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "zh_hans"; MessagesFile: "languages\ChineseSimplified.isl"
Name: "zh_hant"; MessagesFile: "languages\ChineseTraditional.isl"
Name: "ja"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "ko"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "pt_br"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "th"; MessagesFile: "compiler:Languages\Thai.isl"

[CustomMessages]
en.OtherInstall=DeviceOut is also installed at:%n%1%nBoth copies share the same class ID, so the host may keep loading the other one. Continue anyway?
en.Finished=DeviceOut was installed to:%n%1%nRestart the DAW, insert DeviceOut on a bus, then pick the output device.
en.LockedInstall=The plugin is in use by a host and cannot be replaced. Close the DAW and try again.
en.LockedUninstall=The plugin is in use by a host. Close the DAW and run the uninstaller again. Nothing was changed.

zh_hans.OtherInstall=另一处已安装 DeviceOut：%n%1%n两处的类 ID 相同，宿主可能继续加载另一份。仍要继续？
zh_hans.Finished=DeviceOut 已安装到：%n%1%n重启 DAW，在总线上插入 DeviceOut，再选择输出设备。
zh_hans.LockedInstall=插件正被宿主占用，无法覆盖。请关闭 DAW 后重试。
zh_hans.LockedUninstall=插件正被宿主占用。请关闭 DAW 后重新卸载。本次未改动任何文件。

zh_hant.OtherInstall=另一處已安裝 DeviceOut：%n%1%n兩處的類別 ID 相同，宿主可能繼續載入另一份。仍要繼續？
zh_hant.Finished=DeviceOut 已安裝到：%n%1%n重新啟動 DAW，在匯流排上插入 DeviceOut，再選擇輸出裝置。
zh_hant.LockedInstall=外掛正被宿主使用，無法覆寫。請關閉 DAW 後重試。
zh_hant.LockedUninstall=外掛正被宿主使用。請關閉 DAW 後重新解除安裝。本次未變更任何檔案。

ja.OtherInstall=DeviceOut は別の場所にもインストールされています：%n%1%n両方が同じクラス ID を持つため、ホストがもう一方を読み込み続ける可能性があります。続行しますか？
ja.Finished=DeviceOut を次の場所にインストールしました：%n%1%nDAW を再起動し、バスに DeviceOut を挿入して出力デバイスを選んでください。
ja.LockedInstall=プラグインがホストで使用中のため置き換えられません。DAW を終了してからやり直してください。
ja.LockedUninstall=プラグインがホストで使用中です。DAW を終了してから再度アンインストールしてください。何も変更されていません。

ko.OtherInstall=DeviceOut이 다른 위치에도 설치되어 있습니다:%n%1%n두 사본이 같은 클래스 ID를 사용하므로 호스트가 다른 사본을 계속 불러올 수 있습니다. 계속할까요?
ko.Finished=DeviceOut이 다음 위치에 설치되었습니다:%n%1%nDAW를 다시 시작하고 버스에 DeviceOut을 삽입한 뒤 출력 장치를 선택하세요.
ko.LockedInstall=플러그인이 호스트에서 사용 중이어서 교체할 수 없습니다. DAW를 종료한 뒤 다시 시도하세요.
ko.LockedUninstall=플러그인이 호스트에서 사용 중입니다. DAW를 종료한 뒤 제거를 다시 실행하세요. 아무것도 변경되지 않았습니다.

de.OtherInstall=DeviceOut ist auch hier installiert:%n%1%nBeide Kopien haben dieselbe Klassen-ID, der Host lädt möglicherweise weiterhin die andere. Trotzdem fortfahren?
de.Finished=DeviceOut wurde installiert nach:%n%1%nStarte die DAW neu, füge DeviceOut auf einem Bus ein und wähle das Ausgabegerät.
de.LockedInstall=Das Plugin wird von einem Host verwendet und kann nicht ersetzt werden. Schließe die DAW und versuche es erneut.
de.LockedUninstall=Das Plugin wird von einem Host verwendet. Schließe die DAW und starte die Deinstallation erneut. Es wurde nichts geändert.

fr.OtherInstall=DeviceOut est aussi installé ici :%n%1%nLes deux copies partagent le même identifiant de classe ; l'hôte peut continuer à charger l'autre. Continuer quand même ?
fr.Finished=DeviceOut a été installé dans :%n%1%nRedémarrez la DAW, insérez DeviceOut sur un bus, puis choisissez le périphérique de sortie.
fr.LockedInstall=Le plugin est utilisé par un hôte et ne peut pas être remplacé. Fermez la DAW puis réessayez.
fr.LockedUninstall=Le plugin est utilisé par un hôte. Fermez la DAW puis relancez la désinstallation. Rien n'a été modifié.

es.OtherInstall=DeviceOut también está instalado en:%n%1%nAmbas copias comparten el mismo ID de clase, por lo que el host puede seguir cargando la otra. ¿Continuar de todos modos?
es.Finished=DeviceOut se instaló en:%n%1%nReinicia el DAW, inserta DeviceOut en un bus y elige el dispositivo de salida.
es.LockedInstall=Un host está usando el plugin y no se puede reemplazar. Cierra el DAW e inténtalo de nuevo.
es.LockedUninstall=Un host está usando el plugin. Cierra el DAW y vuelve a ejecutar el desinstalador. No se cambió nada.

pt_br.OtherInstall=O DeviceOut também está instalado em:%n%1%nAs duas cópias compartilham o mesmo ID de classe, então o host pode continuar carregando a outra. Continuar mesmo assim?
pt_br.Finished=O DeviceOut foi instalado em:%n%1%nReinicie a DAW, insira o DeviceOut em um bus e escolha o dispositivo de saída.
pt_br.LockedInstall=O plugin está em uso por um host e não pode ser substituído. Feche a DAW e tente novamente.
pt_br.LockedUninstall=O plugin está em uso por um host. Feche a DAW e execute a desinstalação novamente. Nada foi alterado.

ru.OtherInstall=DeviceOut также установлен здесь:%n%1%nОбе копии имеют один и тот же идентификатор класса, поэтому хост может продолжать загружать другую. Всё равно продолжить?
ru.Finished=DeviceOut установлен в:%n%1%nПерезапустите DAW, вставьте DeviceOut на шину и выберите устройство вывода.
ru.LockedInstall=Плагин используется хостом, заменить его нельзя. Закройте DAW и повторите попытку.
ru.LockedUninstall=Плагин используется хостом. Закройте DAW и запустите удаление снова. Ничего не изменено.

th.OtherInstall=DeviceOut ถูกติดตั้งไว้ที่อื่นด้วย:%n%1%nทั้งสองสำเนาใช้ Class ID เดียวกัน โฮสต์อาจยังโหลดสำเนาอีกชุด ต้องการดำเนินการต่อหรือไม่
th.Finished=ติดตั้ง DeviceOut ไว้ที่:%n%1%nเปิด DAW ใหม่ ใส่ DeviceOut ลงในบัส แล้วเลือกอุปกรณ์เอาต์พุต
th.LockedInstall=ปลั๊กอินกำลังถูกโฮสต์ใช้งานอยู่ จึงแทนที่ไม่ได้ โปรดปิด DAW แล้วลองอีกครั้ง
th.LockedUninstall=ปลั๊กอินกำลังถูกโฮสต์ใช้งานอยู่ โปรดปิด DAW แล้วเรียกตัวถอนการติดตั้งอีกครั้ง ยังไม่มีการเปลี่ยนแปลงใดๆ

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
begin
  Result := True;

  if IsAdminInstallMode() then
    Other := UserBundlePath()
  else
    Other := MachineBundlePath();

  if DirExists(Other) then
    Result := (SuppressibleMsgBox(FmtMessage(CustomMessage('OtherInstall'), [Other]),
                 mbConfirmation, MB_YESNO, IDYES) = IDYES);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    WizardForm.FinishedLabel.Caption :=
      FmtMessage(CustomMessage('Finished'), [ExpandConstant('{app}')]);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if IsFileLocked(ExpandConstant('{app}\' + BINARY_REL)) then
    Result := CustomMessage('LockedInstall');
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
  if IsFileLocked(ExpandConstant('{app}\' + BINARY_REL)) then
  begin
    SuppressibleMsgBox(CustomMessage('LockedUninstall'), mbError, MB_OK, IDOK);
    Result := False;
  end;
end;
