; T356 — the uninstaller hook's truth table, run rather than read.
;
; **Why this exists.** The branch that deletes application data can only be reached by ticking
; a checkbox on a dialog: there is no command-line flag for it, so a silent uninstall can never
; exercise it. Reading the macro and declaring it correct is exactly the kind of proof this
; project does not accept — and the case that matters most is the one that is hardest to
; believe from reading:
;
;   an installer of a NEW version runs the OLD uninstaller itself, passing `/UPDATE`.
;
; Get that wrong and every update quietly takes every server profile with it. So all four
; combinations are compiled from the real `uninstall.nsh` and run.
;
; The macro names a real path — it has to, the uninstaller cannot ask the application anything
; — so the harness that drives this backs the real directory up first and puts it back after.
; See `tests/uninstall-hook/run.ps1`.

Name "hook truth table"
OutFile "$%HOOK_TEST_OUT%"
SilentInstall silent
RequestExecutionLevel user

!include LogicLib.nsh

; Declared here because in the real uninstaller the stock template declares them. The macro
; under test only reads them, which is the whole reason it can be exercised this way.
Var DeleteAppDataCheckboxState
Var UpdateMode

; The file under test, unchanged and not copied: a copy would drift from the original the day
; one of them is edited, and the check would then guard a macro nobody ships.
!include "..\..\uninstall.nsh"

Section
  ; Both values come in from the environment at compile time: four builds, four cases.
  StrCpy $DeleteAppDataCheckboxState "$%HOOK_TEST_CHECKBOX%"
  StrCpy $UpdateMode "$%HOOK_TEST_UPDATE%"
  !insertmacro NSIS_HOOK_PREUNINSTALL
SectionEnd
