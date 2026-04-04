; OP Replay Clipper - NSIS Installer Hooks
; Uses registry key HKCU\Software\OP Replay Clipper to track install paths.

!macro customInstall
  SetDetailsPrint both

  DetailPrint ""
  DetailPrint "Setting up OP Replay Clipper backend..."
  DetailPrint ""

  ; Create log directory
  CreateDirectory "$LOCALAPPDATA\op-replay-clipper"

  ; Log diagnostic info
  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" w
  FileWrite $1 "=== NSIS Install Log ===$\r$\n"
  FileWrite $1 "INSTDIR: $INSTDIR$\r$\n"
  FileWrite $1 "LOCALAPPDATA: $LOCALAPPDATA$\r$\n"
  FileWrite $1 "$\r$\n"

  ; Check if bootstrap.ps1 exists
  FileWrite $1 "Looking for bootstrap.ps1...$\r$\n"
  FileWrite $1 "  Checking: $INSTDIR\resources\bootstrap.ps1$\r$\n"
  IfFileExists "$INSTDIR\resources\bootstrap.ps1" 0 ScriptNotInResources
    FileWrite $1 "  FOUND$\r$\n"
    Goto RunBootstrap

  ScriptNotInResources:
  FileWrite $1 "  NOT FOUND$\r$\n"
  FileWrite $1 "  Checking: $INSTDIR\bootstrap.ps1$\r$\n"
  IfFileExists "$INSTDIR\bootstrap.ps1" 0 ScriptNotFound
    FileWrite $1 "  FOUND$\r$\n"
    StrCpy $2 "$INSTDIR\bootstrap.ps1"
    Goto RunBootstrapAlt

  ScriptNotFound:
  FileWrite $1 "  NOT FOUND$\r$\n"
  FileWrite $1 "bootstrap.ps1 not found. App will set up on first launch.$\r$\n"
  FileClose $1
  DetailPrint "WARNING: bootstrap.ps1 not found. App will set up on first launch."
  Goto BootstrapDone

  RunBootstrap:
  StrCpy $2 "$INSTDIR\resources\bootstrap.ps1"

  RunBootstrapAlt:

  ; Detect previous installation — if exists, always run -Clean for a fresh state
  StrCpy $3 ""
  IfFileExists "$LOCALAPPDATA\op-replay-clipper\op-replay-clipper-native\clip.py" 0 NoPreviousInstall
    StrCpy $3 "-Clean"
    DetailPrint "Previous installation detected. Running clean reinstall."
  NoPreviousInstall:

  DetailPrint "Running: $2 $3"
  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" a
  FileSeek $1 0 END
  FileWrite $1 "Launching PowerShell: $2 $3$\r$\n"
  FileClose $1

  ; Write registry key for InstallPath (app exe location)
  WriteRegStr HKCU "Software\OP Replay Clipper" "InstallPath" "$INSTDIR"

  nsExec::ExecToLog 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$2" $3'
  Pop $0

  ; Log exit code
  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" a
  FileSeek $1 0 END
  FileWrite $1 "PowerShell exit code: $0$\r$\n"
  FileClose $1

  ${If} $0 == 0
    DetailPrint "Backend setup completed successfully."
  ${ElseIf} $0 == "error"
    DetailPrint "WARNING: PowerShell could not be launched."
    DetailPrint "The app will attempt setup on first launch."
  ${Else}
    DetailPrint "WARNING: Setup exited with code $0."
    DetailPrint "The app will attempt setup on first launch."
  ${EndIf}

  BootstrapDone:
  DetailPrint ""
!macroend


!macro customUninstall
  SetDetailsPrint both

  ; Confirmation dialog
  MessageBox MB_YESNO|MB_ICONQUESTION \
    "Remove all OP Replay Clipper data?$\r$\n$\r$\nThis deletes downloaded routes, rendered clips, openpilot data, and all backend files.$\r$\n$\r$\nChoose No to keep your data." \
    IDYES DoRemoveData
  Goto SkipDataRemoval

  DoRemoveData:

  ; Read ClipperHome from registry
  ReadRegStr $0 HKCU "Software\OP Replay Clipper" "ClipperHome"

  ${If} $0 != ""
    DetailPrint "Removing data directory (from registry): $0"
    RMDir /r "$0"
  ${EndIf}

  ; Also check common fallback locations in case registry is missing
  IfFileExists "$LOCALAPPDATA\op-replay-clipper\*.*" 0 +3
    DetailPrint "Removing: $LOCALAPPDATA\op-replay-clipper"
    RMDir /r "$LOCALAPPDATA\op-replay-clipper"

  IfFileExists "$PROFILE\.op-replay-clipper\*.*" 0 +3
    DetailPrint "Removing: $PROFILE\.op-replay-clipper"
    RMDir /r "$PROFILE\.op-replay-clipper"

  IfFileExists "$APPDATA\op-replay-clipper\*.*" 0 +3
    DetailPrint "Removing: $APPDATA\op-replay-clipper"
    RMDir /r "$APPDATA\op-replay-clipper"

  DetailPrint "Data removed."

  SkipDataRemoval:

  ; Always clean up registry
  DeleteRegKey HKCU "Software\OP Replay Clipper"
  DetailPrint "Registry cleaned."

  DetailPrint ""
!macroend
