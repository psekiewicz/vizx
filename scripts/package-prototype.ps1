# Builds VizX in release mode and puts it, with everything it needs at run
# time, into one portable folder that runs without the development setup
# (no cargo, no environment variables):
#
#   vizx.exe            the application
#   visualizers\        bundled visualizer plugins
#   presets\            bundled MilkDrop presets
#   ffmpeg.exe          video import and export            (optional)
#   projectM-4.dll      MilkDrop presets (with glew32.dll) (optional)
#   onnxruntime*.dll    runs HTDemucs and WeSpeaker        (optional)
#   models\             HTDemucs, Whisper and WeSpeaker    (optional)
#   data\               pre-rendered frames and stems (portable mode)
#   Install models.cmd  downloads the models that are missing
#
# Usage (from anywhere, in PowerShell):
#   .\scripts\package-prototype.ps1 [-Destination <folder>]
#
# Nothing is assumed about where things are on this computer. The build
# folder comes from cargo itself. FFmpeg, projectM, ONNX Runtime and the
# models are taken from the same settings VizX uses during development
# (VIZX_FFMPEG_DIR, VIZX_PROJECTM_DIR, VIZX_ONNXRUNTIME_DIR,
# VIZX_DEMUCS_MODEL, VIZX_WHISPER_MODEL, VIZX_SPEAKER_MODEL), read from the
# environment or from the [env] section of cargo's personal config.toml;
# FFmpeg is also found on the PATH. Any of them can be given as a parameter
# instead. What is not found is left out with a note: VizX runs without it,
# and "Install models.cmd" in the folder downloads the models and ONNX
# Runtime. Large models are hard-linked when the destination is on the same
# drive, so they take no extra space.

param(
    [string]$Destination = '',
    [string]$FfmpegDir = '',
    [string]$ProjectmDir = '',
    [string]$OnnxRuntimeDir = '',
    [string]$DemucsModel = '',
    [string]$WhisperModel = '',
    [string]$SpeakerModel = '',
    [string]$TargetDir = ''
)

$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
if (-not $Destination) { $Destination = Join-Path $repository 'dist\VizX' }

# --- Settings: parameter, else environment, else cargo's config.toml -------

# The [env] entries of cargo's personal configuration, as cargo run sees
# them: NAME = "value" or NAME = { value = "value", ... }.
$cargoEnv = @{}
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME '.cargo' }
foreach ($file in (Join-Path $cargoHome 'config.toml'), (Join-Path $cargoHome 'config'),
                  (Join-Path $repository '.cargo\config.toml')) {
    if (-not (Test-Path -LiteralPath $file)) { continue }
    $inEnv = $false
    foreach ($line in Get-Content -LiteralPath $file) {
        if ($line -match '^\s*\[(.+)\]\s*$') { $inEnv = ($Matches[1].Trim() -eq 'env'); continue }
        if (-not $inEnv) { continue }
        if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:\{.*?value\s*=\s*)?["'']([^"'']*)["'']') {
            $cargoEnv[$Matches[1]] = $Matches[2]
        }
    }
}

function Get-Setting([string]$given, [string]$variable) {
    if ($given) { return $given }
    $value = [Environment]::GetEnvironmentVariable($variable)
    if ($value) { return $value }
    if ($cargoEnv.ContainsKey($variable)) { return $cargoEnv[$variable] }
    return ''
}

$FfmpegDir = Get-Setting $FfmpegDir 'VIZX_FFMPEG_DIR'
$ProjectmDir = Get-Setting $ProjectmDir 'VIZX_PROJECTM_DIR'
$OnnxRuntimeDir = Get-Setting $OnnxRuntimeDir 'VIZX_ONNXRUNTIME_DIR'
$DemucsModel = Get-Setting $DemucsModel 'VIZX_DEMUCS_MODEL'
$WhisperModel = Get-Setting $WhisperModel 'VIZX_WHISPER_MODEL'
$SpeakerModel = Get-Setting $SpeakerModel 'VIZX_SPEAKER_MODEL'

# --- Build ------------------------------------------------------------------

