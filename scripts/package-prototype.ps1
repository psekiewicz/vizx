# Builds VizX in release mode and puts it, with everything it needs at run
# time, into one portable folder that runs without the development setup
# (no cargo, no environment variables):
#
#   vizx.exe            the application
#   visualizers\        bundled visualizer plugins
#   presets\            bundled MilkDrop presets
#   ffmpeg.exe          video import and export
#   projectM-4.dll      MilkDrop presets (with glew32.dll)
#   onnxruntime*.dll    stem separation
#   models\             HTDemucs, Whisper and WeSpeaker models
#   data\               pre-rendered frames and stems (portable mode)
#   Install models.cmd  downloads the models on a computer without them
#
# Usage (from the repository root, in PowerShell):
#   .\scripts\package-prototype.ps1 [-Destination D:\VizX-prototype]
#
# The tools default to where the development machine keeps them; pass other
# folders as parameters. Models are hard-linked when the destination is on
# the same drive, so they take no extra space.

param(
    [string]$Destination = 'D:\VizX-prototype',
    [string]$FfmpegDir = 'D:\tools\ffmpeg',
    [string]$ProjectmDir = 'D:\tools\projectm',
    [string]$OnnxRuntimeDir = 'D:\tools\onnxruntime-win-x64-1.32.0',
    [string]$DemucsModel = 'D:\tools\demucs\htdemucs_fp16weights.onnx',
    [string]$WhisperModel = 'D:\tools\whisper\ggml-large-v3-turbo-q5_0.bin',
    [string]$SpeakerModel = 'D:\tools\wespeaker\voxceleb_resnet34_LM.onnx',
    [string]$TargetDir = 'D:\rust\target'
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot

Write-Host 'Building VizX (release)...'
Push-Location $repository
# Cargo reports progress on its error output, which Windows PowerShell would
# otherwise treat as an error; the exit code says whether it failed.
$ErrorActionPreference = 'Continue'
try {
    cargo build --release -p vizx-app 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw 'the build failed' }
}
finally {
    Pop-Location
    $ErrorActionPreference = 'Stop'
}

New-Item -ItemType Directory -Force $Destination, "$Destination\models", "$Destination\data" | Out-Null

# Copies a file, or hard-links it when both are on the same drive (large
# models then take no extra space).
function Place-File([string]$Source, [string]$Target) {
    if (-not (Test-Path $Source)) {
        Write-Warning "missing: $Source"
        return
    }
    if (Test-Path $Target) { Remove-Item $Target -Force }
    $sameDrive = [IO.Path]::GetPathRoot($Source) -eq [IO.Path]::GetPathRoot($Target)
    if ($sameDrive -and (Get-Item $Source).Length -gt 50MB) {
        New-Item -ItemType HardLink -Path $Target -Target $Source | Out-Null
    }
    else {
        Copy-Item $Source $Target
    }
}

Place-File "$TargetDir\release\vizx.exe" "$Destination\vizx.exe"
Place-File "$FfmpegDir\bin\ffmpeg.exe" "$Destination\ffmpeg.exe"
Place-File "$ProjectmDir\bin\projectM-4.dll" "$Destination\projectM-4.dll"
Place-File "$ProjectmDir\bin\glew32.dll" "$Destination\glew32.dll"
Place-File "$OnnxRuntimeDir\lib\onnxruntime.dll" "$Destination\onnxruntime.dll"
Place-File "$OnnxRuntimeDir\lib\onnxruntime_providers_shared.dll" "$Destination\onnxruntime_providers_shared.dll"
Place-File $DemucsModel "$Destination\models\$(Split-Path -Leaf $DemucsModel)"
Place-File $WhisperModel "$Destination\models\$(Split-Path -Leaf $WhisperModel)"
Place-File $SpeakerModel "$Destination\models\$(Split-Path -Leaf $SpeakerModel)"

# The installer downloads missing models on another computer. An installer
# copied by an older version of this script is removed.
Remove-Item "$Destination\install-models.ps1" -Force -ErrorAction SilentlyContinue
Copy-Item "$PSScriptRoot\Install models.cmd" "$Destination\Install models.cmd" -Force

foreach ($folder in 'visualizers', 'presets') {
    $target = "$Destination\$folder"
    if (Test-Path $target) { Remove-Item $target -Recurse -Force }
    Copy-Item "$repository\assets\$folder" $target -Recurse
}

$size = (Get-ChildItem $Destination -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("VizX prototype is in {0} ({1:N0} MB, models hard-linked where possible)" -f $Destination, $size)
