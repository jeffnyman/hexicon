; Hexicon's installer for Windows, made with Inno Setup 6
; (https://jrsoftware.org/isinfo.php). tools/build-installer.sh builds the
; programs and runs this; it gives the version and where the programs are:
;
;   ISCC /DAppVersion=0.1.0 /DBinDir=..\..\target\release hexicon.iss
;
; It installs for the user running it, with no administrator's rights: the
; player, which opens .hxg files, and, unless unticked, the compiler, on the
; user's PATH (the code at the end puts it there and takes it off).
; Settings > Apps uninstalls it.

#ifndef AppVersion
  #error AppVersion is not defined: run tools/build-installer.sh
#endif
#ifndef BinDir
  #define BinDir "..\..\target\release"
#endif

[Setup]
AppId={{CEA80D27-DFA7-484F-AFAB-C75B1C7BC229}
AppName=Hexicon
AppVersion={#AppVersion}
AppVerName=Hexicon {#AppVersion}
DefaultDirName={autopf}\Hexicon
DefaultGroupName=Hexicon
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
ChangesEnvironment=yes
OutputDir=..\..\build\installer
OutputBaseFilename=hexicon-{#AppVersion}-setup
SetupIconFile=..\..\crates\hexicon-app\icon\hexicon.ico
UninstallDisplayIcon={app}\hexicon-player.exe
UninstallDisplayName=Hexicon
WizardStyle=modern
Compression=lzma2
SolidCompression=yes

[Types]
Name: "full"; Description: "The player and the compiler"
Name: "player"; Description: "The player alone"
Name: "custom"; Description: "Choose"; Flags: iscustom

[Components]
Name: "player"; Description: "Hexicon Player, which plays games (.hxg files)"; Types: full player custom; Flags: fixed
Name: "compiler"; Description: "The compiler, hexicon, on the PATH, for writing games"; Types: full

[Tasks]
Name: "desktopicon"; Description: "A shortcut on the desktop"; Flags: unchecked

[Files]
Source: "{#BinDir}\hexicon-player.exe"; DestDir: "{app}"; Components: player; Flags: ignoreversion
Source: "{#BinDir}\hexicon.exe"; DestDir: "{app}\bin"; Components: compiler; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"
Source: "..\..\LIBRARY-EXCEPTION"; DestDir: "{app}"; DestName: "LIBRARY-EXCEPTION.txt"; Components: compiler

[Icons]
Name: "{autoprograms}\Hexicon Player"; Filename: "{app}\hexicon-player.exe"
Name: "{autodesktop}\Hexicon Player"; Filename: "{app}\hexicon-player.exe"; Tasks: desktopicon

[Registry]
; A game's file opens in the player, with the player's icon.
; The uninstaller goes through these from the last up, so a key is
; emptied before the key it is in is looked at.
Root: HKA; Subkey: "Software\Classes\.hxg"; ValueType: string; ValueName: ""; ValueData: "Hexicon.Game"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\.hxg\OpenWithProgids"; ValueType: string; ValueName: "Hexicon.Game"; ValueData: ""; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\Hexicon.Game"; ValueType: string; ValueName: ""; ValueData: "Hexicon game"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Hexicon.Game\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\hexicon-player.exe,0"
Root: HKA; Subkey: "Software\Classes\Hexicon.Game\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\hexicon-player.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\hexicon-player.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\hexicon-player.exe\SupportedTypes"; ValueType: string; ValueName: ".hxg"; ValueData: ""

[Run]
Filename: "{app}\hexicon-player.exe"; Description: "Start Hexicon Player"; Flags: nowait postinstall skipifsilent

[Code]
// Whether a folder is on the user's PATH already.
function NotOnPath(Folder: string): Boolean;
var
  Path: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Path) then
    Result := True
  else
    Result := Pos(';' + Uppercase(Folder) + ';', ';' + Uppercase(Path) + ';') = 0;
end;

// Put a folder at the end of the user's PATH, unless it is there. A PATH
// that ends in a semicolon keeps one at the end, so that taking the
// folder off again leaves it as it was.
procedure AddToPath(Folder: string);
var
  Path: string;
begin
  if not NotOnPath(Folder) then
    exit;
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Path) then
    Path := '';
  if Path = '' then
    Path := Folder
  else if Path[Length(Path)] = ';' then
    Path := Path + Folder + ';'
  else
    Path := Path + ';' + Folder;
  RegWriteExpandStringValue(HKCU, 'Environment', 'Path', Path);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsComponentSelected('compiler') then
    AddToPath(ExpandConstant('{app}\bin'));
end;

// Take a folder off the user's PATH, wherever it is in it.
procedure RemoveFromPath(Folder: string);
var
  Path, Padded: string;
  At: Integer;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Path) then
    exit;
  Padded := ';' + Path + ';';
  At := Pos(';' + Uppercase(Folder) + ';', Uppercase(Padded));
  if At = 0 then
    exit;
  Delete(Padded, At, Length(Folder) + 1);
  Path := Copy(Padded, 2, Length(Padded) - 2);
  RegWriteExpandStringValue(HKCU, 'Environment', 'Path', Path);
end;

// What Explorer records once a game has been opened from it: Hexicon's
// name among the programs for .hxg, which goes, and the keys around it
// if nothing else is in them.
procedure ForgetExplorersRecord;
var
  Key: string;
begin
  Key := 'Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.hxg';
  RegDeleteValue(HKCU, Key + '\OpenWithProgids', 'Hexicon.Game');
  RegDeleteKeyIfEmpty(HKCU, Key + '\OpenWithProgids');
  RegDeleteKeyIfEmpty(HKCU, Key + '\OpenWithList');
  RegDeleteKeyIfEmpty(HKCU, Key);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RemoveFromPath(ExpandConstant('{app}\bin'));
    ForgetExplorersRecord;
  end;
end;
