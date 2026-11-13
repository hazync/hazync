; Hazync-Setup.exe: the Windows app as a normal installer (2026-11-13).
;
; Built by .github/workflows/windows-installer.yml:
;   iscc /DAppVersion=0.25.0 /DAppDir=<dist\Hazync> /DProverDir=<host.exe + cudart64_12.dll> /DWorker=<hazync-worker> hazync.iss
;
; What a person gets: Hazync in Start and on the desktop, under "Apps" with an uninstaller, with no admin prompt
; (installed for the current user, in %LOCALAPPDATA%\Programs\Hazync), and nothing else to fetch: the window, the
; worker and the GPU prover are all inside.
;   {app}\Hazync.exe + _internal\      the app (PyInstaller one-folder build of hazync_gui.py)
;   {app}\hazync-worker                the client the app runs (firstrun.install_dir() finds it)
;   {app}\prover\host.exe, cudart*.dll the release's Windows CUDA prover (found first by firstrun.candidate_dirs())
;
; ⛔ THE SIGNING KEY IS NEVER TOUCHED. It lives in %USERPROFILE%\.hazync (the only thing that ties proven blocks to
; a name); this installs and uninstalls only {app}. Settings live there too, so they survive an upgrade.
; ⚠ Not Authenticode-signed (owner's decision, 11-13): Windows SmartScreen shows "Windows protected your PC" until
; it is (More info -> Run anyway). The installer's sha256 is in the release's PGP-signed SHA256SUMS.txt.

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef AppDir
  #define AppDir "..\dist\Hazync"
#endif
#ifndef ProverDir
  #define ProverDir "prover"
#endif
#ifndef Worker
  #define Worker "hazync-worker"
#endif

[Setup]
; a fixed id: an upgrade replaces the installed version instead of adding a second entry
AppId={{6F1C2A57-3B8E-4E0D-9A61-48A1C5E7B2D3}
AppName=Hazync
AppVersion={#AppVersion}
AppVerName=Hazync {#AppVersion}
AppPublisher=Hazync
AppPublisherURL=https://hazync.org
AppSupportURL=https://github.com/hazync/hazync/issues
DefaultDirName={localappdata}\Programs\Hazync
DefaultGroupName=Hazync
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19044
; the release signs only hazync-* assets, under a fixed name (scripts/release-assets.txt); the version is inside
OutputBaseFilename=hazync-setup-x86_64-windows
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=Hazync
UninstallDisplayIcon={app}\Hazync.exe
; the app (and its workers) must be closed before files are replaced
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "{#AppDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Worker}"; DestDir: "{app}"; DestName: "hazync-worker"; Flags: ignoreversion
Source: "{#ProverDir}\host.exe"; DestDir: "{app}\prover"; Flags: ignoreversion
Source: "{#ProverDir}\cudart64_*.dll"; DestDir: "{app}\prover"; Flags: ignoreversion

[Icons]
Name: "{group}\Hazync"; Filename: "{app}\Hazync.exe"
Name: "{group}\Uninstall Hazync"; Filename: "{uninstallexe}"
Name: "{userdesktop}\Hazync"; Filename: "{app}\Hazync.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Hazync.exe"; Description: "Open Hazync"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; only what the app itself writes inside {app}; %USERPROFILE%\.hazync (the key) is never touched
Type: filesandordirs; Name: "{app}\_internal\__pycache__"
