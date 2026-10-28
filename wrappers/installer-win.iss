; The Windows installer for the vfx-core plugins: OpenFX (Resolve, Nuke, Natron), FFGL
; (Resolume) and After Effects / Premiere Pro. package-win.sh builds the plugins, then:
;   ISCC -DVersion=1.2.3 -DSource=<folder with the built plugins> installer-win.iss
; Only the plugins found in Source are offered. -DTestRoot=<folder> installs everything
; under that folder instead, without administrator rights, for testing the installer.

#ifndef Version
  #define Version "dev"
#endif
#ifndef Source
  #define Source "..\target\vfx-plugin"
#endif

#ifdef TestRoot
  #define OfxPlugins TestRoot + "\OFX\Plugins"
  #define MediaCore TestRoot + "\MediaCore"
  #define Documents TestRoot + "\Documents"
  #define AppDir TestRoot + "\App"
#else
  #define OfxPlugins "{commoncf64}\OFX\Plugins"
  #define MediaCore "{commonpf64}\Adobe\Common\Plug-ins\7.0\MediaCore"
  #define Documents "{userdocs}"
  #define AppDir "{commonpf64}\VFX Preview"
#endif

[Setup]
AppId={{2FFBD5EA-ABD9-4A8F-9ACC-24CF0C2C2E3E}
AppName=VFX Preview
AppVersion={#Version}
AppPublisher=vfx-core
AppPublisherURL=https://github.com/jamecoyne/vfx-core
UninstallDisplayName=VFX Preview plugins
; The folder only holds the uninstaller; the plugins go where each app looks.
DefaultDirName={#AppDir}
DisableDirPage=yes
DisableProgramGroupPage=yes
#ifdef TestRoot
PrivilegesRequired=lowest
#else
PrivilegesRequired=admin
#endif
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
Compression=lzma2
SolidCompression=yes

[Types]
Name: "recommended"; Description: "Resolve and Resolume"
Name: "custom"; Description: "Choose plugins"; Flags: iscustom

[Components]
#if DirExists(Source + "\VfxPreview.ofx.bundle")
Name: "ofx"; Description: "DaVinci Resolve, Nuke, Natron (OpenFX)"; Types: recommended
#endif
#if FileExists(Source + "\VfxPreview.dll")
Name: "ffgl"; Description: "Resolume Avenue and Arena (FFGL)"; Types: recommended
#endif
#if FileExists(Source + "\VfxAdobe.aex")
Name: "adobe"; Description: "After Effects and Premiere Pro (not tested on Windows yet)"
#endif

[Files]
#if DirExists(Source + "\VfxPreview.ofx.bundle")
Source: "{#Source}\VfxPreview.ofx.bundle\*"; DestDir: "{#OfxPlugins}\VfxPreview.ofx.bundle"; Components: ofx; Flags: recursesubdirs createallsubdirs ignoreversion
#endif
#if FileExists(Source + "\VfxPreview.dll")
Source: "{#Source}\VfxPreview.dll"; DestDir: "{#Documents}\Resolume Avenue\Extra Effects"; Components: ffgl; Check: ForAvenue; Flags: ignoreversion
Source: "{#Source}\VfxPreview.dll"; DestDir: "{#Documents}\Resolume Arena\Extra Effects"; Components: ffgl; Check: ForArena; Flags: ignoreversion
#endif
#if FileExists(Source + "\VfxAdobe.aex")
Source: "{#Source}\VfxAdobe.aex"; DestDir: "{#MediaCore}\VfxPreview"; Components: adobe; Flags: ignoreversion
#endif

[Code]
// Resolume makes its Documents folder on first start. Arena gets the plugin when its
// folder is there; Avenue's is the default, so a later Avenue install finds it.
function ForArena: Boolean;
begin
  Result := DirExists(ExpandConstant('{#Documents}\Resolume Arena'));
end;

function ForAvenue: Boolean;
begin
  Result := DirExists(ExpandConstant('{#Documents}\Resolume Avenue')) or not ForArena;
end;
