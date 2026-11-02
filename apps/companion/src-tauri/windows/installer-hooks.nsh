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

; Sets `outvar` to 1 while a process runs this install's bundled WebView2 runtime
; (webview2-fixed\msedgewebview2.exe), else 0. Only HealthierYou runs that private copy of
; the runtime, so these are exactly this app's WebView2 processes (other apps' WebView2
; processes run another copy and are never counted). Asks the Restart Manager, as the
; template does for the app itself. Uses $0, $1, $2, $3 and $5.
!macro HY_RUNTIME_IN_USE outvar
  StrCpy ${outvar} 0
  ${If} ${FileExists} "$INSTDIR\webview2-fixed\msedgewebview2.exe"
    !insertmacro RestartManager_StartSession $1
    ${If} $1 != ""
      !insertmacro RestartManager_RegisterFile $1 "$INSTDIR\webview2-fixed\msedgewebview2.exe"
      ${If} $0 = 0
        ; ERROR_MORE_DATA (234): at least one process uses the file.
        System::Call 'RSTRTMGR::RmGetList(i r1, *i .r2, *i .r3, p 0, *i .r5) i .r0'
        ${If} $0 = 234
          StrCpy ${outvar} 1
        ${EndIf}
      ${EndIf}
      !insertmacro RestartManager_EndSession $1
    ${EndIf}
  ${EndIf}
!macroend

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
    ; I9a (docs/V1_Acceptance.md): the template has closed the app by now (or it was not
    ; running), but its WebView2 processes end on their own a little later and keep files
    ; of the runtime and of the WebView2 cache open until then; a scanner can also hold a
    ; new file for a moment. So wait for every WebView2 process of this app to end (at
    ; most 30 s), close any still running after that, then remove the runtime and the
    ; cache, retrying for at most 30 s. $R8 counts the tries ($R9 is used below).
    StrCpy $R8 0
    ${Do}
      !insertmacro HY_RUNTIME_IN_USE $4
      ${If} $4 = 0
      ${OrIf} $R8 >= 60
        ${ExitDo}
      ${EndIf}
      Sleep 500
      IntOp $R8 $R8 + 1
    ${Loop}
    ${If} $4 = 1
      !insertmacro RestartManager_ShutdownFile "$INSTDIR\webview2-fixed\msedgewebview2.exe" $0
      Sleep 1000
    ${EndIf}
    ; The runtime folder, whatever an older runtime left in it, and WebView2's cache for
    ; this app (the pages keep nothing in browser storage, so it holds no personal data),
    ; then the program folder if that leaves it empty (the template tried before the
    ; runtime was gone).
    StrCpy $R8 0
    ${Do}
      RMDir /r "$INSTDIR\webview2-fixed"
      RMDir /r "$LOCALAPPDATA\${BUNDLEID}"
      ${IfNot} ${FileExists} "$INSTDIR\webview2-fixed\*.*"
      ${AndIfNot} ${FileExists} "$LOCALAPPDATA\${BUNDLEID}\*.*"
        ${ExitDo}
      ${EndIf}
      ${If} $R8 >= 60
        ${ExitDo}
      ${EndIf}
      Sleep 500
      IntOp $R8 $R8 + 1
    ${Loop}
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
