; HealthierYou Server installer (slice V1-6b). Per machine, run by an administrator on
; the office server PC. It:
;   - installs hy-server.exe to Program Files and registers the Windows service
;     HealthierYouServer under its virtual account NT SERVICE\HealthierYouServer
;     (V1-5 contract 6.2), started automatically (delayed) and restarted after a failure;
;   - creates %ProgramData%\HealthierYou\Server open only to SYSTEM, Administrators and
;     the service (inheritance removed). The service makes its keys there at its first
;     start, protected with machine-scope DPAPI;
;   - creates the control-log mirror folder on another volume (/MIRROR=, contract 7.3);
;   - opens TCP 8443 inbound for hy-server.exe on the private and domain profiles only.
;     The admin page (8444) listens on loopback only and gets no rule;
;   - on an upgrade, has the INSTALLED server check this installer's .sig first
;     (hy-server verify-release), keeps the previous hy-server.exe, and puts it back if
;     the new one does not reach "running".
; Uninstall removes the service, the firewall rule, the program and its registry entries.
; The server data (keys, database, backups, mirror) is kept unless "Also delete the
; server data" is ticked (unticked by default) or a silent uninstall passes /DELETEDATA.
;
; Build: makensis /DVERSION=0.1.0 /DSRCEXE=<hy-server.exe> /DOUTFILE=<setup.exe> server-installer.nsi
; Silent: setup.exe /S /MIRROR=D:\HealthierYou-Mirror     (the mirror is kept on upgrades)
; Exit codes: 0 done, 2 bad options or not an administrator, 3 the signature check
; refused this installer (nothing changed), 4 the new version did not start and the
; previous one was put back, 5 the service did not start (fresh install).

Unicode true
ManifestDPIAware true
RequestExecutionLevel admin
SetCompressor /SOLID lzma

!ifndef VERSION
  !error "Pass /DVERSION=x.y.z"
!endif
!ifndef SRCEXE
  !error "Pass /DSRCEXE=path\to\hy-server.exe"
!endif
!ifndef OUTFILE
  !define OUTFILE "HealthierYou-Server_${VERSION}_x64-setup.exe"
!endif

!define PRODUCT "HealthierYou Server"
!define SERVICE "HealthierYouServer"
!define ACCOUNT "NT SERVICE\HealthierYouServer"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\HealthierYouServer"
!define SETTINGSKEY "Software\HealthierYou\Server"
!define FIREWALL_RULE "HealthierYou Server"
!define PORT 8443
!define MIRROR_FILE "control-log.mirror"

!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"
!include "nsDialogs.nsh"
!include "StrFunc.nsh"
${StrStr}
${UnStrStr}

Name "${PRODUCT}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\HealthierYou Server"
BrandingText "${PRODUCT} ${VERSION}"
ShowInstDetails show
ShowUninstDetails show

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCT}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${PRODUCT} setup"
VIAddVersionKey "CompanyName" "HealthierYou"
VIAddVersionKey "LegalCopyright" "HealthierYou"

Var DataDir
Var Mirror
Var Upgrade
Var MirrorField
Var DeleteData
Var DeleteBox

Page custom MirrorPage MirrorPageLeave
Page instfiles
UninstPage custom un.DataPage un.DataPageLeave
UninstPage instfiles

; ------------------------------------------------------------------- helpers

; Run a command hidden; leaves the exit code in $0 and the output in $1.
!macro Run CMD
  nsExec::ExecToStack '${CMD}'
  Pop $0
  Pop $1
  DetailPrint "exit $0"
!macroend

; Wait up to 90 s for the service to settle; leaves RUNNING, STOPPED or MISSING in $2.
!macro WaitService UN
  StrCpy $3 0
  ${Do}
    nsExec::ExecToStack '"$SYSDIR\sc.exe" query ${SERVICE}'
    Pop $0
    Pop $1
    ${If} $0 = 1060
      StrCpy $2 "MISSING"
      ${Break}
    ${EndIf}
    ${${UN}StrStr} $4 $1 "RUNNING"
    ${If} $4 != ""
      StrCpy $2 "RUNNING"
      ${Break}
    ${EndIf}
    ${${UN}StrStr} $4 $1 "STOPPED"
    ${If} $4 != ""
    ${AndIf} $3 >= 3
      StrCpy $2 "STOPPED"
      ${Break}
    ${EndIf}
    IntOp $3 $3 + 1
    ${If} $3 >= 90
      StrCpy $2 "STOPPED"
      ${Break}
    ${EndIf}
    Sleep 1000
  ${Loop}
!macroend

