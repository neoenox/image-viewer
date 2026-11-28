; Inno Setup 6 script for image-viewer.
; Build with scripts/build-installer.ps1 (passes AppVersion/SourceExe/OutputDirectory).
; Save as UTF-8 with BOM.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\image-viewer.exe"
#endif
#ifndef OutputDirectory
  #define OutputDirectory "..\dist"
#endif

#define AppName "画像ビューワー"
#define AppExeName "image-viewer.exe"
#define ProgId "ImageViewer.App"

[Setup]
AppId={{2B9B3E7C-6A5D-4C21-9F8E-1D4A7C3B6E20}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=neoenox
AppPublisherURL=https://github.com/neoenox/image-viewer
AppSupportURL=https://github.com/neoenox/image-viewer/issues
DefaultDirName={localappdata}\Programs\ImageViewer
DefaultGroupName={#AppName}
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDirectory}
OutputBaseFilename=image-viewer-installer-{#AppVersion}
VersionInfoVersion={#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=no
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}
SetupIconFile=..\assets\icon.ico
ChangesAssociations=yes
SetupLogging=yes

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Tasks]
Name: "desktopicon"; Description: "デスクトップにショートカットを作成する"; Flags: unchecked
Name: "defaultassoc"; Description: "対応ファイルのダブルクリック既定にする"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{group}\{#AppName} をアンインストール"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueData: """{app}\{#AppExeName}"",0"
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\shell\open\command"; ValueType: string; ValueData: """{app}\{#AppExeName}"" ""%1"""
Root: HKCU; Subkey: "Software\ImageViewer\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#AppName}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\ImageViewer\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "{#AppName} 画像ビューワー"
Root: HKCU; Subkey: "Software\ImageViewer\Capabilities"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: "{app}\{#AppExeName},0"
Root: HKCU; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#AppName}"; ValueData: "Software\ImageViewer\Capabilities"; Flags: uninsdeletevalue

Root: HKCU; Subkey: "Software\Classes\.jpg\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".jpg"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.jpeg\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".jpeg"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.png\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".png"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.gif\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".gif"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.bmp\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".bmp"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.webp\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".webp"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.tif\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".tif"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.tiff\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".tiff"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.ico\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".ico"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.dds\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".dds"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.hdr\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".hdr"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.exr\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".exr"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.qoi\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".qoi"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.pbm\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".pbm"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.pgm\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".pgm"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.ppm\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".ppm"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\.pam\OpenWithProgids"; ValueType: string; ValueName: "{#ProgId}"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKCU; Subkey: "Software\Classes\Applications\{#AppExeName}\SupportedTypes"; ValueType: string; ValueName: ".pam"; ValueData: ""

; 任意タスク: ダブルクリックの既定にする
Root: HKCU; Subkey: "Software\Classes\.jpg"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.jpeg"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.png"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.gif"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.bmp"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.webp"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.tif"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.tiff"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.ico"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.dds"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.hdr"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.exr"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.qoi"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.pbm"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.pgm"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.ppm"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.pam"; ValueType: string; ValueData: "{#ProgId}"; Tasks: defaultassoc; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExeName}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
