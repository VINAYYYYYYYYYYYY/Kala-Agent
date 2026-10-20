# Build Kala Studio for Windows (MSVC + NSIS)
#
# Native Rust backend + Vite UI + bundled OCCT DLLs + FreeCAD adapter and its
# Python dependencies vendored for FreeCAD's OWN bundled interpreter, then an
# electron-builder NSIS installer. This script does not sign, publish or
# install the application. The final runtime check is the post-install smoke
# checklist printed at the end (run it on the Windows machine).
#
# FreeCAD dependency bundling:
#   The adapter runs inside `FreeCADCmd.exe`, not the repository virtualenv.
#   The script locates FreeCADCmd (parameter, PATH, then
#   %ProgramFiles%\FreeCAD*\bin) and asks that interpreter for its
#   version/cache_tag only to verify the bundled set. scripts/native-backend.cjs
#   always vendors EVERY supported CPython ABI (cp311, cp312, cp313, cp314) as
#   win_amd64 wheels with `pip download --only-binary=:all:`, unpacked into
#   backend-native\adapters\freecad\site-packages-cpNNN\. The provider probes
#   FreeCADCmd's Python at runtime and puts only the matching directory on
#   sys.path, so one installer works with every current FreeCAD build. Every
#   wheel digest is recorded in ...\adapters\freecad\vendor.json. FreeCAD itself
#   is NOT bundled. When FreeCAD is absent, -FreeCadPythonVersion/-FreeCadAbi
#   remain as a development aid to restrict the vendored set to one interpreter
#   (the resulting installer is then deliberately single-ABI and the post-stage
#   check requires only that ABI).
#
# Prerequisites (see README.md for versions and one-time setup):
#   * rustup with the MSVC toolchain (rustc/cargo on PATH)
#   * Visual Studio 2022 Build Tools with the "Desktop development with C++" workload
#   * Node.js 22.12 or newer (node/npm on PATH)
#   * git
#   * vcpkg with opencascade:x64-windows installed
#   * FreeCAD 1.1.x x64 (optional; the provider needs it at runtime)
#
# Usage (from the repository root or anywhere):
#   powershell -ExecutionPolicy Bypass -File packaging\windows\build-windows.ps1
#   powershell ... -VcpkgRoot C:\vcpkg -VcpkgTriplet x64-windows -RunGates
#   powershell ... -FreeCadExe "C:\Program Files\FreeCAD 1.1\bin\FreeCADCmd.exe"
#
# The script stops on the first failure. It never invokes sudo/elevation.

[CmdletBinding()]
param(
    [string]$VcpkgRoot = $env:VCPKG_ROOT,
    [string]$VcpkgTriplet = "x64-windows",
    [string]$FreeCadExe = "",
    [string]$FreeCadPythonVersion = "",
    [string]$FreeCadAbi = "",
    [switch]$SkipNpmInstall,
    [switch]$RunGates,
    [switch]$SkipBuildTests
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Write-Step([string]$Message) {
    Write-Host ""
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Require-Command([string]$Name, [string]$Hint) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found. $Hint"
    }
}

function Invoke-Checked([string]$Command, [string[]]$Arguments) {
    Write-Host "    $Command $($Arguments -join ' ')"
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "'$Command' exited with code $LASTEXITCODE"
    }
}

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = (Resolve-Path (Join-Path $ScriptDir "..\..")).Path
$StudioDir = Join-Path $RepoRoot "studio"
$BackendDir = Join-Path $RepoRoot "backend"
$StageDir = Join-Path $StudioDir "backend-native"
$ReleaseDir = Join-Path $StudioDir "release"

Write-Host "Kala Studio Windows build" -ForegroundColor Green
Write-Host "  repository: $RepoRoot"
Write-Host "  vcpkg root: $VcpkgRoot"
Write-Host "  triplet:    $VcpkgTriplet"

