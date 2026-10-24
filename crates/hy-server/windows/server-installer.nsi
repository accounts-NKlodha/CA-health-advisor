; HealthierYou Server installer (slice V1-6b). Per machine, run by an administrator on
; the office server PC. It:
;   - installs hy-server.exe to Program Files and registers the Windows service
;     HealthierYouServer under its virtual account NT SERVICE\HealthierYouServer
;     (V1-5 contract 6.2), started automatically (delayed) and restarted after a failure;
;   - makes %ProgramData%\HealthierYou\Server open only to SYSTEM, Administrators and
;     the service: Administrators own the whole tree, the folder's list is set and
;     protected, every file and folder under it loses its own entries and takes the
;     folder's, and hy-server check-acl reads it all back (fails closed, exit 8). The
;     service makes its keys there at its first start, protected with machine-scope DPAPI;
;   - keeps the data bound to its own control-log mirror (contract 7.3): the mirror's
;     folder is remembered in the data folder (mirror-location.txt, kept on uninstall)
;     and is the default for a reinstall; before the service is pointed at a mirror,
;     hy-server check-mirror confirms it is the data's own (or that there is no data yet).
;     It runs the startup comparison without its effects on the stopped server, so a
;     stale copy (behind what the server recorded as mirrored) or a forked one is refused
;     too. Anything else is refused and nothing changes (exit 6). Moving the mirror is:
;     with the service stopped (uninstall keeping the data), copy control-log.mirror to
;     the new folder, then run setup with /MIRROR= that folder;
;   - opens TCP 8443 inbound for hy-server.exe on the private and domain profiles only.
;     The admin page (8444) listens on loopback only and gets no rule;
;   - on an upgrade, has the INSTALLED server check this installer's .sig and the version
;     policy first (hy-server verify-release: a newer version, or exactly the version it
;     was last updated from), stops the service and confirms it stopped, makes and
;     verifies copies of hy-server.exe and the database (with its WAL) before anything is
;     replaced, and puts them back if the new version does not reach "running".
; Uninstall removes the service, the firewall rule, the program and its registry entries.
; The server data (keys, database, backups, mirror) is kept unless "Also delete the
; server data" is ticked (unticked by default) or a silent uninstall passes /DELETEDATA.
;
; Build: makensis /DVERSION=0.1.0 /DSRCEXE=<hy-server.exe> /DOUTFILE=<setup.exe> server-installer.nsi
; Silent: setup.exe /S [/MIRROR=D:\HealthierYou-Mirror]   (default: the remembered mirror)
; Exit codes:
;   0 done
;   2 bad options, not an administrator, or the program could not be written (fresh install)
;   3 the installed server refused this installer: signature or version (nothing changed)
;   4 the new version did not start; the previous program (and, if needed, its database
;     copy) was put back and is running
;   5 the service did not start (fresh install)
;   6 the mirror is not the server data's own (nothing changed)
;   7 the service could not be confirmed stopped, or the copies of the program and the
;     database could not be made and verified (nothing changed; the previous version runs)
;   8 the data folder's rights could not be set or verified (nothing replaced)
;   9 putting the previous version back did not complete: the message says exactly what
;     is where; nothing was deleted

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
Var Current
Var Previous
Var Server
Var Failed

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

; Stop the service and wait (up to 60 s) until it has stopped; leaves STOPPED, MISSING
; or RUNNING (not confirmed stopped) in $2.
!macro StopService UN
  DetailPrint "Stopping the service"
  nsExec::ExecToStack '"$SYSDIR\sc.exe" stop ${SERVICE}'
  Pop $0
  Pop $1
  StrCpy $3 0
  StrCpy $2 "RUNNING"
  ${Do}
    nsExec::ExecToStack '"$SYSDIR\sc.exe" query ${SERVICE}'
    Pop $0
    Pop $1
    ${If} $0 = 1060
      StrCpy $2 "MISSING"
      ${Break}
    ${EndIf}
    ${${UN}StrStr} $4 $1 "STOPPED"
    ${If} $4 != ""
      StrCpy $2 "STOPPED"
      ${Break}
    ${EndIf}
    IntOp $3 $3 + 1
    ${If} $3 >= 60
      ${Break}
    ${EndIf}
    Sleep 1000
  ${Loop}
  ; Let the process release hy-server.exe and the database.
  Sleep 1500
