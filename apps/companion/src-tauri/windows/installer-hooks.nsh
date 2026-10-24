; HealthierYou companion installer hooks (slice V1-6), included by the Tauri 2.12.3 NSIS
; template (bundle > windows > nsis > installerHooks). The template installs per user
; into %LOCALAPPDATA%\HealthierYou with no administrator rights; these hooks add start at
; sign-in and make the uninstall leave nothing behind but the user's own data.
;
; Folders under %LOCALAPPDATA%\HealthierYou:
;   hy-companion.exe, uninstall.exe, webview2-fixed\   the program (template)
;   updates\                                           update record, staged installer,
;                                                      previous-version copy (program state)
;   data\                                              personal data: kept unless the user
;                                                      ticks "Also delete my data" or a
;                                                      silent uninstall passes /DELETEDATA

!define HY_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define HY_STARTUP_APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"
!define HY_DATA_ROOT "$LOCALAPPDATA\HealthierYou"

!macro NSIS_HOOK_PREINSTALL
  ; An upgrade replaces the bundled WebView2 runtime as a whole, so no file of an older
  ; runtime is left behind (and the uninstaller, which lists only its own files, can
  ; remove the folder). The app must be closed first: the template checks that after
  ; this hook, so check it here too.
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
  RMDir /r "$INSTDIR\webview2-fixed"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Start at sign-in from the per-user Run key (architecture 4.1), unless the user switched
  ; it off in HealthierYou (the marker file), so a reinstall or an update keeps that
  ; choice. A choice made in Task Manager is kept by Windows itself (StartupApproved).
  ${IfNot} ${FileExists} "${HY_DATA_ROOT}\data\autostart-off"
    WriteRegStr HKCU "${HY_RUN_KEY}" "${PRODUCTNAME}" '"$INSTDIR\${MAINBINARYNAME}.exe"'
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Not while an update replaces the program (/UPDATE): then everything stays.
  ${If} $UpdateMode <> 1
    ; The runtime folder, whatever an older runtime left in it, then the program folder
    ; if that leaves it empty (the template tried before the runtime was gone).
    RMDir /r "$INSTDIR\webview2-fixed"
    RMDir "$INSTDIR"
    ; Program state, not personal data.
    RMDir /r "${HY_DATA_ROOT}\updates"
    ; The template removes the Run value; Windows' record of a Task Manager choice too.
    DeleteRegValue HKCU "${HY_RUN_KEY}" "${PRODUCTNAME}"
    DeleteRegValue HKCU "${HY_STARTUP_APPROVED_KEY}" "${PRODUCTNAME}"
    ; The install-location key the template writes on every install but removes only
    ; when the data box is ticked: the uninstall residue feasibility batch 1 found.
    DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"
    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty SHCTX "${MANUKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"
    ; WebView2's cache for this app. The pages keep nothing in browser storage, so this
    ; holds no personal data.
    RMDir /r "$LOCALAPPDATA\${BUNDLEID}"
    ; Personal data only when asked (unticked by default; architecture 8, Q7).
    ClearErrors
    ${GetOptions} $CMDLINE "/DELETEDATA" $R9
    ${IfNot} ${Errors}
      StrCpy $DeleteAppDataCheckboxState 1
    ${EndIf}
    ${If} $DeleteAppDataCheckboxState = 1
      RMDir /r "${HY_DATA_ROOT}\data"
    ${EndIf}
    ; The folder itself goes only if nothing is left in it (kept data keeps it).
    RMDir "${HY_DATA_ROOT}"
  ${EndIf}
!macroend
