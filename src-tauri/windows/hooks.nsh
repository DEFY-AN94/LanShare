; LanShare installer hooks (NSIS).
; Allow inbound connections to LanShare from the local subnet on Private
; networks only. On a Public network Windows keeps blocking it, and the app
; tells the user to switch the network profile to Private.
; The installer runs per-machine (elevated), which netsh needs.

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec 'netsh advfirewall firewall delete rule name="LanShare"'
  nsExec::Exec 'netsh advfirewall firewall add rule name="LanShare" dir=in action=allow program="$INSTDIR\${MAINBINARYNAME}.exe" enable=yes profile=private remoteip=localsubnet protocol=TCP'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  nsExec::Exec 'netsh advfirewall firewall delete rule name="LanShare"'
  ; Launch at login: Tauri's uninstaller removes the Run value, also before
  ; an upgrade, and the new version puts it back. Task Manager's record of it
  ; (StartupApproved) stays, so turning it off there survives an upgrade.
!macroend
