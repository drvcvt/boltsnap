Unicode true
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

!ifndef PRODUCT_VERSION
  !error "PRODUCT_VERSION is required"
!endif
!ifndef OUTPUT_FILE
  !error "OUTPUT_FILE is required"
!endif
!ifndef BOLTSNAP_SOURCE_DIR
  !error "BOLTSNAP_SOURCE_DIR is required"
!endif
!ifndef EDDY_SOURCE_DIR
  !error "EDDY_SOURCE_DIR is required"
!endif
!ifndef LICENSE_FILE
  !error "LICENSE_FILE is required"
!endif

!define PRODUCT_NAME "Boltsnap"
!define PRODUCT_PUBLISHER "Boltsnap contributors"
!define PRODUCT_WEB_SITE "https://github.com/drvcvt/boltsnap"
!define PRODUCT_UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Boltsnap"

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "${OUTPUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\Boltsnap"
InstallDirRegKey HKCU "Software\Boltsnap" "InstallLocation"
BrandingText "Boltsnap"
ShowInstDetails show
ShowUnInstDetails show

VIProductVersion "${PRODUCT_VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey /LANG=1033 "CompanyName" "${PRODUCT_PUBLISHER}"
VIAddVersionKey /LANG=1033 "FileDescription" "Boltsnap Windows installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${PRODUCT_VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${PRODUCT_VERSION}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "MIT License"

!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC
!define MUI_FINISHPAGE_NOAUTOCLOSE
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${LICENSE_FILE}"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "English"

Section "!Boltsnap (required)" SEC_BOLTSNAP
  SectionIn RO
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  SetOverwrite on
  File "${BOLTSNAP_SOURCE_DIR}\boltsnap.exe"
  File "${BOLTSNAP_SOURCE_DIR}\boltsnap-background.exe"

  RMDir /r "$INSTDIR\Eddy"
  Delete "$SMPROGRAMS\Boltsnap\Eddy.lnk"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\App Paths\eddy.exe"

  WriteRegStr HKCU "Software\Boltsnap" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKCU "Software\Boltsnap" "StartMenuShortcut" 1
  WriteRegDWORD HKCU "Control Panel\Keyboard" "PrintScreenKeyForSnippingEnabled" 0
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\App Paths\boltsnap.exe" "" "$INSTDIR\boltsnap.exe"

  CreateDirectory "$SMPROGRAMS\Boltsnap"
  CreateShortcut "$SMPROGRAMS\Boltsnap\Boltsnap.lnk" "$INSTDIR\boltsnap-background.exe" "" "$INSTDIR\boltsnap-background.exe"

  WriteUninstaller "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "${PRODUCT_UNINSTALL_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKCU "${PRODUCT_UNINSTALL_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKCU "${PRODUCT_UNINSTALL_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKCU "${PRODUCT_UNINSTALL_KEY}" "URLInfoAbout" "${PRODUCT_WEB_SITE}"
  WriteRegStr HKCU "${PRODUCT_UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${PRODUCT_UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\boltsnap.exe"
  WriteRegStr HKCU "${PRODUCT_UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "${PRODUCT_UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "${PRODUCT_UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${PRODUCT_UNINSTALL_KEY}" "NoRepair" 1

  ExecWait '"$INSTDIR\boltsnap.exe" __install-autostart' $0
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "Boltsnap was installed, but its autostart task could not be configured (exit code $0)."
    SetErrorLevel $0
    Abort
  ${EndIf}
SectionEnd

Section "Eddy image editor (recommended)" SEC_EDDY
  SetShellVarContext current
  SetOutPath "$INSTDIR\Eddy"
  File /r "${EDDY_SOURCE_DIR}\*"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\App Paths\eddy.exe" "" "$INSTDIR\Eddy\eddy.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\App Paths\eddy.exe" "Path" "$INSTDIR\Eddy"
  WriteRegDWORD HKCU "Software\Boltsnap" "EddyStartMenuShortcut" 1
  CreateShortcut "$SMPROGRAMS\Boltsnap\Eddy.lnk" "$INSTDIR\Eddy\eddy.exe" "" "$INSTDIR\Eddy\eddy.exe"
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  IfFileExists "$INSTDIR\boltsnap.exe" 0 +2
  ExecWait '"$INSTDIR\boltsnap.exe" __remove-autostart'
  nsExec::ExecToLog '"$SYSDIR\taskkill.exe" /F /IM boltsnap-background.exe'

  Delete "$SMPROGRAMS\Boltsnap\Boltsnap.lnk"
  Delete "$SMPROGRAMS\Boltsnap\Eddy.lnk"
  RMDir "$SMPROGRAMS\Boltsnap"

  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\App Paths\boltsnap.exe"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\App Paths\eddy.exe"
  DeleteRegKey HKCU "${PRODUCT_UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\Boltsnap"
  DeleteRegValue HKCU "Control Panel\Keyboard" "PrintScreenKeyForSnippingEnabled"

  RMDir /r "$INSTDIR\Eddy"
  Delete "$INSTDIR\boltsnap.exe"
  Delete "$INSTDIR\boltsnap-background.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_BOLTSNAP} "Screenshot capture, screen recording, shelf, tray, and global Windows shortcuts."
  !insertmacro MUI_DESCRIPTION_TEXT ${SEC_EDDY} "Companion editor for arrows, text, highlighting, blur, and redaction."
!insertmacro MUI_FUNCTION_DESCRIPTION_END
