[Setup]
AppName=Corros
AppVersion=0.1.0
DefaultDirName={autopf}\Corros
DefaultGroupName=Corros
OutputDir=Output
OutputBaseFilename=CorrosSetup
Compression=lzma
SolidCompression=yes
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64

[Files]
Source: "target\release\corros.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "src\compiler.cro"; DestDir: "{app}"; Flags: ignoreversion
Source: "src\vm.cro"; DestDir: "{app}"; Flags: ignoreversion
Source: "src\cli.cro"; DestDir: "{app}"; Flags: ignoreversion
Source: "src\prelude.cro"; DestDir: "{app}"; Flags: ignoreversion
Source: "src\codegen.cro"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Corros"; Filename: "{app}\corros.exe"
Name: "{group}\Uninstall Corros"; Filename: "{uninstallexe}"

[Tasks]
Name: "envPath"; Description: "Add to PATH environment variable"; GroupDescription: "Additional Configuration:"

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: envPath; Check: NeedsAddPath(ExpandConstant('{app}'))

[Code]
function NeedsAddPath(Param: string): boolean;
var
  OrigPath: string;
begin
  if not RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', OrigPath)
  then begin
    Result := True;
    exit;
  end;
  // look for the path with leading and trailing semicolon
  // Pos() returns 0 if not found
  Result := Pos(';' + Param + ';', ';' + OrigPath + ';') = 0;
end;