; Stop the service and wait (up to 60 s) until it has stopped.
!macro StopService UN
  DetailPrint "Stopping the service"
  nsExec::ExecToStack '"$SYSDIR\sc.exe" stop ${SERVICE}'
  Pop $0
  Pop $1
  StrCpy $3 0
  ${Do}
    nsExec::ExecToStack '"$SYSDIR\sc.exe" query ${SERVICE}'
    Pop $0
    Pop $1
    ${If} $0 = 1060
      ${Break}
    ${EndIf}
    ${${UN}StrStr} $4 $1 "STOPPED"
    ${If} $4 != ""
      ${Break}
    ${EndIf}
    IntOp $3 $3 + 1
    ${If} $3 >= 60
      ${Break}
    ${EndIf}
    Sleep 1000
  ${Loop}
  ; Let the process release hy-server.exe.
  Sleep 1500
!macroend

!macro Fail CODE TEXT
  DetailPrint "${TEXT}"
  IfSilent +2
    MessageBox MB_OK|MB_ICONSTOP "${TEXT}"
  SetErrorLevel ${CODE}
  Quit
!macroend

; --------------------------------------------------------------------- install

Function .onInit
  ${If} ${RunningX64}
    ${DisableX64FSRedirection}
    SetRegView 64
  ${Else}
    MessageBox MB_OK|MB_ICONSTOP "${PRODUCT} needs 64-bit Windows."
    SetErrorLevel 2
    Quit
  ${EndIf}
  SetShellVarContext all
  StrCpy $DataDir "$APPDATA\HealthierYou\Server"
  UserInfo::GetAccountType
  Pop $0
  ${If} $0 != "Admin"
    IfSilent +2
      MessageBox MB_OK|MB_ICONSTOP "Run the ${PRODUCT} setup as an administrator."
    SetErrorLevel 2
    Quit
  ${EndIf}
  ReadRegStr $0 HKLM "${UNINSTKEY}" "InstallLocation"
  ${If} $0 != ""
  ${AndIf} ${FileExists} "$0\hy-server.exe"
    StrCpy $INSTDIR $0
    StrCpy $Upgrade 1
  ${Else}
    StrCpy $Upgrade 0
  ${EndIf}
  ReadRegStr $Mirror HKLM "${SETTINGSKEY}" "MirrorFolder"
  ClearErrors
  ${GetOptions} $CMDLINE "/MIRROR=" $0
  ${IfNot} ${Errors}
    StrCpy $Mirror $0
  ${EndIf}
FunctionEnd

Function MirrorPage
  ${If} $Upgrade = 1
  ${AndIf} $Mirror != ""
    Abort ; an upgrade keeps the mirror
  ${EndIf}
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateLabel} 0 0 100% 60u "The server keeps an append-only copy of its control log (the mirror) on ANOTHER volume than C:: a second disk or a network share. It is how a restored old database is detected. Choose that folder:"
  Pop $0
  ${NSD_CreateText} 0 64u 100% 12u "$Mirror"
  Pop $MirrorField
  ${NSD_CreateLabel} 0 84u 100% 40u "Data folder (keys, database, backups): $DataDir. Only SYSTEM, Administrators and the HealthierYou service can open it. Copy the backups folder off this PC as well."
  Pop $0
  nsDialogs::Show
FunctionEnd