!macroend

; Start the service and wait until it runs and is still running 5 s later; leaves
; RUNNING or STOPPED in $2.
!macro StartService
  Delete "$DataDir\service-error.txt"
  !insertmacro Run '"$SYSDIR\sc.exe" start ${SERVICE}'
  !insertmacro WaitService ""
  ${If} $2 == "RUNNING"
    Sleep 5000
    !insertmacro WaitService ""
  ${EndIf}
!macroend

; Compare two files byte for byte; leaves 0 in $0 only when both exist and are equal.
!macro Same A B
  ${If} ${FileExists} "${A}"
  ${AndIf} ${FileExists} "${B}"
    nsExec::ExecToStack '"$SYSDIR\fc.exe" /b "${A}" "${B}"'
    Pop $0
    Pop $1
  ${Else}
    StrCpy $0 2
  ${EndIf}
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
  ReadRegStr $Current HKLM "${SETTINGSKEY}" "Version"
  ${If} $Current == ""
    ReadRegStr $Current HKLM "${UNINSTKEY}" "DisplayVersion"
  ${EndIf}
  ReadRegStr $Previous HKLM "${SETTINGSKEY}" "PreviousVersion"
  ; The mirror: the one given, else the installed one, else the one the kept data
  ; remembers (a reinstall after an uninstall that kept the data).
  ReadRegStr $Mirror HKLM "${SETTINGSKEY}" "MirrorFolder"
  ${If} $Mirror == ""
  ${AndIf} ${FileExists} "$DataDir\mirror-location.txt"
    ClearErrors
    FileOpen $0 "$DataDir\mirror-location.txt" r
    ${IfNot} ${Errors}
      FileReadUTF16LE $0 $Mirror
      FileClose $0
      ; Without the line end.
      ${Do}
        StrCpy $1 $Mirror 1 -1
        ${If} $1 == "$\r"
        ${OrIf} $1 == "$\n"
          StrCpy $Mirror $Mirror -1
        ${Else}
          ${Break}
        ${EndIf}
      ${Loop}
    ${EndIf}
  ${EndIf}
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
  ${NSD_CreateLabel} 0 0 100% 60u "The server keeps an append-only copy of its control log (the mirror) on ANOTHER volume than C:: a second disk or a network share. It is how a restored old database is detected. Choose that folder (kept server data must use its own, current mirror: to move it, copy control-log.mirror while the service is stopped):"
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

