; OP Replay Clipper — NSIS Installer Hooks
; Called by Tauri's generated NSIS script at install/uninstall time.

!macro customInstall
  ; Show progress in the installer detail log
  SetDetailsPrint both

  DetailPrint ""
  DetailPrint "Setting up OP Replay Clipper backend..."
  DetailPrint "This installs Python, Git, and the rendering pipeline."
  DetailPrint "Please wait — this may take a few minutes on first install."
  DetailPrint ""

  ; Run the PowerShell bootstrap script bundled as a resource.
  ; -ExecutionPolicy Bypass is needed because scripts are unsigned.
  ; -WindowStyle Hidden keeps the PowerShell window from flashing.
  DetailPrint "Running bootstrap setup..."
  nsExec::ExecToLog 'powershell.exe -ExecutionPolicy Bypass -WindowStyle Hidden -File "$INSTDIR\resources\bootstrap.ps1" -Silent'
  Pop $0

  ${If} $0 == 0
    DetailPrint ""
    DetailPrint "Backend setup completed successfully."
  ${Else}
    DetailPrint ""
    DetailPrint "WARNING: Backend setup exited with code $0."
    DetailPrint "The app will attempt setup on first launch."
    ; Don't fail the installer — the app has a first-run fallback
  ${EndIf}

  DetailPrint ""
!macroend


!macro customUninstall
  SetDetailsPrint both
  DetailPrint "Cleaning up OP Replay Clipper data..."

  ; Remove the clipper data directory
  ; Use LOCALAPPDATA since that's where bootstrap.ps1 puts everything
  RMDir /r "$LOCALAPPDATA\op-replay-clipper"

  ${If} ${Errors}
    DetailPrint "Note: Some files could not be removed (may be in use)."
  ${Else}
    DetailPrint "Clipper data removed."
  ${EndIf}

  DetailPrint ""
!macroend
