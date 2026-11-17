[Setup]
AppId={{886AE1C0-F529-4D04-BA23-A78E80BD5E6F}
AppName=TGOW Recorder
AppVersion=@VERSION@
AppVerName=TGOW Recorder @VERSION@
AppPublisher=T-GOW
AppPublisherURL=https://github.com/T-G0W/tgow-recorder
AppSupportURL=https://github.com/T-G0W/tgow-recorder/issues
AppUpdatesURL=https://github.com/T-G0W/tgow-recorder/releases
DefaultDirName={localappdata}\Programs\TGOW Recorder
DefaultGroupName=TGOW Recorder
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=@OUTDIR@
OutputBaseFilename=tgow-recorder-@VERSION@-windows-x64-setup
SetupIconFile=@ROOT@\assets\tgow-recorder.ico
UninstallDisplayIcon={app}\tgow-recorder.exe
UninstallDisplayName=TGOW Recorder
LicenseFile=@ROOT@\LICENSE-MIT
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
ChangesAssociations=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[CustomMessages]
english.StartWithWindows=Start TGOW Recorder when I sign in to Windows
german.StartWithWindows=TGOW Recorder beim Anmelden bei Windows starten
english.LaunchNow=Start TGOW Recorder now
german.LaunchNow=TGOW Recorder jetzt starten
english.Shortcuts=Shortcuts:
german.Shortcuts=Verknüpfungen:
english.Startup=Startup:
german.Startup=Autostart:

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:Shortcuts}"
Name: "autostart"; Description: "{cm:StartWithWindows}"; GroupDescription: "{cm:Startup}"; Flags: unchecked

[Files]
Source: "@STAGE@\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\TGOW Recorder"; Filename: "{app}\tgow-recorder.exe"; Parameters: "--show"; AppUserModelID: "T-GOW.TGOWRecorder"; Comment: "Record, replay and stream your screen"
Name: "{autodesktop}\TGOW Recorder"; Filename: "{app}\tgow-recorder.exe"; Parameters: "--show"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "TGOW Recorder"; ValueData: """{app}\tgow-recorder.exe"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\tgow-recorder.exe"; Parameters: "--show"; Description: "{cm:LaunchNow}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\tgow-recorder.exe"; Parameters: "ctl quit"; Flags: runhidden waituntilterminated skipifdoesntexist; RunOnceId: "StopRecorder"

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\T-GOW\tgow-recorder\replay"
