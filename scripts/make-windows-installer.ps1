# Builds the Windows installer (Inno Setup) on a Windows machine with KDE Craft:
#   1. cargo build --release with Craft's Qt 6 / KF6 (QMAKE from Craft, ghostty-vt prebuilt),
#   2. stages <Out>\stage: bin\termigator.exe, Qt via windeployqt, Kirigami / qqc2-desktop-style
#      QML and KF6 plugins from Craft, every Craft DLL the binaries load (dumpbin), the VC++ runtime
#      (app-local), share\ (Breeze colour schemes and fonts, see scripts/windows-share.sh),
#   3. collects the sources of everything from Craft in the stage (and of the Linux data in share\)
#      -> <Out>\termigator-<version>-windows-sources.tar, to publish next to the installer (LGPL),
#   4. compiles packaging\windows\termigator.iss -> <Out>\termigator-<version>-setup.exe.
#
#   powershell -ExecutionPolicy Bypass -File scripts\make-windows-installer.ps1 -Share C:\dev\share
#
# Needs: Rust (MSVC target), MSVC Build Tools (dumpbin, VC redist), KDE Craft with
# kde/frameworks/tier1/kirigami and kde/frameworks/tier3/qqc2-desktop-style, Inno Setup 6, Python 3
# (scripts/third-party-licenses.py) and network access (craft --fetch, the share\ sources).
param(
    [string]$Craft = "C:\CraftRoot",
    # Folder with color-schemes\Breeze{Light,Dark}.colors, fonts\*.ttf and icons\breeze-dark.rcc
    # (scripts/windows-share.sh).
    [Parameter(Mandatory = $true)][string]$Share,
    [string]$Out = "dist\windows",
    [string]$TargetDir = $env:CARGO_TARGET_DIR,
    [switch]$SkipBuild
)
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

function Need($path, $what) {
    if (-not (Test-Path $path)) { throw "$what not found: $path" }
}

# ---- tools
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
          "$env:ProgramFiles\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 (ISCC.exe) not found: winget install JRSoftware.InnoSetup" }
$dumpbin = Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe",
                         "$env:ProgramFiles\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" -ErrorAction SilentlyContinue |
           Sort-Object FullName | Select-Object -Last 1
if (-not $dumpbin) { throw "dumpbin.exe (MSVC Build Tools) not found" }
$crt = Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT",
                     "$env:ProgramFiles\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT" -Directory -ErrorAction SilentlyContinue |
       Sort-Object FullName | Select-Object -Last 1
if (-not $crt) { throw "VC++ redistributable folder (Microsoft.VC*.CRT) not found" }
Need "$Craft\bin\windeployqt.exe" "windeployqt (KDE Craft)"
Need "$Share\color-schemes" "colour schemes"
Need "$Share\fonts" "fonts"
Need "$Share\icons\breeze-dark.rcc" "dark Breeze icons"
Need "$Share\licenses\packages\PACKAGES.txt" "third-party licences"
Need "$Share\upstream-sources.txt" "sources list of share\ (scripts/windows-share.sh)"
$python = Get-Command python -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $python) { throw "Python 3 (python.exe) not found" }

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

# ---- build
$env:Path = "$Craft\bin;$HOME\.cargo\bin;" + $env:Path
$env:QMAKE = "$Craft\bin\qmake.exe"
$env:GHOSTTY_VT_PREBUILT_MSVC = "1"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = "rust-lld"
if (-not $TargetDir) { $TargetDir = "target" }
$env:CARGO_TARGET_DIR = $TargetDir
if (-not $SkipBuild) {
    cmd /c "cargo build --release --locked -p termigator 2>&1"
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}
$exe = Join-Path $TargetDir "release\termigator.exe"
Need $exe "release build"

# ---- stage
$stage = Join-Path $Out "stage"
$bin = Join-Path $stage "bin"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $bin | Out-Null
Copy-Item $exe $bin

