; CC Reminder NSIS uninstall extras.
; Included AFTER the MUI pages are declared and BEFORE the uninstall section
; that !insertmacros NSIS_HOOK_PREUNINSTALL.
;
; Adds to the stock Tauri uninstaller:
;   a PREUNINSTALL hook that stops the running app first (its self-heal loop
;   would reinstall freshly-removed hooks otherwise) and, when the confirm-page
;   checkbox is ticked, invokes the app's --uninstall-hooks CLI, which runs the
;   exact same uninstall transaction the GUI uses.
;
; The checkbox itself lives in installer.nsi (un.ConfirmShow) because its
; label needs $LANGUAGE at page-show time.

!macro NSIS_HOOK_PREUNINSTALL
  ; Stop the running app BEFORE removing hooks: its self-heal loop would
  ; otherwise reinstall what we just removed. /F because the tray keeps the
  ; app alive after the window closes.
  nsExec::Exec 'taskkill /IM "${MAINBINARYNAME}.exe" /F'
  Pop $0
  ; Give the process and its file handles a moment to go away.
  Sleep 800

  ${If} $CcUninstallHooksCheckboxState = 1
    nsExec::ExecToLog '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-hooks'
    Pop $0
  ${EndIf}
!macroend
