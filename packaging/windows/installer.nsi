Unicode true
!include "MUI2.nsh"
!ifndef STAGE
!error "Pass /DSTAGE=absolute-staged-package-directory"
!endif
!ifndef VERSION
!error "Pass /DVERSION=release-version"
!endif
Name "YamiNet ${VERSION}"
!ifndef OUTPUT
!define OUTPUT "YamiNet-${VERSION}-windows-x86_64-setup.exe"
!endif
OutFile "${OUTPUT}"
InstallDir "$LOCALAPPDATA\Programs\YamiNet"
RequestExecutionLevel user
SetCompressor /SOLID lzma
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "YamiNet desktop"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File /r "${STAGE}\*"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\YamiNet"
  CreateShortcut "$SMPROGRAMS\YamiNet\Yami Browser.lnk" "$INSTDIR\yami-browser.exe" "--desktop"
  CreateShortcut "$SMPROGRAMS\YamiNet\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\YamiNet" "DisplayName" "YamiNet ${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\YamiNet" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\YamiNet" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
SectionEnd
Section "Uninstall"
  SetShellVarContext current
  ; Application directory is exclusively application files. User state lives elsewhere.
  Delete "$SMPROGRAMS\YamiNet\Yami Browser.lnk"
  Delete "$SMPROGRAMS\YamiNet\Uninstall.lnk"
  RMDir "$SMPROGRAMS\YamiNet"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\YamiNet"
  ; Delete only installed application subdirectories and known top-level files.
  RMDir /r "$INSTDIR\docs"
  RMDir /r "$INSTDIR\fonts"
  RMDir /r "$INSTDIR\examples"
  RMDir /r "$INSTDIR\sites"
  RMDir /r "$INSTDIR\config-examples"
  RMDir /r "$INSTDIR\games"
  Delete "$INSTDIR\yami-browser.exe"
  Delete "$INSTDIR\yami-node.exe"
  Delete "$INSTDIR\yami-relay.exe"
  Delete "$INSTDIR\Launch Browser.cmd"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\SECURITY.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\VERSION.txt"
  Delete "$INSTDIR\MANIFEST.sha256"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
