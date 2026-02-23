; GamePadCC v2 — Inno Setup installer script
; Build:
;   1. cargo build --release
;   2. Open this file in Inno Setup Compiler (https://jrsoftware.org/isinfo.php)
;   3. Press Compile — output lands in installer\output\GamePadCC-Setup.exe

#define MyAppName      "GamePadCC"
#define MyAppVersion   "2.0"
#define MyAppPublisher "VeigaPunk"
#define MyAppURL       "https://github.com/VeigaPunk/GamePadCCv2"
#define MyAppExe       "gamepadcc.exe"
#define WisprURL       "https://wisprflow.ai"

; ── Setup ──────────────────────────────────────────────────────────────
[Setup]
AppId={{F3A2C1D4-8B7E-4F5A-9C6D-2E1B0A3F4C5D}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}

; Install to %LOCALAPPDATA%\GamePadCC — no UAC prompt needed
DefaultDirName={localappdata}\{#MyAppName}
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog

; Output
OutputDir=output
OutputBaseFilename=GamePadCC-Setup
SetupIconFile=

; Appearance
WizardStyle=modern
DisableProgramGroupPage=yes
DisableWelcomePage=no

; Compression
Compression=lzma2/ultra64
SolidCompression=yes
LZMAUseSeparateProcess=yes

; Uninstall
UninstallDisplayIcon={app}\{#MyAppExe}
UninstallDisplayName={#MyAppName}

; Version info embedded in the installer .exe
VersionInfoVersion=2.0.0.0
VersionInfoProductName={#MyAppName}
VersionInfoProductVersion=2.0

; ── Languages ──────────────────────────────────────────────────────────
[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

; ── Tasks (optional features shown as checkboxes) ──────────────────────
[Tasks]
; Auto-start: OFF by default — user must consciously enable it
Name: "autostart";  Description: "Start {#MyAppName} automatically when Windows starts"; \
                    GroupDescription: "Startup:"; Flags: unchecked
; Desktop shortcut: on by default, easy to opt out
Name: "desktopicon"; Description: "Create a desktop shortcut"; \
                     GroupDescription: "Additional icons:"; Flags: checkedonce
; Wispr Flow: off by default
Name: "wisprflow";  Description: "Open the Wispr Flow download page after install (required for Speech-to-Text)"; \
                    GroupDescription: "Speech-to-Text:"; Flags: unchecked

; ── Files ──────────────────────────────────────────────────────────────
[Files]
Source: "..\target\release\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion

; ── Icons (Start Menu + optional desktop) ──────────────────────────────
[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{autodesktop}\{#MyAppName}";  Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

; ── Registry ───────────────────────────────────────────────────────────
[Registry]
; Auto-start entry — only added when the task is checked; removed on uninstall
Root: HKCU; \
  Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; \
  ValueType: string; ValueName: "{#MyAppName}"; \
  ValueData: """{app}\{#MyAppExe}"""; \
  Flags: uninsdeletevalue; Tasks: autostart

; ── Post-install actions ───────────────────────────────────────────────
[Run]
; Launch the app when the user clicks "Finish" (optional tick, default on)
Filename: "{app}\{#MyAppExe}"; \
  Description: "Launch {#MyAppName} now"; \
  Flags: nowait postinstall skipifsilent

; Open Wispr Flow download page — only if the task was checked
Filename: "{#WisprURL}"; \
  Description: "Open Wispr Flow download page"; \
  Flags: shellexec postinstall skipifsilent; \
  Tasks: wisprflow

; ── Installer logic (Pascal) ───────────────────────────────────────────
[Code]

{ Check for WSL2 — needed for Tmux and Codex features.
  Not a hard requirement: the app works fine without it for basic mapping. }
function IsWSL2Present(): Boolean;
var
  ResultCode: Integer;
begin
  Result := Exec(ExpandConstant('{sys}\wsl.exe'), '--status', '',
                 SW_HIDE, ewWaitUntilTerminated, ResultCode)
            and (ResultCode = 0);
end;

procedure InitializeWizard();
begin
  if not IsWSL2Present() then
  begin
    MsgBox(
      'WSL2 was not detected on this machine.' + #13#10 + #13#10 +
      'GamePadCC will work normally for controller mapping,' + #13#10 +
      'but the Tmux and Codex (AI agent) integrations require WSL2.' + #13#10 + #13#10 +
      'You can install WSL2 at any time from the Microsoft Store, ' +
      'or by running:' + #13#10 +
      '    wsl --install' + #13#10 + #13#10 +
      'The installation will continue regardless.',
      mbInformation, MB_OK);
  end;
end;