; Put the previous version back after the new one did not start (an upgrade). Each step
; is checked; a step that fails stops here with exit 9 and says exactly what is where.
Function PutPreviousBack
  DetailPrint "Version ${VERSION} did not start; putting the previous version ($Current) back."
  !insertmacro StopService ""
  ${If} $2 == "RUNNING"
    !insertmacro Fail 9 "Version ${VERSION} did not start and its service could not be stopped, so nothing was put back. The previous program is kept as $INSTDIR\hy-server.previous.exe and the database copy made before the update in $DataDir\pre-update. Stop the service, then run the setup of version $Current."
  ${EndIf}
  ClearErrors
  Delete "$INSTDIR\hy-server.exe"
  CopyFiles /SILENT "$INSTDIR\hy-server.previous.exe" "$INSTDIR\hy-server.exe"
  !insertmacro Same "$INSTDIR\hy-server.previous.exe" "$INSTDIR\hy-server.exe"
  ${If} $0 <> 0
    !insertmacro Run '"$SYSDIR\sc.exe" config ${SERVICE} start= disabled'
    !insertmacro Fail 9 "Version ${VERSION} did not start and the previous program could not be put back. The service was disabled. The previous program is kept as $INSTDIR\hy-server.previous.exe and the database is unchanged in $DataDir. Run the setup of version $Current."
  ${EndIf}
  WriteRegStr HKLM "${UNINSTKEY}" "DisplayVersion" "$Current"
  !insertmacro StartService
  ${If} $2 == "RUNNING"
    Delete "$INSTDIR\hy-server.previous.exe"
    RMDir /r "$DataDir\pre-update"
    !insertmacro Fail 4 "Version ${VERSION} did not start, so version $Current was put back and is running with the current data. See service-error.txt and logs in the data folder."
  ${EndIf}

  ; The previous version may not open a database the new one changed: put back the
  ; copy made before the update, and keep the failed version's database with its WAL.
  DetailPrint "The previous version did not start with the current database; putting back the copy made before the update."
  !insertmacro StopService ""
  ${If} $2 == "RUNNING"
    !insertmacro Fail 9 "Version $Current is back but did not start, and its service could not be stopped, so the database was not touched. The copy made before the update is in $DataDir\pre-update."
  ${EndIf}
  ${IfNot} ${FileExists} "$DataDir\pre-update\server.db"
    !insertmacro Fail 9 "Version $Current is back but did not start, and there is no database copy from before the update. The database is unchanged in $DataDir."
  ${EndIf}
  ; While the database is being changed the service must not be restarted (its restart
  ; after a failure, or anyone): with no database it would make an empty one.
  !insertmacro Run '"$SYSDIR\sc.exe" config ${SERVICE} start= disabled'
  StrCpy $Failed "$DataDir\failed-update-${VERSION}"
  StrCpy $5 1
  ${DoWhile} ${FileExists} "$Failed\*.*"
    IntOp $5 $5 + 1
    StrCpy $Failed "$DataDir\failed-update-${VERSION}-$5"
  ${Loop}
  ClearErrors
  CreateDirectory "$Failed"
  ${IfNot} ${FileExists} "$Failed\*.*"
    !insertmacro Run '"$SYSDIR\sc.exe" config ${SERVICE} start= delayed-auto'
    !insertmacro Fail 9 "Version $Current is back but did not start; the folder for the failed version's database could not be made, so the database was not touched. The copy made before the update is in $DataDir\pre-update."
  ${EndIf}
  ClearErrors
  Rename "$DataDir\server.db" "$Failed\server.db"
  ${If} ${Errors}
  ${OrIf} ${FileExists} "$DataDir\server.db"
    !insertmacro Run '"$SYSDIR\sc.exe" config ${SERVICE} start= delayed-auto'
    !insertmacro Fail 9 "Version $Current is back but did not start; its database could not be set aside, so it is unchanged in $DataDir. The copy made before the update is in $DataDir\pre-update."
  ${EndIf}
  ${If} ${FileExists} "$DataDir\server.db-wal"
    ClearErrors
    Rename "$DataDir\server.db-wal" "$Failed\server.db-wal"
    ${If} ${Errors}
    ${OrIf} ${FileExists} "$DataDir\server.db-wal"
      ; Never leave a database apart from its WAL: move it back.
      ClearErrors
      Rename "$Failed\server.db" "$DataDir\server.db"
      ${If} ${Errors}
        !insertmacro Fail 9 "Version $Current is back but did not start. Setting its database aside did not complete: server.db is in $Failed and server.db-wal is still in $DataDir; move server.db back next to it. The service was disabled. The copy made before the update is in $DataDir\pre-update."
      ${EndIf}
      !insertmacro Run '"$SYSDIR\sc.exe" config ${SERVICE} start= delayed-auto'
      !insertmacro Fail 9 "Version $Current is back but did not start; its database could not be set aside, so it is unchanged in $DataDir. The copy made before the update is in $DataDir\pre-update."
    ${EndIf}
  ${EndIf}
  ${If} ${FileExists} "$DataDir\server.db-shm"
    Rename "$DataDir\server.db-shm" "$Failed\server.db-shm"
    Delete "$DataDir\server.db-shm"
  ${EndIf}
  ClearErrors
  CopyFiles /SILENT "$DataDir\pre-update\server.db" "$DataDir"
  !insertmacro Same "$DataDir\pre-update\server.db" "$DataDir\server.db"
  StrCpy $6 $0
  ${If} ${FileExists} "$DataDir\pre-update\server.db-wal"
    CopyFiles /SILENT "$DataDir\pre-update\server.db-wal" "$DataDir"
    !insertmacro Same "$DataDir\pre-update\server.db-wal" "$DataDir\server.db-wal"
    ${If} $0 <> 0
      StrCpy $6 $0
    ${EndIf}
  ${EndIf}
  ${If} $6 <> 0
    !insertmacro Fail 9 "Version $Current is back but did not start, and the database copy from before the update could not be put back. The service was disabled so it does not make an empty database. Copy the files in $DataDir\pre-update into $DataDir, then start the service. The failed version's database is in $Failed."
  ${EndIf}
  !insertmacro Run '"$SYSDIR\sc.exe" config ${SERVICE} start= delayed-auto'
  !insertmacro StartService
  ${If} $2 == "RUNNING"
    Delete "$INSTDIR\hy-server.previous.exe"
    RMDir /r "$DataDir\pre-update"
    !insertmacro Fail 4 "Version ${VERSION} did not start, so version $Current and the database from before the update were put back and are running. Version ${VERSION}'s database is kept in $Failed."
  ${EndIf}
  !insertmacro Fail 9 "Version ${VERSION} did not start. Version $Current and the database from before the update were put back but did not start either. Version ${VERSION}'s database is kept in $Failed. See service-error.txt and logs in the data folder."