Function MirrorPageLeave
  ${NSD_GetText} $MirrorField $Mirror
  ${If} $Mirror == ""
    MessageBox MB_OK|MB_ICONEXCLAMATION "Choose a mirror folder on another volume."
    Abort
  ${EndIf}
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"
  ; Without a trailing backslash ("D:\" -> "D:").
  StrCpy $0 $Mirror 1 -1
  ${If} $0 == "\"
    StrCpy $Mirror $Mirror -1
  ${EndIf}
  ${If} $Mirror == ""
    !insertmacro Fail 2 "Give the mirror folder on another volume: /MIRROR=D:\HealthierYou-Mirror"
  ${EndIf}

  ${If} $Upgrade = 1
    ; The installed server checks this installer and its signature before anything
    ; changes (the same keys and rules as the companion's update files).
    DetailPrint "Checking the signature with the installed server"
    nsExec::ExecToStack '"$INSTDIR\hy-server.exe" verify-release --file "$EXEPATH"'
    Pop $0
    Pop $1
    DetailPrint "$1"
    ${If} $0 <> 0
      !insertmacro Fail 3 "The installed server refused this installer, so nothing was changed: $1"
    ${EndIf}
    !insertmacro StopService ""
    Delete "$INSTDIR\hy-server.previous.exe"
    CopyFiles /SILENT "$INSTDIR\hy-server.exe" "$INSTDIR\hy-server.previous.exe"
    ; A copy of the database as it was, in case the new version changed it and the
    ; previous one cannot open it (it stays in the protected data folder).
    CreateDirectory "$DataDir\pre-update"
    Delete "$DataDir\pre-update\server.db*"
    CopyFiles /SILENT "$DataDir\server.db" "$DataDir\pre-update"
    IfFileExists "$DataDir\server.db-wal" 0 +2
      CopyFiles /SILENT "$DataDir\server.db-wal" "$DataDir\pre-update"
  ${EndIf}

  ClearErrors
  File /oname=hy-server.exe "${SRCEXE}"
  ${If} ${Errors}
    !insertmacro Fail 2 "hy-server.exe could not be written (is the service still running?)."
  ${EndIf}
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; The data folder: inheritance removed; full control for SYSTEM and Administrators,
  ; modify for the service's virtual account (by SID where Windows has a fixed one).
  CreateDirectory "$DataDir"
  CreateDirectory "$Mirror"

  ${If} $Upgrade = 0
    DetailPrint "Registering the service"
    !insertmacro Run '"$SYSDIR\sc.exe" create ${SERVICE} binPath= "\"$INSTDIR\hy-server.exe\" service --mirror \"$Mirror\${MIRROR_FILE}\"" start= delayed-auto obj= "${ACCOUNT}" DisplayName= "${PRODUCT}"'
    ${If} $0 <> 0
    ${AndIf} $0 <> 1073
      !insertmacro Fail 2 "The service could not be registered (sc exit $0)."
    ${EndIf}
  ${Else}
    !insertmacro Run '"$SYSDIR\sc.exe" config ${SERVICE} binPath= "\"$INSTDIR\hy-server.exe\" service --mirror \"$Mirror\${MIRROR_FILE}\"" start= delayed-auto obj= "${ACCOUNT}"'
  ${EndIf}
  !insertmacro Run '"$SYSDIR\sc.exe" description ${SERVICE} "HealthierYou office server: pairing and sync for the desktop companions (TCP ${PORT}, admin page on this PC only)."'
  !insertmacro Run '"$SYSDIR\sc.exe" failure ${SERVICE} reset= 86400 actions= restart/60000/restart/60000/restart/300000'
  !insertmacro Run '"$SYSDIR\sc.exe" failureflag ${SERVICE} 1'
  ; The service's own SID (NT SERVICE\HealthierYouServer) in its token, so the folder
  ; rules below apply to it.
  !insertmacro Run '"$SYSDIR\sc.exe" sidtype ${SERVICE} unrestricted'

  DetailPrint "Protecting the data folder"
  !insertmacro Run '"$SYSDIR\icacls.exe" "$DataDir" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F "${ACCOUNT}:(OI)(CI)M"'
  ${If} $0 <> 0
    !insertmacro Fail 2 "The data folder could not be protected (icacls exit $0): $1"
  ${EndIf}
  ; Files made before (keys, database) take the folder's rules again.
  !insertmacro Run '"$SYSDIR\icacls.exe" "$DataDir\*" /reset /T /C /Q'
  DetailPrint "Protecting the mirror folder"
  !insertmacro Run '"$SYSDIR\icacls.exe" "$Mirror" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F "${ACCOUNT}:(OI)(CI)M"'
  ${If} $0 <> 0
    ; A share can refuse local ACL changes; its owner sets its rights instead.
    DetailPrint "The mirror folder's rights were not changed (a share?). Give the HealthierYou service modify rights there."
  ${EndIf}

  DetailPrint "Firewall: TCP ${PORT} in, private and domain networks only"
  !insertmacro Run '"$SYSDIR\netsh.exe" advfirewall firewall delete rule name="${FIREWALL_RULE}"'
  !insertmacro Run '"$SYSDIR\netsh.exe" advfirewall firewall add rule name="${FIREWALL_RULE}" dir=in action=allow protocol=TCP localport=${PORT} profile=private,domain program="$INSTDIR\hy-server.exe" description="HealthierYou companions connect to the office server here."'
  ${If} $0 <> 0
    !insertmacro Fail 2 "The firewall rule could not be added (netsh exit $0)."
  ${EndIf}

  WriteRegStr HKLM "${SETTINGSKEY}" "MirrorFolder" "$Mirror"
  WriteRegStr HKLM "${UNINSTKEY}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKLM "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTKEY}" "Publisher" "HealthierYou"
  WriteRegStr HKLM "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\hy-server.exe"
  WriteRegStr HKLM "${UNINSTKEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTKEY}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKLM "${UNINSTKEY}" "EstimatedSize" $0

  DetailPrint "Starting the service"
  Delete "$DataDir\service-error.txt"
  !insertmacro Run '"$SYSDIR\sc.exe" start ${SERVICE}'
  !insertmacro WaitService ""
  ${If} $2 == "RUNNING"
    ; Still running a few seconds later (not a crash right after the ports opened).
    Sleep 5000
    !insertmacro WaitService ""
  ${EndIf}
  ${If} $2 == "RUNNING"
    DetailPrint "The service is running."
    Delete "$INSTDIR\hy-server.previous.exe"
    RMDir /r "$DataDir\pre-update"
  ${ElseIf} $Upgrade = 1
    DetailPrint "The new version did not start; putting the previous one back."
    !insertmacro StopService ""
    Delete "$INSTDIR\hy-server.exe"
    CopyFiles /SILENT "$INSTDIR\hy-server.previous.exe" "$INSTDIR\hy-server.exe"
    !insertmacro Run '"$SYSDIR\sc.exe" start ${SERVICE}'
    !insertmacro WaitService ""
    ${If} $2 != "RUNNING"
      ; The previous version may not open a database the new one changed.
      DetailPrint "Restoring the database copy made before the update."
      !insertmacro StopService ""
      Rename "$DataDir\server.db" "$DataDir\server.db.after-failed-update"
      Delete "$DataDir\server.db-wal"
      Delete "$DataDir\server.db-shm"
      CopyFiles /SILENT "$DataDir\pre-update\server.db*" "$DataDir"
      !insertmacro Run '"$SYSDIR\sc.exe" start ${SERVICE}'
      !insertmacro WaitService ""
    ${EndIf}
    ReadRegStr $5 HKLM "${SETTINGSKEY}" "Version"
    ${If} $5 != ""
      WriteRegStr HKLM "${UNINSTKEY}" "DisplayVersion" "$5"
    ${EndIf}
    ${If} $2 == "RUNNING"
      !insertmacro Fail 4 "Version ${VERSION} did not start, so the previous version ($5) was put back and is running. See service-error.txt and logs in the data folder."
    ${Else}
      !insertmacro Fail 4 "Version ${VERSION} did not start and the previous version did not start either. See service-error.txt and logs in the data folder."
    ${EndIf}
  ${Else}
    !insertmacro Fail 5 "The service did not start. See service-error.txt and logs in $DataDir."
  ${EndIf}
  WriteRegStr HKLM "${SETTINGSKEY}" "Version" "${VERSION}"
SectionEnd

; ------------------------------------------------------------------- uninstall

Function un.onInit
  ${If} ${RunningX64}
    ${DisableX64FSRedirection}
    SetRegView 64
  ${EndIf}
  SetShellVarContext all
  StrCpy $DataDir "$APPDATA\HealthierYou\Server"
  ReadRegStr $Mirror HKLM "${SETTINGSKEY}" "MirrorFolder"
  StrCpy $DeleteData 0
  ClearErrors
  ${GetOptions} $CMDLINE "/DELETEDATA" $0
  ${IfNot} ${Errors}
    StrCpy $DeleteData 1
  ${EndIf}
FunctionEnd

Function un.DataPage
  nsDialogs::Create 1018
  Pop $0
  ${NSD_CreateLabel} 0 0 100% 40u "The service, the firewall rule and the program will be removed. The server data (keys, database, backups and the mirror) is kept unless you tick the box: without the keys every PC must pair again and the backups cannot be opened."
  Pop $0
  ${NSD_CreateCheckbox} 0 48u 100% 24u "Also delete the server data: $DataDir and the mirror in $Mirror"
  Pop $DeleteBox
  nsDialogs::Show
FunctionEnd

Function un.DataPageLeave
  ${NSD_GetState} $DeleteBox $0
  ${If} $0 == ${BST_CHECKED}
    StrCpy $DeleteData 1
  ${EndIf}
FunctionEnd

Section "Uninstall"
  !insertmacro StopService "Un"
  !insertmacro Run '"$SYSDIR\sc.exe" delete ${SERVICE}'
  !insertmacro Run '"$SYSDIR\netsh.exe" advfirewall firewall delete rule name="${FIREWALL_RULE}"'
  Delete "$INSTDIR\hy-server.exe"
  Delete "$INSTDIR\hy-server.previous.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "${UNINSTKEY}"
  DeleteRegKey HKLM "${SETTINGSKEY}"
  DeleteRegKey /ifempty HKLM "Software\HealthierYou"
  ${If} $DeleteData = 1
    RMDir /r "$DataDir"
    ${If} $Mirror != ""
      Delete "$Mirror\${MIRROR_FILE}"
      RMDir "$Mirror"
    ${EndIf}
    RMDir "$APPDATA\HealthierYou"
  ${EndIf}
SectionEnd
