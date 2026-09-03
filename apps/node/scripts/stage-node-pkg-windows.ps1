# Stage the bundled BTX node binaries for an easyBTX Node WINDOWS build.
#
# Takes the output of the btxd-windows.yml CI build and stages the flat package
# layout resolve_bundled_node_pkg() probes:
#   src-tauri\resources\node-pkg\bin\{btxd.exe, btx-cli.exe, *.dll}
#
# NOT a static triplet. btxd-windows.yml cross-compiles with mingw-w64, and the
# result imports at least libgomp-1.dll. 0.6.6 shipped without it: btxd.exe died
# with 0xC0000135 (STATUS_DLL_NOT_FOUND) on every machine that was not the build
# runner, the app's `--version` probe hung instead of failing, and the app fell
# back to the OLD node while reporting the new app version. Any DLL beside
# btxd.exe in $SourceDir is therefore vendored, and the run check below is the
# gate that proves the staged tree works with nothing but system DLLs on PATH.
#
# Usage: .\stage-node-pkg-windows.ps1 -SourceDir <dir> [-VerifyShaFile <SHA256SUMS.txt>]
#                                     [-ExpectedVersion v0.33.2]
#
# -ExpectedVersion asserts the staged btxd.exe really IS that release. It matters
# more than it looks: NODE_RELEASE_TAG decides the install directory, and
# node.rs derives its version-gated CLI flags (-autoupdate, and since the MatMul
# v4.7 fork -matmulrcexecution) from that directory name. Staging the wrong bytes
# under the right tag hands a flag to a btxd that rejects unknown args fatally,
# and the node then refuses to start at all.
param(
    [Parameter(Mandatory = $true)][string]$SourceDir,
    [string]$VerifyShaFile = "",
    [string]$ExpectedVersion = ""
)
$ErrorActionPreference = "Stop"

$AppDir = Split-Path -Parent $PSScriptRoot
$Dest = Join-Path $AppDir "src-tauri\resources\node-pkg"

foreach ($name in "btxd.exe", "btx-cli.exe") {
    if (-not (Test-Path (Join-Path $SourceDir $name))) {
        throw "missing $name in $SourceDir"
    }
}

if ($VerifyShaFile) {
    # Lines are "<sha256-lower>  <name>" (the artifact's SHA256SUMS.txt).
    foreach ($line in Get-Content $VerifyShaFile) {
        if ($line -notmatch '^([0-9a-f]{64})\s+(\S+)$') { continue }
        $want = $Matches[1]; $name = $Matches[2]
        $got = (Get-FileHash (Join-Path $SourceDir $name) -Algorithm SHA256).Hash.ToLower()
        if ($got -ne $want) { throw "sha256 mismatch for ${name}: got $got, want $want" }
        Write-Host "verified ${name}: $got"
    }
}

if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
New-Item -ItemType Directory -Force (Join-Path $Dest "bin") | Out-Null
Copy-Item (Join-Path $SourceDir "btxd.exe") (Join-Path $Dest "bin")
Copy-Item (Join-Path $SourceDir "btx-cli.exe") (Join-Path $Dest "bin")

# Vendor every DLL the build shipped next to the binaries. mingw links libgomp
# dynamically, so the package is only self-contained if these travel with it.
$dlls = @(Get-ChildItem -Path $SourceDir -Filter *.dll -File -ErrorAction SilentlyContinue)
foreach ($d in $dlls) {
    Copy-Item $d.FullName (Join-Path $Dest "bin")
    Write-Host "vendored $($d.Name)"
}

# Run the staged btxd with a SYSTEM-ONLY PATH.
#
# This is the check that would have caught the 0.6.6 break. Inheriting the
# runner's PATH is what hid it: GitHub's Windows images carry mingw, so the
# missing libgomp-1.dll resolved in CI and only failed once it reached a user.
# Resolving imports against System32 alone reproduces a clean user machine.
$probe = Join-Path $Dest "bin\btxd.exe"
$savedPath = $env:PATH
$verLine = ""
try {
    $env:PATH = "$env:SystemRoot\system32;$env:SystemRoot"
    $verLine = (& $probe --version 2>&1 | Select-Object -First 1)
    $probeExit = $LASTEXITCODE
} finally {
    $env:PATH = $savedPath
}
if ($probeExit -ne 0) {
    throw ("staged btxd.exe failed to run with a system-only PATH (exit $probeExit): '$verLine'. " +
           "Exit -1073741515 = 0xC0000135 STATUS_DLL_NOT_FOUND means a dependency DLL is missing " +
           "from $SourceDir. Vendor it beside btxd.exe, or link it statically in btxd-windows.yml.")
}
Write-Host $verLine
if ($ExpectedVersion -and ($verLine -notmatch [regex]::Escape($ExpectedVersion))) {
    throw "staged btxd.exe is not ${ExpectedVersion}: reported '$verLine'"
}

# Declare which btxd this package carries, in the marker provisioning reads.
#
# WITHOUT this file, provision_node_package falls back to deriving the expected
# version from the INSTALL DIRECTORY (i.e. NODE_RELEASE_TAG) and then rejects a
# btxd that reports anything else. For a branch build those legitimately differ
# — BTX's `pr/0.33.3-network-stability` never bumped CLIENT_VERSION_BUILD, so a
# build of it reports v0.33.2 while our install tag has to move to re-provision
# — and the mismatch refuses the whole tree. That failure lands on the USER's
# machine after the ~450 MB snapshot download, not in CI, which is exactly the
# skew the workflow guards exist to prevent. The mac source-build script writes
# the same marker; see crates/btx-core/src/installer.rs (BTXD_VERSION_MARKER).
if ($verLine -match 'v[0-9]+\.[0-9]+\.[0-9]+') {
    # WriteAllText, NOT Set-Content: under pwsh 7 this writes UTF-8 with NO BOM.
    # A BOM would be fatal here — the Rust side reads the marker and compares it
    # after `.trim()`, and U+FEFF is not whitespace in Rust, so a BOM'd marker
    # would never match and would refuse the tree exactly like a missing one.
    [System.IO.File]::WriteAllText((Join-Path $Dest ".btxd-version"), $Matches[0])
    Write-Host "==> declares btxd $($Matches[0])"
} else {
    throw "could not parse a version out of btxd.exe --version: '$verLine'"
}

Write-Host "==> staged node package at $Dest"
