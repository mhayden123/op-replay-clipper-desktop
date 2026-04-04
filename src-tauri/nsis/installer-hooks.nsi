; OP Replay Clipper — NSIS Installer Hooks
; Called by Tauri's generated NSIS script at install/uninstall time.

!macro customInstall
  SetDetailsPrint both

  DetailPrint ""
  DetailPrint "Setting up OP Replay Clipper backend..."
  DetailPrint "This installs Python, Git, and the rendering pipeline."
  DetailPrint "Please wait — this may take a few minutes on first install."
  DetailPrint ""

  ; Create the log directory first
  CreateDirectory "$LOCALAPPDATA\op-replay-clipper"

  ; Log the install attempt for debugging
  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" w
  FileWrite $1 "NSIS Install started$\r$\n"
  FileWrite $1 "INSTDIR: $INSTDIR$\r$\n"
  FileWrite $1 "LOCALAPPDATA: $LOCALAPPDATA$\r$\n"

  ; Check that the bootstrap script exists
  IfFileExists "$INSTDIR\resources\bootstrap.ps1" 0 +3
    FileWrite $1 "bootstrap.ps1 found at: $INSTDIR\resources\bootstrap.ps1$\r$\n"
    Goto BootstrapExists
  FileWrite $1 "ERROR: bootstrap.ps1 NOT FOUND at: $INSTDIR\resources\bootstrap.ps1$\r$\n"
  FileWrite $1 "Listing INSTDIR\resources:$\r$\n"
  DetailPrint "WARNING: bootstrap.ps1 not found — app will set up on first launch."
  FileClose $1
  Goto BootstrapDone

  BootstrapExists:
  DetailPrint "Running bootstrap setup..."
  FileWrite $1 "Launching PowerShell bootstrap...$\r$\n"
  FileClose $1

  ; Run the bootstrap and redirect all output to the log file.
  ; Use ExecWait instead of nsExec to ensure the full script runs.
  ; -NoProfile skips loading the user's profile (faster, avoids interference).
  ; Redirect stdout+stderr to install.log via PowerShell's own redirection.
  nsExec::ExecToLog 'powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& {$ErrorActionPreference=\"Continue\"; & \"$INSTDIR\resources\bootstrap.ps1\" -Silent *>&1 | Tee-Object -FilePath \"$LOCALAPPDATA\op-replay-clipper\install.log\" -Append}"'
  Pop $0

  ; Log the exit code
  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" a
  FileSeek $1 0 END
  FileWrite $1 "$\r$\nNSIS: PowerShell exited with code: $0$\r$\n"
  FileClose $1

  ${If} $0 == 0
    DetailPrint ""
    DetailPrint "Backend setup completed successfully."
  ${ElseIf} $0 == "error"
    DetailPrint ""
    DetailPrint "WARNING: PowerShell could not be launched."
    DetailPrint "The app will attempt setup on first launch."
  ${Else}
    DetailPrint ""
    DetailPrint "WARNING: Backend setup exited with code $0."
    DetailPrint "The app will attempt setup on first launch."
    DetailPrint "See log: $LOCALAPPDATA\op-replay-clipper\install.log"
  ${EndIf}

  BootstrapDone:
  DetailPrint ""
!macroend


!macro customUninstall
  SetDetailsPrint both
  DetailPrint "Cleaning up OP Replay Clipper data..."

  ; Remove the clipper data directory
  RMDir /r "$LOCALAPPDATA\op-replay-clipper"

  ${If} ${Errors}
    DetailPrint "Note: Some files could not be removed (may be in use)."
  ${Else}
    DetailPrint "Clipper data removed."
  ${EndIf}

  DetailPrint ""
!macroend
