; Inno Setup script of the Windows installer. Built by scripts/make-windows-installer.ps1, which
; stages the app (bin\termigator.exe with its Qt/KF6 DLLs, QML and plugins; share\ with the Breeze
; colour schemes and fonts) and passes:
;   /DAppVersion=<x.y.z>  /DStage=<staged folder>  /DOutDir=<where the setup .exe goes>
; The wizard asks whether to install for this user only (no administrator rights,
; %LOCALAPPDATA%\Programs) or for all users (Program Files).

#ifndef AppVersion
  #error AppVersion is not set (/DAppVersion=...)
#endif
#ifndef Stage
  #error Stage is not set (/DStage=...)
#endif
#ifndef OutDir
  #define OutDir "."
#endif

[Setup]
AppId={{8F0C2B6E-5A0B-4E7E-9C1D-6B9E1F2A7C31}
AppName=Termigator
AppVersion={#AppVersion}
AppVerName=Termigator {#AppVersion}
AppPublisher=Termigator
AppPublisherURL=https://github.com/Ernold11/Termigator
AppSupportURL=https://github.com/Ernold11/Termigator/issues
AppUpdatesURL=https://github.com/Ernold11/Termigator/releases
DefaultDirName={autopf}\Termigator
DefaultGroupName=Termigator
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutDir}
OutputBaseFilename=termigator-{#AppVersion}-setup
SetupIconFile=..\..\assets\termigator.ico
LicenseFile=..\..\LICENSE
UninstallDisplayIcon={app}\bin\termigator.exe
UninstallDisplayName=Termigator
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "polish"; MessagesFile: "compiler:Languages\Polish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; An update replaces the whole app tree: DLLs, plugins or QML (bin\qml) dropped by a newer Qt/KF6
; must not linger next to the new ones. The uninstaller and the user's settings live elsewhere.
Type: filesandordirs; Name: "{app}\bin"
Type: filesandordirs; Name: "{app}\share"

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Termigator"; Filename: "{app}\bin\termigator.exe"; WorkingDir: "{userdocs}"
Name: "{autodesktop}\Termigator"; Filename: "{app}\bin\termigator.exe"; WorkingDir: "{userdocs}"; Tasks: desktopicon

[Run]
; One start without a window that quits at the first frame: the antivirus scans the DLLs and Qt
; builds its QML cache now, not at the user's first start.
Filename: "{app}\bin\termigator.exe"; Parameters: "--warm-up"; StatusMsg: "Preparing the first start..."; Flags: runhidden waituntilterminated
Filename: "{app}\bin\termigator.exe"; Description: "{cm:LaunchProgram,Termigator}"; Flags: nowait postinstall skipifsilent

; The user's settings (%APPDATA%\termigator) stay on uninstall, like on Linux.
