# Builds the one-file VizX installer, VizX-Setup-<version>.exe: the setup
# program (crates/vizx-setup) with the whole application appended to it.
#
#   1. package-prototype.ps1 builds VizX (release) and gathers it with
#      everything it needs into a staging folder: tools, presets,
#      visualizers and models.
#   2. The Visual C++ runtime DLLs are added, so VizX also starts on a
#      computer that never had them installed, and the licences of the
#      bundled tools go into licenses\.
#   3. vizx-pack appends the staging folder to vizx-setup.exe.
#
# Usage (from the repository root, in PowerShell):
#   .\scripts\build-installer.ps1 [-OutputDir D:\vizx-installer]
#
# The tool folders are passed on to package-prototype.ps1; their defaults
# are where the development machine keeps them. Everything is written to
# OutputDir (keep it off a nearly full system drive: the installer is about
# 1 GB, and the staging folder hard-links the models when it is on their
# drive).

param(
    [string]$OutputDir = 'D:\vizx-installer',
    [string]$FfmpegDir = 'D:\tools\ffmpeg',
    [string]$ProjectmDir = 'D:\tools\projectm',
    [string]$ProjectmSourceDir = 'D:\tools\projectm-src',
    [string]$GlewDir = 'D:\tools\glew',
    [string]$OnnxRuntimeDir = 'D:\tools\onnxruntime-win-x64-1.32.0',
    [string]$DemucsModel = 'D:\tools\demucs\htdemucs_fp16weights.onnx',
    [string]$WhisperModel = 'D:\tools\whisper\ggml-large-v3-turbo-q5_0.bin',
    [string]$SpeakerModel = 'D:\tools\wespeaker\voxceleb_resnet34_LM.onnx',
    [string]$TargetDir = 'D:\rust\target'
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
$staging = Join-Path $OutputDir 'staging'

# The version of the workspace, for the installer's name.
$version = (Select-String -Path "$repository\Cargo.toml" -Pattern '^version = "(.+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value

if (Test-Path $staging) { Remove-Item $staging -Recurse -Force }
New-Item -ItemType Directory -Force $OutputDir | Out-Null

# 1. VizX and everything it needs.
& "$PSScriptRoot\package-prototype.ps1" -Destination $staging -FfmpegDir $FfmpegDir `
    -ProjectmDir $ProjectmDir -OnnxRuntimeDir $OnnxRuntimeDir -DemucsModel $DemucsModel `
    -WhisperModel $WhisperModel -SpeakerModel $SpeakerModel -TargetDir $TargetDir
# The installer makes the data folder itself; a submodule's link file is
# of no use outside the repository.
Remove-Item "$staging\data" -Recurse -Force -ErrorAction SilentlyContinue
Get-ChildItem $staging -Recurse -Force -Filter '.git' | Remove-Item -Recurse -Force

# 2. The Visual C++ runtime, placed next to the programs that use it (VizX,
#    projectM and ONNX Runtime). The newest copies on this computer are
#    taken, so libraries built with a newer compiler find what they need.
foreach ($name in 'msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'vcruntime140_1.dll') {
    Copy-Item "$env:WINDIR\System32\$name" "$staging\$name"
}
$licenses = New-Item -ItemType Directory -Force "$staging\licenses"
$licenseFiles = @{
    "$FfmpegDir\LICENSE"                          = 'FFmpeg.txt'
    "$ProjectmSourceDir\COPYING"                  = 'projectM.txt'
    "$GlewDir\LICENSE.txt"                        = 'GLEW.txt'
    "$OnnxRuntimeDir\LICENSE"                     = 'ONNX Runtime.txt'
    "$OnnxRuntimeDir\ThirdPartyNotices.txt"       = 'ONNX Runtime third party notices.txt'
}
foreach ($source in $licenseFiles.Keys) {
    if (Test-Path $source) {
        Copy-Item $source (Join-Path $licenses $licenseFiles[$source])
    }
    else {
        Write-Warning "missing licence: $source"
    }
}

# 3. The setup program, and the one file. It is linked with the C runtime
#    built in (crt-static): it must start on a computer without the Visual
#    C++ runtime, and as the uninstaller it must not hold the runtime DLLs
#    of the install folder open while it removes them. Its own target
#    folder keeps these differently built libraries apart from VizX's.
Write-Host 'Building the setup program (release, static C runtime)...'
$setupTarget = Join-Path $TargetDir 'setup-static'
Push-Location $repository
$ErrorActionPreference = 'Continue'
$previousFlags = $env:RUSTFLAGS
try {
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    cargo build --release -p vizx-setup --target-dir $setupTarget 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw 'the build of the setup program failed' }
}
finally {
    $env:RUSTFLAGS = $previousFlags
    Pop-Location
    $ErrorActionPreference = 'Stop'
}

$installer = Join-Path $OutputDir "VizX-Setup-$version.exe"
Write-Host "Packing $staging into $installer..."
& "$setupTarget\release\vizx-pack.exe" "$setupTarget\release\vizx-setup.exe" $staging $installer
if ($LASTEXITCODE -ne 0) { throw 'packing failed' }

$hash = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLower()
Write-Host "Installer: $installer"
Write-Host "SHA-256:   $hash"
