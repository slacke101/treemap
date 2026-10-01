#define AppName "TreeMap"
#define AppVersion "0.1.0"
#define AppPublisher "Castron"

[Setup]
AppId={{54A4D3F1-9335-40B6-AF23-44FCD153B31A}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL=https://github.com/slacke101/treemap
DefaultDirName={localappdata}\Programs\TreeMap
DefaultGroupName=TreeMap
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
UninstallDisplayIcon={app}\TreeMap.exe
SetupIconFile=..\TreeMapicon.ico
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
OutputBaseFilename=TreeMap-Setup-x64

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "..\target\release\DirMap.exe"; DestDir: "{app}"; DestName: "TreeMap.exe"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\BRANDING.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\docs\*"; DestDir: "{app}\docs"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\open-source-core\LICENSE"; DestDir: "{app}"; DestName: "LICENSE-open-source-core.txt"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\TreeMap"; Filename: "{app}\TreeMap.exe"; WorkingDir: "{app}"; IconFilename: "{app}\TreeMap.exe"
Name: "{userdesktop}\TreeMap"; Filename: "{app}\TreeMap.exe"; WorkingDir: "{app}"; IconFilename: "{app}\TreeMap.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\TreeMap.exe"; Description: "Launch TreeMap"; Flags: postinstall nowait skipifsilent