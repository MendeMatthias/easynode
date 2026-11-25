# Stage the bundled BTX node binaries for an easyBTX Node WINDOWS build.
#
# Takes the output of the btxd-windows.yml CI build and stages the package
# layout resolve_bundled_node_pkg() probes:
#   src-tauri\resources\node-pkg\bin\{btxd.exe, btx-cli.exe, *.dll}
# and, for BTX 0.34.16 and later, the CPython runtime btxd embeds for SwapNet:
#   ...\bin\{pythonXY.dll, python3.dll, vcruntime140*.dll, pythonXY._pth}
#   ...\bin\python\   (standard library zip, extension modules, site-packages)
# btxd.exe imports pythonXY.dll, so without that runtime it does not start at
# all, not even for --version and not with -swapnet=0. The whole artifact is
# therefore copied as a tree. provision_node_package copies the tree on to the
# install directory, and Tauri bundles resources/node-pkg/**/*.
#
# NOT a static triplet. btxd-windows.yml cross-compiles with mingw-w64, and the
# result imports at least libgomp-1.dll. 0.6.6 shipped without it: btxd.exe died
# with 0xC0000135 (STATUS_DLL_NOT_FOUND) on every machine that was not the build
# runner, the app's `--version` probe hung instead of failing, and the app fell
# back to the OLD node while reporting the new app version. Everything beside
# btxd.exe in $SourceDir is therefore vendored, and the run check below is the
# gate that proves the staged tree works with nothing but system DLLs on PATH.
#
# With -VerifyShaFile the STAGED tree is compared with the artifact's
# SHA256SUMS.txt in both directions: every staged file must be listed with the
# right hash, and every listed file must have been staged. btxd-windows.yml runs
# its Windows smoke on a tree staged by this script from the same sums, so a
# tree that passes here is, byte for byte, the one that was smoked.
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

# Lines are "<sha256-lower>  <path relative to the artifact root>" (the
# artifact's SHA256SUMS.txt, forward slashes). A line that cannot be read is an
# error, not something to skip: a skipped line is an unverified file.
$sums = @{}
if ($VerifyShaFile) {
    foreach ($line in Get-Content $VerifyShaFile) {
        if (-not $line.Trim()) { continue }
        if ($line -notmatch '^([0-9a-f]{64})\s+\*?(.+)$') {
            throw "unreadable line in ${VerifyShaFile}: '$line'"
        }
        $sums[$Matches[2].Trim()] = $Matches[1]
    }
    foreach ($name in "btxd.exe", "btx-cli.exe") {
        if (-not $sums.ContainsKey($name)) { throw "$VerifyShaFile does not list $name" }
    }
}

if (Test-Path $Dest) { Remove-Item -Recurse -Force $Dest }
$DestBin = (New-Item -ItemType Directory -Force (Join-Path $Dest "bin")).FullName

# Vendor everything the build shipped next to the binaries, as a tree. mingw
# links libgomp dynamically and 0.34.16 imports pythonXY.dll, so the package is
# only self-contained if all of it travels. The sums file itself stays behind.
$top = @(Get-ChildItem -LiteralPath $SourceDir -Force | Where-Object { $_.Name -ne "SHA256SUMS.txt" })
foreach ($item in $top) {
    Copy-Item -LiteralPath $item.FullName -Destination $DestBin -Recurse -Force
    if ($item.PSIsContainer) { Write-Host "vendored $($item.Name)\ (directory)" }
    elseif ($item.Extension -ne ".exe") { Write-Host "vendored $($item.Name)" }
}

if ($VerifyShaFile) {
    $staged = @(Get-ChildItem -LiteralPath $DestBin -Recurse -File -Force)
    $seen = @{}
    foreach ($f in $staged) {
        $rel = $f.FullName.Substring($DestBin.Length + 1).Replace('\', '/')
        if (-not $sums.ContainsKey($rel)) { throw "staged file is not listed in ${VerifyShaFile}: $rel" }
        $got = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash.ToLower()
        if ($got -ne $sums[$rel]) { throw "sha256 mismatch for ${rel}: got $got, want $($sums[$rel])" }
        $seen[$rel] = $true
    }
    foreach ($rel in $sums.Keys) {
        if (-not $seen.ContainsKey($rel)) { throw "listed in $VerifyShaFile but not staged: $rel" }
    }
    foreach ($name in "btxd.exe", "btx-cli.exe") { Write-Host "verified ${name}: $($sums[$name])" }
    Write-Host "verified all $($staged.Count) staged files against $VerifyShaFile"
}

# The embedded CPython, when the package has one. CPython reads pythonXY._pth
# beside pythonXY.dll and uses exactly the paths it lists, so each of them must
# exist in the staged tree. This is a structural check for a package staged
# WITHOUT a sums file; with one, the comparison above already covers it.
$pyDll = @(Get-ChildItem -LiteralPath $DestBin -Filter "python3*.dll" -File |
    Where-Object { $_.BaseName -match '^python3\d+$' })
if ($pyDll.Count -gt 1) { throw "more than one pythonXY.dll in the package: $($pyDll.Name -join ', ')" }
if ($pyDll.Count -eq 1) {
    $pth = Join-Path $DestBin ($pyDll[0].BaseName + "._pth")
    if (-not (Test-Path -LiteralPath $pth)) {
        throw "$($pyDll[0].Name) is staged without $(Split-Path -Leaf $pth): CPython would search the machine for a standard library"
    }
    foreach ($line in Get-Content -LiteralPath $pth) {
        $entry = $line.Trim()
        if (-not $entry -or $entry.StartsWith("#") -or $entry.StartsWith("import ")) { continue }
        if (-not (Test-Path -LiteralPath (Join-Path $DestBin $entry))) {
            throw "$(Split-Path -Leaf $pth) names '$entry', which is not in the staged tree"
        }
    }
    foreach ($need in "python3.dll", "vcruntime140.dll") {
        if (-not (Test-Path -LiteralPath (Join-Path $DestBin $need))) { throw "the embedded CPython needs $need beside btxd.exe" }
    }
    Write-Host "embedded CPython: $($pyDll[0].Name), every path in $(Split-Path -Leaf $pth) is staged"
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
    # Collect all of the output first: piping into Select-Object -First 1
    # ends the pipeline after one line and can leave $LASTEXITCODE at -1.
    $verOut = @(& $probe --version 2>&1)
    $probeExit = $LASTEXITCODE
    $verLine = "$($verOut | Select-Object -First 1)"
} finally {
    $env:PATH = $savedPath
}
if ($probeExit -ne 0) {
    throw ("staged btxd.exe failed to run with a system-only PATH (exit $probeExit): '$verLine'. " +
           "Exit -1073741515 = 0xC0000135 STATUS_DLL_NOT_FOUND means a dependency DLL is missing " +
           "from $SourceDir (a mingw runtime DLL, or from BTX 0.34.16 pythonXY.dll and vcruntime140.dll). " +
           "Vendor it beside btxd.exe, or link it statically in btxd-windows.yml.")
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
# The pattern must accept FOUR segments; a three-segment one silently
# truncates a reseal tag like v0.33.4.1 to v0.33.4 and provisioning then
# refuses a package that is exactly right. The mac scripts already do this.
if ($verLine -match 'v[0-9]+(\.[0-9]+)+') {
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
