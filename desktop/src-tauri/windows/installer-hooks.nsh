; NSIS installer hooks for Token Router.
; InstFilesShow/Leave disable the title-bar close button and Cancel during installation.

!ifndef SC_CLOSE
  !define SC_CLOSE 0xF060
!endif
!ifndef MF_BYCOMMAND
  !define MF_BYCOMMAND 0x00000000
!endif
!ifndef MF_GRAYED
  !define MF_GRAYED 0x00000001
!endif
!ifndef MF_ENABLED
  !define MF_ENABLED 0x00000000
!endif

Function InstFilesShow
  System::Call 'user32::GetSystemMenu(p $HWNDPARENT, i 0) p.r0'
  System::Call 'user32::EnableMenuItem(p r0, i ${SC_CLOSE}, i ${MF_BYCOMMAND}|${MF_GRAYED})'
  GetDlgItem $0 $HWNDPARENT 2
  EnableWindow $0 0
FunctionEnd

Function InstFilesLeave
  System::Call 'user32::GetSystemMenu(p $HWNDPARENT, i 0) p.r0'
  System::Call 'user32::EnableMenuItem(p r0, i ${SC_CLOSE}, i ${MF_BYCOMMAND}|${MF_ENABLED})'
  GetDlgItem $0 $HWNDPARENT 2
  EnableWindow $0 1
FunctionEnd

!macro InstallerDisableClose
  Call InstFilesShow
!macroend

!macro InstallerEnableClose
  Call InstFilesLeave
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro InstallerDisableClose
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro InstallerEnableClose
!macroend
