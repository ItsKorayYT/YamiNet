# Native MSVC bundle + real NSIS old-to-new upgrade. Use a disposable Windows profile.
param([Parameter(Mandatory=$true)][string]$Stage, [Parameter(Mandatory=$true)][string]$Version)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/../..").Path
$nsis = 'C:/Program Files (x86)/NSIS/makensis.exe'
if (-not (Test-Path $nsis)) { choco install nsis --no-progress -y; if ($LASTEXITCODE -ne 0) { throw 'NSIS installation failed' } }
$currentInstaller = Join-Path $repo "dist/YamiNet-$Version-windows-x86_64-setup.exe"
& $nsis "/DSTAGE=$Stage" "/DVERSION=$Version" "/DOUTPUT=$currentInstaller" "$PSScriptRoot/installer.nsi"
if ($LASTEXITCODE -ne 0) { throw 'Current installer compilation failed' }
$baseline = Join-Path $env:RUNNER_TEMP 'Yami baseline source'
# This immutable prior milestone exists in repository history, not a release tag.
git -C $repo worktree add --detach $baseline 760398d9cc00b84ce7dd02ce57d5af6dd0f09d63
if ($LASTEXITCODE -ne 0) { throw 'Baseline checkout failed' }
Push-Location $baseline
try {
    $env:CARGO_TARGET_DIR = Join-Path $baseline 'target'
    cargo build --workspace --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Native baseline MSVC build failed' }
    $oldStage = python packaging/package.py --platform windows --binaries "$env:CARGO_TARGET_DIR/release"
    if ($LASTEXITCODE -ne 0) { throw 'Baseline staging failed' }
    $oldInstaller = Join-Path $env:RUNNER_TEMP 'Yami baseline installer.exe'
    & $nsis "/DSTAGE=$oldStage" '/DVERSION=0.3.0-alpha.1' "/DOUTPUT=$oldInstaller" packaging/windows/installer.nsi
    if ($LASTEXITCODE -ne 0) { throw 'Baseline NSIS build failed' }
} finally { Pop-Location; Remove-Item Env:CARGO_TARGET_DIR }
$archive = Join-Path $repo "dist/YamiNet-$Version-windows-x86_64.zip"
$guiArgs = @()
if ([Environment]::UserInteractive -and (Get-Process -Id $PID).SessionId -ne 0) { $guiArgs = @('--gui') }
python "$repo/packaging/lifecycle.py" --archive $archive --installer $currentInstaller --previous-installer $oldInstaller --report "$repo/dist/windows-lifecycle.json" @guiArgs
if ($LASTEXITCODE -ne 0) { throw 'Native installer/package lifecycle failed' }
Get-ChildItem "$repo/dist/*.exe" | ForEach-Object {
    $hash = (Get-FileHash $_ -Algorithm SHA256).Hash.ToLower()
    "$hash  $($_.Name)" | Set-Content -Encoding ascii "$($_.FullName).sha256"
}