# Qt itself and the QML modules our QML imports (Kirigami and the rest from Craft's qml folder).
& "$Craft\bin\windeployqt.exe" --release --no-translations --no-compiler-runtime `
    --qmldir crates\termigator\qml --qmlimport "$Craft\qml" --dir $bin (Join-Path $bin "termigator.exe")
if ($LASTEXITCODE -ne 0) { throw "windeployqt failed" }

# The offscreen platform for `termigator.exe --warm-up`, which the installer runs once (see the .iss).
Copy-Item "$Craft\plugins\platforms\qoffscreen.dll" (Join-Path $bin "platforms") -Force

# The QtQuick Controls style (org.kde.desktop) is chosen at run time, not imported: windeployqt does
# not see it. Take the whole org\kde QML tree and the KF6 plugins (Kirigami platform, style helpers).
# The contents (\*): windeployqt already made qml\org\kde, and copying the folder itself into an
# existing one would nest it (org\kde\kde).
New-Item -ItemType Directory -Force (Join-Path $bin "qml\org\kde"), (Join-Path $bin "kf6") | Out-Null
Copy-Item -Recurse -Force "$Craft\qml\org\kde\*" (Join-Path $bin "qml\org\kde")
if (Test-Path "$Craft\plugins\kf6") { Copy-Item -Recurse -Force "$Craft\plugins\kf6\*" (Join-Path $bin "kf6") }

# Every DLL from Craft that the staged binaries load, recursively (KF6, ICU, zlib, ...). DLLs not in
# Craft's bin are Windows' own.
$queue = [System.Collections.Generic.Queue[string]]::new()
Get-ChildItem $bin -Recurse -Include *.exe, *.dll | ForEach-Object { $queue.Enqueue($_.FullName) }
$seen = @{}
while ($queue.Count -gt 0) {
    $pe = $queue.Dequeue()
    foreach ($line in (& $dumpbin.FullName /nologo /dependents $pe)) {
        $name = $line.Trim()
        if ($name -notmatch '\.dll$' -or $seen.ContainsKey($name.ToLower())) { continue }
        $seen[$name.ToLower()] = $true
        $from = Join-Path "$Craft\bin" $name
        $to = Join-Path $bin $name
        if ((Test-Path $from) -and -not (Test-Path $to)) {
            Copy-Item $from $to
            $queue.Enqueue($to)
        }
    }
}

# The VC++ runtime next to the exe (no separate redistributable install).
Copy-Item "$($crt.FullName)\*.dll" $bin -Force

# No DLL without its licence: each one maps to a folder of share\licenses\packages
# (scripts/windows-share.sh collects them); $null = Microsoft's redistributable VC++ runtime.
$licenseOf = [ordered]@{
    '^Qt6' = 'qt6-base'; '^KF6Archive' = 'karchive'; '^KF6ColorScheme' = 'kcolorscheme'; '^KF6Config' = 'kconfig'
    '^KF6CoreAddons' = 'kcoreaddons'; '^KF6GuiAddons' = 'kguiaddons'; '^KF6I18n' = 'ki18n'; '^KF6Icon' = 'kiconthemes'
    '^KF6WidgetsAddons' = 'kwidgetsaddons'; '^KF6Sonnet' = 'sonnet'; '^KF6BreezeIcons' = 'breeze-icons'
    '^Kirigami' = 'kirigami'; '^icu' = 'icu'; '^freetype' = 'freetype2'; '^harfbuzz' = 'harfbuzz'; '^brotli' = 'brotli'
    '^bz2' = 'bzip2'; '^iconv' = 'libiconv'; '^intl' = 'gettext'; '^jpeg' = 'libjpeg-turbo'; '^lib(crypto|ssl)' = 'openssl'
    '^liblzma' = 'xz'; '^libpng' = 'libpng'; '^pcre2' = 'pcre2'; '^zlib' = 'zlib'; '^zstd' = 'zstd'; '^b2-' = 'libb2'
    '^(msvcp|vcruntime|vccorlib|concrt|vcomp)' = $null
}
foreach ($dll in Get-ChildItem $bin -Filter *.dll) {
    $key = $licenseOf.Keys | Where-Object { $dll.Name -match $_ } | Select-Object -First 1
    if (-not $key) { throw "No licence for $($dll.Name): map it here and collect it in scripts/windows-share.sh" }
    $pkg = $licenseOf[$key]
    if ($pkg -and -not (Test-Path (Join-Path $Share "licenses\packages\$pkg"))) {
        throw "Licence of $($dll.Name) missing: $Share\licenses\packages\$pkg (scripts/windows-share.sh)"
    }
}

# Sources (LGPL / GPL) of what came from Craft, matched file by file, and of share\'s Linux data.
$sourcesTar = "termigator-$version-windows-sources.tar"
$sources = Join-Path $Out "sources"
if (Test-Path $sources) { Remove-Item -Recurse -Force $sources }
$env:SOURCES_ARCHIVE = $sourcesTar
& $python.Source -I scripts\third-party-licenses.py craft-sources $stage $sources --craft $Craft `
    --share-sources (Join-Path $Share "upstream-sources.txt")
if ($LASTEXITCODE -ne 0) { throw "collecting the sources failed" }
tar -cf (Join-Path $Out $sourcesTar) -C $Out sources
if ($LASTEXITCODE -ne 0) { throw "tar of the sources failed" }

# Breeze colour schemes and fonts: applyFixedLook reads <exe dir>\..\share.
Copy-Item -Recurse -Force $Share (Join-Path $stage "share")
# Where the sources are, next to the licences in the installed app.
Copy-Item (Join-Path $sources "SOURCES.txt") (Join-Path $stage "share\licenses\packages\SOURCES.txt")

# ---- installer
$outFull = (Resolve-Path $Out).Path
$stageFull = (Resolve-Path $stage).Path
& $iscc "/DAppVersion=$version" "/DStage=$stageFull" "/DOutDir=$outFull" packaging\windows\termigator.iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
$setup = Join-Path $outFull "termigator-$version-setup.exe"
"installer: $setup ($([math]::Round((Get-Item $setup).Length / 1MB, 1)) MB)"
"sources: $(Join-Path $Out $sourcesTar) ($([math]::Round((Get-Item (Join-Path $Out $sourcesTar)).Length / 1MB, 1)) MB)"
