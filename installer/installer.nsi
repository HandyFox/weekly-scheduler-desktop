; Weekly Assignment Scheduler — Windows installer (NSIS 3)
; Build:  makensis -DVERSION=1.1.0 -DEXE=..\target\release\WeeklyScheduler.exe -DOUTDIR=..\dist installer.nsi
; Installs for the current user only, so no administrator password is needed.

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user

!ifndef VERSION
  !define VERSION "1.1.0"
!endif
!ifndef EXE
  !define EXE "..\target\release\WeeklyScheduler.exe"
!endif
!ifndef OUTDIR
  !define OUTDIR "..\dist"
!endif

!define APPNAME "Weekly Assignment Scheduler"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\WeeklyAssignmentScheduler"
!define APPKEY "Software\TeacherResource\WeeklyAssignmentScheduler"

Name "${APPNAME}"
OutFile "${OUTDIR}\WeeklyScheduler-Setup-${VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\${APPNAME}"
InstallDirRegKey HKCU "${APPKEY}" "InstallDir"
BrandingText "${APPNAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "FileDescription" "${APPNAME} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Teacher Resource"
VIAddVersionKey "LegalCopyright" "Teacher Resource"

!include "MUI2.nsh"
!define MUI_ICON "..\assets\icon.ico"
!define MUI_UNICON "..\assets\icon.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "This puts ${APPNAME} on this computer.$\r$\n$\r$\nIt runs in your web browser but works without the internet. Your schedule is kept in Documents\${APPNAME}.$\r$\n$\r$\nIf you already use the scheduler, you can bring in your scheduler.db from its Settings page after it opens."
!define MUI_FINISHPAGE_RUN "$INSTDIR\WeeklyScheduler.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open the scheduler now"
!define MUI_FINISHPAGE_SHOWREADME "$INSTDIR\Read me.txt"
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Show the Read me"
!define MUI_FINISHPAGE_SHOWREADME_NOTCHECKED

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!macro CloseRunning
  ; a running copy would keep the program file locked
  nsExec::Exec 'taskkill /IM WeeklyScheduler.exe /F'
  Pop $0
  Sleep 500
!macroend

Section "Weekly Assignment Scheduler" SecMain
  SectionIn RO
  !insertmacro CloseRunning
  SetOutPath "$INSTDIR"
  File "/oname=WeeklyScheduler.exe" "${EXE}"
  File "..\assets\icon.ico"
  File "/oname=Read me.txt" "readme.txt"
  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\WeeklyScheduler.exe" "" "$INSTDIR\icon.ico"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "${APPKEY}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "Teacher Resource"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1
SectionEnd

Section "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\WeeklyScheduler.exe" "" "$INSTDIR\icon.ico"
SectionEnd

Section /o "Start when I sign in to Windows (no browser window)" SecStartup
  CreateShortcut "$SMSTARTUP\${APPNAME}.lnk" "$INSTDIR\WeeklyScheduler.exe" "--no-browser" "$INSTDIR\icon.ico"
SectionEnd

LangString DESC_Main ${LANG_ENGLISH} "The scheduler and a Start menu shortcut."
LangString DESC_Desktop ${LANG_ENGLISH} "A shortcut on the desktop."
LangString DESC_Startup ${LANG_ENGLISH} "Start quietly when you sign in, so it is ready by the clock."
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} $(DESC_Main)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} $(DESC_Desktop)
  !insertmacro MUI_DESCRIPTION_TEXT ${SecStartup} $(DESC_Startup)
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Function .onInit
  ; updating: keep "start when I sign in" the way it was
  IfFileExists "$SMSTARTUP\${APPNAME}.lnk" 0 +2
    SectionSetFlags ${SecStartup} ${SF_SELECTED}
FunctionEnd

Section "Uninstall"
  !insertmacro CloseRunning
  Delete "$INSTDIR\WeeklyScheduler.exe"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\Read me.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  Delete "$DESKTOP\${APPNAME}.lnk"
  Delete "$SMSTARTUP\${APPNAME}.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "${APPKEY}"
  IfSilent +2
    MessageBox MB_OK|MB_ICONINFORMATION "${APPNAME} was removed.$\r$\n$\r$\nYour schedule was NOT deleted. It is still in Documents\${APPNAME}."
SectionEnd
