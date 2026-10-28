; Modo installer: per-user, no admin rights needed.
; Built by scripts/package-windows.ps1, which defines VERSION, NUMERIC_VERSION,
; STAGING (folder with modo.exe, modo.ico, LICENSE), and OUTFILE.
Unicode true
!include "MUI2.nsh"
!include "x64.nsh"

Name "Modo"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Modo"
InstallDirRegKey HKCU "Software\Modo" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
VIProductVersion "${NUMERIC_VERSION}.0"
VIAddVersionKey "ProductName" "Modo"
VIAddVersionKey "FileDescription" "Modo installer"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Modo"
!define MUI_ICON "${STAGING}\modo.ico"
!define MUI_UNICON "${STAGING}\modo.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\modo.exe"
!insertmacro MUI_PAGE_LICENSE "${STAGING}\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "Modo requires 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"
  File "${STAGING}\modo.exe"
  File "${STAGING}\modo.ico"
  File "${STAGING}\LICENSE"
  File /nonfatal "${STAGING}\THIRD_PARTY_NOTICES.md"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Modo.lnk" "$INSTDIR\modo.exe" "" "$INSTDIR\modo.ico" 0

  ; "Open with Modo" on folders and inside folders in Explorer.
  WriteRegStr HKCU "Software\Classes\Directory\shell\Modo" "" "Open with Modo"
  WriteRegStr HKCU "Software\Classes\Directory\shell\Modo" "Icon" "$INSTDIR\modo.ico"
  WriteRegStr HKCU "Software\Classes\Directory\shell\Modo\command" "" '"$INSTDIR\modo.exe" "%V"'
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\Modo" "" "Open with Modo"
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\Modo" "Icon" "$INSTDIR\modo.ico"
  WriteRegStr HKCU "Software\Classes\Directory\Background\shell\Modo\command" "" '"$INSTDIR\modo.exe" "%V"'

  WriteRegStr HKCU "Software\Modo" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Modo"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\modo.ico"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "Modo"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetRegView 64
  Delete "$INSTDIR\modo.exe"
  Delete "$INSTDIR\modo.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Modo.lnk"
  DeleteRegKey HKCU "Software\Classes\Directory\shell\Modo"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\Modo"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\Modo"
SectionEnd