FunctionEnd

; Set the data folder's owner and rights on the whole tree; leaves 0 in $0 on success.
Function ProtectData
  DetailPrint "Protecting the data folder"
  ; Administrators own everything (an owner can always change the rights).
  !insertmacro Run '"$SYSDIR\icacls.exe" "$DataDir" /setowner *S-1-5-32-544 /T /C /Q'
  ${If} $0 <> 0
    Return
  ${EndIf}
  ; The folder's own list: first drop every entry it has, then stop inheriting and
  ; grant only SYSTEM, Administrators and the service.
  !insertmacro Run '"$SYSDIR\icacls.exe" "$DataDir" /reset /Q'
  ${If} $0 <> 0
    Return
  ${EndIf}
  !insertmacro Run '"$SYSDIR\icacls.exe" "$DataDir" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F "${ACCOUNT}:(OI)(CI)M"'
  ${If} $0 <> 0
    Return
  ${EndIf}
  ; Everything under it loses its own entries and takes the folder's.
  ${If} ${FileExists} "$DataDir\*"
    !insertmacro Run '"$SYSDIR\icacls.exe" "$DataDir\*" /reset /T /C /Q'
    ${If} $0 <> 0
      Return
    ${EndIf}
  ${EndIf}
  ; Read it all back.
  DetailPrint "Checking the data folder's rights"
  !insertmacro Run '"$Server" check-acl --data-dir "$DataDir"'
  DetailPrint "$1"
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

  ; The new program, unpacked to a temporary folder first (it is copied into place only
  ; after every check). The server that makes the checks: on an upgrade the INSTALLED
  ; one (it decides what may replace it); on a fresh install this installer's own.
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=hy-server-new.exe "${SRCEXE}"
  SetOutPath "$INSTDIR"
  ${If} $Upgrade = 1
    StrCpy $Server "$INSTDIR\hy-server.exe"
  ${Else}
    StrCpy $Server "$PLUGINSDIR\hy-server-new.exe"
  ${EndIf}

  ${If} $Upgrade = 1
    DetailPrint "Checking the signature and the version with the installed server"
    ${If} $Previous != ""
      nsExec::ExecToStack '"$Server" verify-release --file "$EXEPATH" --current "$Current" --previous "$Previous"'
    ${Else}
      nsExec::ExecToStack '"$Server" verify-release --file "$EXEPATH" --current "$Current"'
    ${EndIf}
    Pop $0
    Pop $1
    DetailPrint "$1"
    ${If} $0 <> 0
      !insertmacro Fail 3 "The installed server refused this installer, so nothing was changed: $1"
    ${EndIf}
    ; Copies left by an earlier update that did not complete may be the only good
    ; database: never overwrite them.
    ${If} ${FileExists} "$DataDir\pre-update\*.*"
      !insertmacro Fail 7 "An earlier update did not complete and its copies of the database are in $DataDir\pre-update, so nothing was changed. Follow the message of that update (docs/V1-6_Install_Update.md, server recovery); once the server runs with the right database, move the pre-update folder out of the data folder and run this setup again."
    ${EndIf}
    !insertmacro StopService ""
    ${If} $2 != "STOPPED"
      !insertmacro Fail 7 "The service could not be confirmed stopped, so nothing was changed."
    ${EndIf}
  ${EndIf}

  ; The mirror must be the kept data's own (or there is no data yet).
  DetailPrint "Checking the mirror $Mirror"
  !insertmacro Run '"$Server" check-mirror --data-dir "$DataDir" --mirror "$Mirror\${MIRROR_FILE}"'
  DetailPrint "$1"
  ${If} $0 <> 0
    ${If} $Upgrade = 1
      !insertmacro StartService
    ${EndIf}
    !insertmacro Fail 6 "The server data in $DataDir belongs with its own control-log mirror, and $Mirror\${MIRROR_FILE} is not it, so nothing was changed. Use the mirror folder the data was set up with. To move it: uninstall keeping the data (or stop the service), copy control-log.mirror to the new folder, then run setup with /MIRROR= that folder. $1"
  ${EndIf}

  ${If} $Upgrade = 1
    ; Copies of the program and the database as they are, each verified, before anything
    ; is replaced (they stay in the program folder and the protected data folder).
    DetailPrint "Copying the current program and database"
    StrCpy $6 0
    Delete "$INSTDIR\hy-server.previous.exe"
    CopyFiles /SILENT "$INSTDIR\hy-server.exe" "$INSTDIR\hy-server.previous.exe"
    !insertmacro Same "$INSTDIR\hy-server.exe" "$INSTDIR\hy-server.previous.exe"
    ${If} $0 <> 0
      StrCpy $6 1
    ${EndIf}
    RMDir /r "$DataDir\pre-update"
    ${If} ${FileExists} "$DataDir\pre-update\*.*"
      StrCpy $6 1
    ${EndIf}
    ${If} ${FileExists} "$DataDir\server.db"
    ${AndIf} $6 = 0
      CreateDirectory "$DataDir\pre-update"
      CopyFiles /SILENT "$DataDir\server.db" "$DataDir\pre-update"
      !insertmacro Same "$DataDir\server.db" "$DataDir\pre-update\server.db"
      ${If} $0 <> 0
        StrCpy $6 1
      ${EndIf}
      ${If} ${FileExists} "$DataDir\server.db-wal"
        CopyFiles /SILENT "$DataDir\server.db-wal" "$DataDir\pre-update"
        !insertmacro Same "$DataDir\server.db-wal" "$DataDir\pre-update\server.db-wal"
        ${If} $0 <> 0
          StrCpy $6 1
        ${EndIf}
      ${EndIf}
    ${EndIf}
    ${If} $6 <> 0
      !insertmacro StartService
      !insertmacro Fail 7 "The copies of the current program and database could not be made and checked, so nothing was changed and version $Current was started again."
    ${EndIf}
  ${EndIf}

  ; A fresh install registers the service first: its account (and SID) exists only then.
  ${If} $Upgrade = 0
    DetailPrint "Registering the service"
    !insertmacro Run '"$SYSDIR\sc.exe" create ${SERVICE} binPath= "\"$INSTDIR\hy-server.exe\" service --mirror \"$Mirror\${MIRROR_FILE}\"" start= delayed-auto obj= "${ACCOUNT}" DisplayName= "${PRODUCT}"'
    ${If} $0 <> 0
    ${AndIf} $0 <> 1073
      !insertmacro Fail 2 "The service could not be registered (sc exit $0)."
    ${EndIf}
    ; The service's own SID (NT SERVICE\HealthierYouServer) in its token, so the folder
    ; rules apply to it.
    !insertmacro Run '"$SYSDIR\sc.exe" sidtype ${SERVICE} unrestricted'
  ${EndIf}

  ; The data folder's rights, before the new program is in place.
  CreateDirectory "$DataDir"
  Call ProtectData
  ${If} $0 <> 0
    ${If} $Upgrade = 1
      !insertmacro StartService
      !insertmacro Fail 8 "The data folder's rights could not be set or checked, so the program was not replaced: $1"
    ${EndIf}
    StrCpy $5 $1
    !insertmacro Run '"$SYSDIR\sc.exe" delete ${SERVICE}'
    !insertmacro Fail 8 "The data folder's rights could not be set or checked, so nothing was installed: $5"
  ${EndIf}

  Delete "$INSTDIR\hy-server.exe"
  CopyFiles /SILENT "$PLUGINSDIR\hy-server-new.exe" "$INSTDIR\hy-server.exe"
  !insertmacro Same "$PLUGINSDIR\hy-server-new.exe" "$INSTDIR\hy-server.exe"
  ${If} $0 <> 0
    ${If} $Upgrade = 1
      Call PutPreviousBack
    ${EndIf}
    !insertmacro Fail 2 "hy-server.exe could not be written."
  ${EndIf}
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateDirectory "$Mirror"

  ${If} $Upgrade = 1
    !insertmacro Run '"$SYSDIR\sc.exe" config ${SERVICE} binPath= "\"$INSTDIR\hy-server.exe\" service --mirror \"$Mirror\${MIRROR_FILE}\"" start= delayed-auto obj= "${ACCOUNT}"'
  ${EndIf}
  !insertmacro Run '"$SYSDIR\sc.exe" description ${SERVICE} "HealthierYou office server: pairing and sync for the desktop companions (TCP ${PORT}, admin page on this PC only)."'
  !insertmacro Run '"$SYSDIR\sc.exe" failure ${SERVICE} reset= 86400 actions= restart/60000/restart/60000/restart/300000'
  !insertmacro Run '"$SYSDIR\sc.exe" failureflag ${SERVICE} 1'
  ; The service's own SID (NT SERVICE\HealthierYouServer) in its token, so the folder
  ; rules apply to it.
  !insertmacro Run '"$SYSDIR\sc.exe" sidtype ${SERVICE} unrestricted'

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
  !insertmacro StartService
  ${If} $2 != "RUNNING"
    ${If} $Upgrade = 1
      Call PutPreviousBack
    ${EndIf}
    !insertmacro Fail 5 "The service did not start. See service-error.txt and logs in $DataDir."
  ${EndIf}
  DetailPrint "The service is running."
  Delete "$INSTDIR\hy-server.previous.exe"
  RMDir /r "$DataDir\pre-update"
  ; Remember what runs, what it was updated from, and the data's mirror (in the data
  ; folder too, so a reinstall after an uninstall that kept the data finds it).
  ${If} $Upgrade = 1
  ${AndIf} $Current != "${VERSION}"
    WriteRegStr HKLM "${SETTINGSKEY}" "PreviousVersion" "$Current"
  ${EndIf}
  WriteRegStr HKLM "${SETTINGSKEY}" "Version" "${VERSION}"
  WriteRegStr HKLM "${SETTINGSKEY}" "MirrorFolder" "$Mirror"
  ClearErrors
  FileOpen $0 "$DataDir\mirror-location.txt" w
  ${IfNot} ${Errors}
    FileWriteUTF16LE /BOM $0 "$Mirror$\r$\n"
    FileClose $0
  ${EndIf}
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
