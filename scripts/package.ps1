# Build and package the way the Release workflow does, on this machine.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1 -OutDir target\package
#
# The steps, the flags and the checks are the ones in
# .github/workflows/release.yml, in that order, so what comes out of this script
# is the artifact that comes out of the runner. The one difference is the version:
# the workflow reads it from the tag it was triggered by, and here there is no
# tag, so it reads it from Cargo.toml.
#
# What makes it a release build rather than a `cargo build --release` is
# RUSTFLAGS=-C target-feature=+crt-static: without it the exe imports
# VCRUNTIME140.dll, which is not part of Windows, and the README's promise of
# "no runtime needed" is false. Because the flag changes every fingerprint, the
# first run rebuilds the whole workspace.
#
# The artifact is a folder with exactly three files - ShaiZero.exe, LICENSE,
# README.md - zipped, plus the SHA256 of the zip. All three land in -OutDir,
# which defaults to the repository root, and all three are gitignored there.
param(
  [string]$OutDir = (Join-Path $PSScriptRoot "..")
)

$ErrorActionPreference = "Stop"
$LASTEXITCODE = 0
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repo

function Invoke-Step([string]$name, [scriptblock]$body) {
  Write-Host "== $name =="
  & $body
  if ($LASTEXITCODE -ne 0) { throw "FAIL: $name exited $LASTEXITCODE" }
}

# The stage folder is replaced, not merged into, and a folder somebody is running
# from cannot be replaced - Windows lets the delete get as far as the LICENSE and
# the README and then stops on the locked exe, which leaves the running program
# without the two files next to it. Asking that exe for exclusive write access is
# how a script notices it is in use, and it is asked before anything is deleted.
function Assert-NotRunning([string]$path) {
  if (-not (Test-Path $path)) { return }
  $stream = $null
  try {
    $stream = [System.IO.File]::Open($path, 'Open', 'ReadWrite', 'None')
  } catch {
    throw "FAIL: $path is in use. Quit the ShaiZero that runs from it, or point -OutDir somewhere else."
  } finally {
    if ($stream) { $stream.Dispose() }
  }
}

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
foreach ($needed in @("LICENSE", "README.md", "assets\shaiwin.ico")) {
  # Checked before the build so a missing file costs seconds, not four minutes.
  if (-not (Test-Path $needed)) { throw "FAIL: $needed is missing, and a release cannot be made without it" }
}
Write-Host "version $version (Cargo.toml)"

# Everything after this line is the workflow, step by step.
$env:RUSTFLAGS = "-C target-feature=+crt-static"

Invoke-Step "toolchain" {
  rustc --version
  cargo --version
  node --version
}
Invoke-Step "test" { cargo test --workspace }
Invoke-Step "clippy" { cargo clippy --workspace --all-targets -- -D warnings }
Invoke-Step "build" { cargo build -p shaiwin --release }
Invoke-Step "the exe wears the icon" {
  node scripts/check-exe-icon.mjs target/release/ShaiZero.exe assets/shaiwin.ico
}
Invoke-Step "the icon group table" {
  node scripts/check-exe-group.mjs target/release/ShaiZero.exe assets/shaiwin.ico
}

Invoke-Step "the exe carries its own runtime" {
  $text = [System.Text.Encoding]::ASCII.GetString(
    [System.IO.File]::ReadAllBytes("target/release/ShaiZero.exe"))
  if ($text.Contains("VCRUNTIME140.dll")) {
    throw "the exe imports VCRUNTIME140.dll, so RUSTFLAGS did not reach the build"
  }
  "no VC++ runtime import"
}

# The default is already an absolute path (next to the script), and a caller may
# pass either kind, so a relative -OutDir is read against the repository root.
$out = if ([System.IO.Path]::IsPathRooted($OutDir)) { $OutDir } else { Join-Path $repo $OutDir }
if (-not (Test-Path $out)) { New-Item -ItemType Directory -Force -Path $out | Out-Null }
$out = (Resolve-Path $out).Path
$stage = Join-Path $out "shaizero-$version-win64"
$zip = Join-Path $out "shaizero-$version-win64.zip"
$sums = Join-Path $out "SHA256SUMS.txt"
Assert-NotRunning (Join-Path $stage "ShaiZero.exe")
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
if (Test-Path $zip) { Remove-Item $zip -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

# Three files, and the check below is what keeps it that way: a fourth one
# creeping in is how an artifact starts to differ from the published one.
Copy-Item target/release/ShaiZero.exe $stage/
Copy-Item LICENSE $stage/
Copy-Item README.md $stage/
$packed = @(Get-ChildItem $stage -File)
$want = @("README.md", "LICENSE", "ShaiZero.exe")
$got = @($packed | ForEach-Object { $_.Name } | Sort-Object)
if (Compare-Object $want $got) {
  throw "FAIL: the artifact holds $($got -join ', ') instead of $($want -join ', ')"
}

Compress-Archive -Path "$stage/*" -DestinationPath $zip -CompressionLevel Optimal
$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
"$hash  $(Split-Path $zip -Leaf)" | Out-File -FilePath $sums -Encoding utf8 -NoNewline

Write-Host "== packaged =="
Get-ChildItem $stage -File | Select-Object Name, Length | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
Get-Item $zip | Select-Object Name, Length | Format-Table -AutoSize | Out-String -Width 200 | Write-Host
Write-Host (Get-Content $sums -Raw)