Write-Step "Checking prerequisites"
if ([string]::IsNullOrWhiteSpace($VcpkgRoot) -or -not (Test-Path $VcpkgRoot)) {
    throw "VCPKG_ROOT is not set and -VcpkgRoot was not supplied. Set it to your vcpkg checkout."
}
Require-Command "rustc" "Install rustup and the MSVC toolchain: https://rustup.rs"
Require-Command "cargo" "Install rustup and the MSVC toolchain: https://rustup.rs"
Require-Command "node" "Install Node.js 22.12 or newer: https://nodejs.org"
Require-Command "npm" "Install Node.js 22.12 or newer: https://nodejs.org"
Require-Command "git" "Install Git for Windows: https://git-scm.com"

$NodeVersion = (& node --version).TrimStart("v")
$NodeMajor = [int]($NodeVersion.Split(".")[0])
if ($NodeMajor -lt 22) {
    throw "Node.js 22.12 or newer is required (found $NodeVersion)."
}
$RustHost = (& rustc -vV | Select-String "^host:").ToString().Trim()
if ($RustHost -notmatch "windows-msvc") {
    throw "The active Rust host is '$RustHost'; install the x86_64-pc-windows-msvc toolchain."
}
& cargo --version | Write-Host

$InstalledRoot = Join-Path $VcpkgRoot "installed\$VcpkgTriplet"
$OcctInclude = Join-Path $InstalledRoot "include\opencascade"
$OcctLib = Join-Path $InstalledRoot "lib"
$OcctBin = Join-Path $InstalledRoot "bin"
$StandardHeader = Join-Path $OcctInclude "Standard_Version.hxx"
$KernelDll = Join-Path $OcctBin "TKernel.dll"

if (-not (Test-Path $StandardHeader) -or -not (Test-Path $KernelDll)) {
    throw @"
OpenCascade was not found under $InstalledRoot.
Install it once with:
    $VcpkgRoot\vcpkg.exe install opencascade:$VcpkgTriplet
Expected:
    $StandardHeader
    $KernelDll
"@
}
Write-Host "    OpenCascade: $OcctInclude"

# The Rust build discovers OCCT through KALA_OCCT_ROOT / KALA_OCCT_INCLUDE_DIR /
# KALA_OCCT_LIB_DIR (see backend/crates/kala-cad/build.rs).
$env:KALA_OCCT_ROOT = $InstalledRoot
$env:KALA_OCCT_INCLUDE_DIR = $OcctInclude
$env:KALA_OCCT_LIB_DIR = $OcctLib
$env:PATH = "$OcctBin;$env:PATH"

Set-Location $StudioDir

if (-not $SkipNpmInstall) {
    Write-Step "Installing Studio dependencies (npm ci)"
    Invoke-Checked "npm" @("ci", "--no-audit", "--no-fund")
}

if ($RunGates) {
    Write-Step "Studio typecheck and tests"
    Invoke-Checked "npm" @("run", "typecheck")
    if (-not $SkipBuildTests) {
        Invoke-Checked "npm" @("test")
    }
}

Write-Step "Building the Vite UI"
Invoke-Checked "npm" @("run", "build")

# The adapter is imported by FreeCADCmd, not by the repository virtualenv.
# Never inherit a Python injection into the staging helpers.
Remove-Item Env:PYTHONPATH -ErrorAction SilentlyContinue
Remove-Item Env:PYTHONHOME -ErrorAction SilentlyContinue

Write-Step "Detecting FreeCAD's bundled Python (verification only), then vendoring every supported ABI"
$FreeCadVendorArgs = @()
$FreeCadDetected = $null
if ([string]::IsNullOrWhiteSpace($FreeCadExe)) {
    $FreeCadCommand = Get-Command "FreeCADCmd.exe" -ErrorAction SilentlyContinue
    if ($null -ne $FreeCadCommand) {
        $FreeCadExe = $FreeCadCommand.Source
    }
}
if ([string]::IsNullOrWhiteSpace($FreeCadExe)) {
    foreach ($Root in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
        if ([string]::IsNullOrWhiteSpace($Root)) { continue }
        foreach ($Candidate in @(Get-ChildItem -Path $Root -Directory -Filter "FreeCAD*" -ErrorAction SilentlyContinue)) {
            foreach ($Relative in @("bin\FreeCADCmd.exe", "FreeCADCmd.exe")) {
                $Probe = Join-Path $Candidate.FullName $Relative
                if (Test-Path $Probe) { $FreeCadExe = $Probe; break }
            }
            if (-not [string]::IsNullOrWhiteSpace($FreeCadExe)) { break }
        }
        if (-not [string]::IsNullOrWhiteSpace($FreeCadExe)) { break }
    }
}

