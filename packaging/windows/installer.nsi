; Instalador de Hive Agents (Rune) para Windows. Se compila con: makensis /DVERSION=x.y.z /DSTAGE=<carpeta> /DOUT=<archivo.exe> installer.nsi
; Instala por usuario (sin administrador) en %LOCALAPPDATA%\Programs\Hive Agents.
!include "MUI2.nsh"
Name "Hive Agents"
OutFile "${OUT}"
InstallDir "$LOCALAPPDATA\Programs\Hive Agents"
RequestExecutionLevel user
!define MUI_ICON "${STAGE}\hive-agents.ico"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Spanish"

Section "Instalar"
  SetOutPath "$INSTDIR"
  File "${STAGE}\hive-desktop-rune.exe"
  File "${STAGE}\hive-gateway-rust.exe"
  File "${STAGE}\hive-agents.ico"
  WriteUninstaller "$INSTDIR\desinstalar.exe"
  CreateShortcut "$SMPROGRAMS\Hive Agents.lnk" "$INSTDIR\hive-desktop-rune.exe" "" "$INSTDIR\hive-agents.ico"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\HiveAgentsRune" "DisplayName" "Hive Agents"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\HiveAgentsRune" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\HiveAgentsRune" "UninstallString" "$INSTDIR\desinstalar.exe"
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\Hive Agents.lnk"
  Delete "$INSTDIR\hive-desktop-rune.exe"
  Delete "$INSTDIR\hive-gateway-rust.exe"
  Delete "$INSTDIR\hive-agents.ico"
  Delete "$INSTDIR\desinstalar.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\HiveAgentsRune"
  ; No se toca %USERPROFILE%\.hive: ahí están los datos del usuario.
SectionEnd
