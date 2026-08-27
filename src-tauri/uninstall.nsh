; T356 — make the uninstaller's "delete application data" checkbox mean what it says.
;
; **The checkbox already exists and already lies.** Tauri's own uninstaller shows it and, when
; it is ticked, removes `$APPDATA\${BUNDLEID}` and `$LOCALAPPDATA\${BUNDLEID}` — for this
; application, `ru.vrcast.studio`. Measured on a real installation, 2026-08-27:
;
;   %LOCALAPPDATA%\ru.vrcast.studio      276 files — the webview's cache
;   %APPDATA%\ru.vrcast.studio           does not exist
;   %APPDATA%\VRCast\VRCast Studio       6 files, 133.6 MB — THE DATA
;
; That last line is the server profiles, the settings, the library cache and both place
; tables. It is what a person means when they tick the box, and it is the one thing the box
; does not touch: they tick it, the uninstaller reports success, and everything stays on disk
; for ever. A promise that looks kept (constitution, principle II).
;
; The path is what `directories::ProjectDirs::from("ru", "VRCast", "VRCast Studio")` produces
; on Windows, and it is written out here rather than derived, because NSIS cannot ask the
; application anything — it runs when the application is already going away.
;
; **The `$UpdateMode` guard is not optional and not obvious.** The installer of a new version
; runs the OLD uninstaller itself and passes it `/UPDATE`. Without this condition every update
; would silently take every server profile with it — the same shape of loss the checkbox is
; supposed to be an explicit choice about.

!macro NSIS_HOOK_PREUNINSTALL
  ; Runs before anything is removed, and after the confirmation page — so the checkbox's
  ; state is already known here. Checked against the stock uninstaller's own template.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    ; **Which user's `$APPDATA`.** The hook is inserted at the very top of the uninstall
    ; section, before the stock code reaches its own deletion — and the stock code sets this
    ; explicitly there rather than trusting whatever the context happens to be. Following it
    ; costs one line and removes the question; left out, `$APPDATA` could resolve to the
    ; all-users profile, our directory would not be there, and the removal would report
    ; success having deleted nothing. Read off the generated installer.nsi, 2026-08-27.
    SetShellVarContext current
    DetailPrint "Removing VRCast Studio data"
    RMDir /r "$APPDATA\VRCast\VRCast Studio"
    RMDir /r "$LOCALAPPDATA\VRCast\VRCast Studio"
    ; The organisation folder only if it is now empty: another application of the same
    ; publisher would live beside it, and taking the parent would take theirs too.
    RMDir "$APPDATA\VRCast"
    RMDir "$LOCALAPPDATA\VRCast"
  ${EndIf}
!macroend