if (-not [string]::IsNullOrWhiteSpace($FreeCadExe)) {
    if (-not (Test-Path $FreeCadExe)) {
        throw "FreeCADCmd was supplied but does not exist: $FreeCadExe"
    }
    $FreeCadProbe = Join-Path $env:TEMP ("kala-freecad-python-" + [guid]::NewGuid().ToString("N") + ".py")
    @'
import json, sys, sysconfig
print("KALA_FREECAD_PYTHON=" + json.dumps({
    "version": "%d.%d" % tuple(sys.version_info[:2]),
    "fullVersion": sys.version.split()[0],
    "cacheTag": sys.implementation.cache_tag,
    "abi": "cp%d%d" % tuple(sys.version_info[:2]),
    "platform": sysconfig.get_platform(),
}))
'@ | Set-Content -Encoding UTF8 -Path $FreeCadProbe
    try {
        $FreeCadProbeOutput = & $FreeCadExe $FreeCadProbe 2>&1 | Out-String
    } finally {
        Remove-Item -Force $FreeCadProbe -ErrorAction SilentlyContinue
    }
    $FreeCadProbeLine = ($FreeCadProbeOutput -split "`r?`n" |
        Where-Object { $_ -like "KALA_FREECAD_PYTHON=*" } | Select-Object -First 1)
    if ([string]::IsNullOrWhiteSpace($FreeCadProbeLine)) {
        throw "FreeCADCmd did not report its Python identity:`n$FreeCadProbeOutput"
    }
    $FreeCadDetected = $FreeCadProbeLine.Substring("KALA_FREECAD_PYTHON=".Length) | ConvertFrom-Json
    Write-Host "    FreeCADCmd:    $FreeCadExe"
    Write-Host "    FreeCAD Python: $($FreeCadDetected.fullVersion) " +
        "($($FreeCadDetected.cacheTag), ABI $($FreeCadDetected.abi), $($FreeCadDetected.platform))"
    $FreeCadVendorArgs = @("--freecad-python", $FreeCadExe, "--require-freecad-vendor")
} else {
    Write-Warning ("FreeCAD was not detected. Every supported ABI (cp311, cp312, cp313, cp314) " +
        "will still be vendored as win_amd64 wheels; only the in-FreeCADCmd import check is " +
        "skipped. Install FreeCAD 1.1.x x64 or pass -FreeCadExe to run that check.")
    $FreeCadVendorArgs = @()
    if (-not [string]::IsNullOrWhiteSpace($FreeCadPythonVersion) -or
        -not [string]::IsNullOrWhiteSpace($FreeCadAbi)) {
        if ([string]::IsNullOrWhiteSpace($FreeCadAbi) -and
            -not [string]::IsNullOrWhiteSpace($FreeCadPythonVersion)) {
            $FreeCadAbi = "cp" + $FreeCadPythonVersion.Replace(".", "")
        }
        if ([string]::IsNullOrWhiteSpace($FreeCadPythonVersion) -and
            -not [string]::IsNullOrWhiteSpace($FreeCadAbi)) {
            $FreeCadPythonVersion = "3." + $FreeCadAbi.Substring(3)
        }
        if ([string]::IsNullOrWhiteSpace($FreeCadPythonVersion)) { $FreeCadPythonVersion = "3.14" }
        if ([string]::IsNullOrWhiteSpace($FreeCadAbi)) { $FreeCadAbi = "cp314" }
        Write-Warning ("Restricting the vendored set to $FreeCadPythonVersion/$FreeCadAbi " +
            "(development aid); the installer will not work with other FreeCAD Pythons.")
        $FreeCadVendorArgs = @(
            "--freecad-python-version", $FreeCadPythonVersion,
            "--freecad-abi", $FreeCadAbi
        )
    }
}