Write-Host 'Building VizX (release)...'
Push-Location $repository
# Cargo reports progress on its error output, which Windows PowerShell would
# otherwise treat as an error; the exit code says whether it failed.
$ErrorActionPreference = 'Continue'
try {
    cargo build --release -p vizx-app 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { throw 'the build failed' }
    if (-not $TargetDir) {
        # Wherever cargo builds to: target\, CARGO_TARGET_DIR or target-dir.
        $metadata = cargo metadata --format-version 1 --no-deps 2>$null | ConvertFrom-Json
        $TargetDir = $metadata.target_directory
    }
}
finally {
    Pop-Location
    $ErrorActionPreference = 'Stop'
}

$exe = Join-Path $TargetDir 'release\vizx.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw "the build did not produce $exe" }

# --- Gather -----------------------------------------------------------------

New-Item -ItemType Directory -Force $Destination, "$Destination\models", "$Destination\data" | Out-Null
$missing = @()

# Copies a file, or hard-links it when both are on the same drive (large
# models then take no extra space). Returns whether it was there.
function Place-File([string]$Source, [string]$Target) {
    if (-not $Source -or -not (Test-Path -LiteralPath $Source -PathType Leaf)) { return $false }
    if (Test-Path -LiteralPath $Target) { Remove-Item -LiteralPath $Target -Force }
    $sameDrive = [IO.Path]::GetPathRoot($Source) -eq [IO.Path]::GetPathRoot($Target)
    if ($sameDrive -and (Get-Item -LiteralPath $Source).Length -gt 50MB) {
        New-Item -ItemType HardLink -Path $Target -Target $Source | Out-Null
    }
    else {
        Copy-Item -LiteralPath $Source $Target
    }
    return $true
}

[void](Place-File $exe "$Destination\vizx.exe")

# FFmpeg: the configured folder, else the one on the PATH.
$ffmpeg = if ($FfmpegDir) { Join-Path $FfmpegDir 'bin\ffmpeg.exe' } else { '' }
if (-not ($ffmpeg -and (Test-Path -LiteralPath $ffmpeg))) {
    $onPath = Get-Command ffmpeg.exe -ErrorAction SilentlyContinue
    $ffmpeg = if ($onPath) { $onPath.Source } else { '' }
}
if (-not (Place-File $ffmpeg "$Destination\ffmpeg.exe")) {
    $missing += 'FFmpeg (video files and export): put ffmpeg.exe next to vizx.exe'
}

$projectmDll = if ($ProjectmDir) { Join-Path $ProjectmDir 'bin\projectM-4.dll' } else { '' }
$glewDll = if ($ProjectmDir) { Join-Path $ProjectmDir 'bin\glew32.dll' } else { '' }
$hasProjectm = Place-File $projectmDll "$Destination\projectM-4.dll"
$hasGlew = Place-File $glewDll "$Destination\glew32.dll"
if (-not ($hasProjectm -and $hasGlew)) {
    $missing += 'projectM (MilkDrop presets): see docs\milkdrop.md'
}

$needsModels = $false
foreach ($dll in 'onnxruntime.dll', 'onnxruntime_providers_shared.dll') {
    $source = if ($OnnxRuntimeDir) { Join-Path $OnnxRuntimeDir "lib\$dll" } else { '' }
    if (-not (Place-File $source "$Destination\$dll")) { $needsModels = $true }
}
foreach ($model in $DemucsModel, $WhisperModel, $SpeakerModel) {
    $placed = $false
    if ($model) { $placed = Place-File $model "$Destination\models\$(Split-Path -Leaf $model)" }
    if (-not $placed) { $needsModels = $true }
}
if ($needsModels) {
    $missing += 'ONNX Runtime or models: run "Install models.cmd" in the folder'
}

# The model installer, for this computer or another one. An installer
# copied by an older version of this script is removed.
Remove-Item "$Destination\install-models.ps1" -Force -ErrorAction SilentlyContinue
Copy-Item -LiteralPath "$PSScriptRoot\Install models.cmd" "$Destination\Install models.cmd" -Force

foreach ($folder in 'visualizers', 'presets') {
    $target = "$Destination\$folder"
    if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Recurse -Force }
    Copy-Item -LiteralPath "$repository\assets\$folder" $target -Recurse
}

$size = (Get-ChildItem -LiteralPath $Destination -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ''
Write-Host ("VizX is in {0} ({1:N0} MB). Start it with vizx.exe." -f $Destination, $size)
if ($missing.Count -gt 0) {
    Write-Host 'Not included (VizX runs without them):'
    foreach ($line in $missing) { Write-Host "  - $line" }
}
