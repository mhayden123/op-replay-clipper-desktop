; OP Replay Clipper — NSIS Installer Hooks
; Runs bootstrap.ps1 during install, cleans up on uninstall.
; All output logged to %LOCALAPPDATA%\op-replay-clipper\install.log

!macro customInstall
  SetDetailsPrint both

  DetailPrint ""
  DetailPrint "Setting up OP Replay Clipper backend..."
  DetailPrint ""

  ; Create log directory
  CreateDirectory "$LOCALAPPDATA\op-replay-clipper"

  ; Start the log file with diagnostic info
  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" w
  FileWrite $1 "=== NSIS Install Log ===$\r$\n"
  FileWrite $1 "Date: ${__DATE__} ${__TIME__}$\r$\n"
  FileWrite $1 "INSTDIR: $INSTDIR$\r$\n"
  FileWrite $1 "LOCALAPPDATA: $LOCALAPPDATA$\r$\n"
  FileWrite $1 "WINDIR: $WINDIR$\r$\n"
  FileWrite $1 "$\r$\n"

  ; Check if bootstrap.ps1 exists at the expected path
  FileWrite $1 "Looking for bootstrap.ps1...$\r$\n"
  FileWrite $1 "  Checking: $INSTDIR\resources\bootstrap.ps1$\r$\n"
  IfFileExists "$INSTDIR\resources\bootstrap.ps1" 0 ScriptNotInResources
    FileWrite $1 "  FOUND: $INSTDIR\resources\bootstrap.ps1$\r$\n"
    Goto RunBootstrap

  ScriptNotInResources:
  FileWrite $1 "  NOT FOUND$\r$\n"
  FileWrite $1 "  Checking: $INSTDIR\bootstrap.ps1$\r$\n"
  IfFileExists "$INSTDIR\bootstrap.ps1" 0 ScriptNotInRoot
    FileWrite $1 "  FOUND: $INSTDIR\bootstrap.ps1$\r$\n"
    StrCpy $2 "$INSTDIR\bootstrap.ps1"
    Goto RunBootstrapAlt

  ScriptNotInRoot:
  FileWrite $1 "  NOT FOUND$\r$\n"
  FileWrite $1 "$\r$\n"

  ; List what IS in the install directory for debugging
  FileWrite $1 "Contents of $INSTDIR:$\r$\n"
  FileClose $1
  nsExec::ExecToLog 'cmd.exe /c "dir /b "$INSTDIR" >> "$LOCALAPPDATA\op-replay-clipper\install.log" 2>&1"'
  Pop $0
  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" a
  FileSeek $1 0 END
  FileWrite $1 "$\r$\nContents of $INSTDIR\resources:$\r$\n"
  FileClose $1
  nsExec::ExecToLog 'cmd.exe /c "dir /b "$INSTDIR\resources" >> "$LOCALAPPDATA\op-replay-clipper\install.log" 2>&1"'
  Pop $0

  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" a
  FileSeek $1 0 END
  FileWrite $1 "$\r$\nbootstrap.ps1 not found in any location.$\r$\n"
  FileWrite $1 "The app will download and run it on first launch.$\r$\n"
  FileClose $1
  DetailPrint "WARNING: bootstrap.ps1 not found — app will set up on first launch."
  Goto BootstrapDone

  RunBootstrap:
  StrCpy $2 "$INSTDIR\resources\bootstrap.ps1"

  RunBootstrapAlt:
  DetailPrint "Running: $2"
  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" a
  FileSeek $1 0 END
  FileWrite $1 "$\r$\nLaunching PowerShell:$\r$\n"
  FileWrite $1 "  Script: $2$\r$\n"
  FileWrite $1 "  Command: powershell.exe -NoProfile -ExecutionPolicy Bypass -File $\"$2$\"$\r$\n"
  FileClose $1

  ; Run PowerShell with output captured to the NSIS log.
  ; -NoProfile avoids loading user PS profile (faster, fewer errors).
  ; The bootstrap script uses Start-Transcript internally for its own log.
  nsExec::ExecToLog 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$2"'
  Pop $0

  ; Record the exit code
  FileOpen $1 "$LOCALAPPDATA\op-replay-clipper\install.log" a
  FileSeek $1 0 END
  FileWrite $1 "$\r$\nPowerShell exit code: $0$\r$\n"
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
  DetailPrint "Cleaning up OP Replay Clipper data..."

  RMDir /r "$LOCALAPPDATA\op-replay-clipper"

  ${If} ${Errors}
    DetailPrint "Note: Some files could not be removed (may be in use)."
  ${Else}
    DetailPrint "Clipper data removed."
  ${EndIf}

  DetailPrint ""
!macroend