Write-Step "Building and staging kala-studio.exe + studio_dist + OCCT DLLs + FreeCAD adapter"
# One helper call builds the locked release binary and stages the runtime.
# --occt-lib-dir copies the vcpkg DLLs next to kala-studio.exe, where the
# Windows loader finds them (and the Electron main process also prepends the
# backend directory to PATH). The FreeCAD flags vendor win_amd64 wheels for
# every supported ABI and verify the build host's ABI inside FreeCADCmd.
$StageArguments = @("scripts/native-backend.cjs", "--occt-lib-dir", $OcctBin) + $FreeCadVendorArgs
Invoke-Checked "node" $StageArguments

$Binary = Join-Path $StageDir "kala-studio.exe"
if (-not (Test-Path $Binary)) {
    throw "Staging failed: $Binary is missing."
}
if (-not (Test-Path (Join-Path $StageDir "adapters\freecad\src\kala_agent_freecad\__init__.py"))) {
    throw "Staging failed: the FreeCAD adapter is missing."
}
$VendorManifestPath = Join-Path $StageDir "adapters\freecad\vendor.json"
if (-not (Test-Path $VendorManifestPath)) {
    throw "Staging failed: the vendored FreeCAD Python dependencies are missing (vendor.json)."
}
$VendorManifest = Get-Content -Raw -Path $VendorManifestPath | ConvertFrom-Json
if ($VendorManifest.schemaVersion -ne 2) {
    throw "Staging failed: the FreeCAD vendor manifest schema is not 2."
}
$ExpectedAbis = @("cp311", "cp312", "cp313", "cp314")
if ($FreeCadVendorArgs -contains "--freecad-abi") {
    # Explicit development restriction: the installer is single-ABI by choice.
    $ExpectedAbis = @($FreeCadAbi)
}
$VendoredAbis = @($VendorManifest.supportedAbis)
foreach ($Abi in $ExpectedAbis) {
    if ($VendoredAbis -notcontains $Abi) {
        throw "Staging failed: the vendored FreeCAD ABI set is missing $Abi."
    }
    $AbiRoot = Join-Path $StageDir "adapters\freecad\site-packages-$Abi"
    foreach ($RequiredPackage in @("pydantic", "pydantic_core")) {
        if (-not (Test-Path (Join-Path $AbiRoot "$RequiredPackage\__init__.py"))) {
            throw "Staging failed: $Abi is missing the vendored package '$RequiredPackage'."
        }
    }
    $PydanticCoreExtension = @(Get-ChildItem -Path (Join-Path $AbiRoot "pydantic_core") `
        -Filter "_pydantic_core*.pyd" -File -ErrorAction SilentlyContinue)
    if ($PydanticCoreExtension.Count -eq 0) {
        throw "Staging failed: the vendored pydantic-core C extension (.pyd) is missing for $Abi."
    }
    if (-not (Test-Path (Join-Path $AbiRoot "pydantic_core\__init__.py"))) {
        throw "Staging failed: the vendored pydantic_core package is incomplete for $Abi."
    }
}
if (-not (Test-Path (Join-Path $StageDir "adapters\freecad\src\kala_agent\__init__.py"))) {
    throw "Staging failed: the shared kala_agent package is missing."
}
if ($null -ne $FreeCadDetected -and $VendoredAbis -notcontains $FreeCadDetected.abi) {
    throw ("Staging failed: FreeCAD's Python ABI $($FreeCadDetected.abi) is not in the " +
        "vendored set ($($VendoredAbis -join ', ')).")
}
$OcctDlls = @(Get-ChildItem -Path $StageDir -Filter "TK*.dll" -File)
if ($OcctDlls.Count -eq 0) {
    throw "Staging failed: no OCCT DLLs were copied next to kala-studio.exe."
}
$WheelCount = 0
foreach ($Abi in $VendoredAbis) {
    $WheelCount += @($VendorManifest.abis.$Abi.wheels).Count
}
Write-Host ("    binary: {0} ({1:N0} bytes)" -f $Binary, (Get-Item $Binary).Length)
Write-Host ("    OCCT DLLs: {0}; FreeCAD deps: ABIs {1}; {2} wheels, {3} files" -f `
    $OcctDlls.Count, ($VendoredAbis -join ", "), $WheelCount, $VendorManifest.fileCount)
if ($null -ne $FreeCadDetected) {
    Write-Host ("    build-host FreeCAD: {0} / Python {1} ({2}); imports verified" -f `
        $FreeCadDetected.version, $FreeCadDetected.fullVersion, $FreeCadDetected.abi)
} else {
    Write-Host "    build-host FreeCAD: not installed; in-FreeCAD import check skipped"
}

Write-Step "Building the NSIS installer with electron-builder"
Invoke-Checked "node" @(
    "scripts/desktop-pack.cjs",
    "--win", "nsis", "--x64",
    "--publish", "never"
)

$Installer = Get-ChildItem -Path $ReleaseDir -Filter "*.exe" -File |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if ($null -eq $Installer) {
    throw "electron-builder did not produce an installer under $ReleaseDir"
}
$Hash = (Get-FileHash -Algorithm SHA256 -Path $Installer.FullName).Hash.ToLowerInvariant()

Write-Host ""
Write-Host "Installer: $($Installer.FullName)" -ForegroundColor Green
Write-Host ("Size:      {0:N0} bytes" -f $Installer.Length)
Write-Host "SHA-256:   $Hash"
Write-Host ""
Write-Host "Post-install smoke checklist (run on this Windows machine):" -ForegroundColor Yellow
@"
1. Install the produced EXE as the current user (do not run as administrator).
2. Set a throwaway data directory before the first launch so real user data is
   not touched:
       set KALA_AGENT_DATA_DIR=%TEMP%\kala-studio-smoke-data
       set KALA_STUDIO_DATA_DIR=%TEMP%\kala-studio-smoke-data
3. Launch "Kala Studio" from the Start menu. Only one kala-studio.exe child may
   appear in Task Manager; no python.exe may appear.
4. The window must load the workspace from http://127.0.0.1:<random port>/.
   Check %TEMP%\... only for the throwaway data directory; the journal lives
   under KALA_AGENT_DATA_DIR.
5. Health: in a second terminal,
       curl.exe http://127.0.0.1:<port>/api/studio/health
   The exact port is not printed to the UI; use the backend child command line
   or netstat to find the loopback listener. status must be "ok".
6. OpenCascade design: request the L-bracket through the UI or POST
   /api/studio/design with provider "opencascade". volume_mm3 must be
   35571.416047 (rel 1e-6), bounds [-30,30] x [0,45] x [0,40].
7. FreeCAD (install FreeCAD 1.1.x x64; it is NOT bundled): POST
   /api/studio/design with provider "freecad". It must build in FreeCADCmd and
   report volume 35571.416047 (rel 1e-6). The installed app supplies the
   adapter from resources\backend\adapters\freecad\src and one matching
   resources\backend\adapters\freecad\site-packages-cpNNN per supported
   FreeCAD Python (cp311-cp314); no virtualenv, no repository checkout and no
   PYTHONPATH are needed. The provider probes FreeCADCmd's Python and uses only
   the matching directory, so FreeCAD 1.1.x with Python 3.11, 3.12, 3.13 or
   3.14 is supported without rebuilding. An unsupported Python is reported as
   provider-unavailable with its version in the reason.
8. Export STEP and re-import it; the view bounds must match within 0.001 mm.
9. Close the window; kala-studio.exe must exit (no orphan process). No
   python.exe may remain; FreeCADCmd.exe may appear only while a FreeCAD design
   is running.
10. Uninstall from Settings; the user data directory must be left in place.
"@
Write-Host ""
Write-Host "No signing, publishing or installation was performed by this script." -ForegroundColor DarkGray
