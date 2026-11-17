# VizX model installer: a small window that downloads the machine learning
# models (and ONNX Runtime, which runs one of them) from where their authors
# publish them, checks every file against its SHA-256, and puts it where
# VizX looks for it:
#
#   <VizX folder>\models\htdemucs_fp16weights.onnx    stem separation
#   <VizX folder>\models\ggml-*.bin                     lyrics recognition
#   <VizX folder>\models\voxceleb_resnet34_LM.onnx      telling singers apart
#   <VizX folder>\onnxruntime*.dll                      runs HTDemucs
#
# Usage: double-click "Install models.cmd" next to vizx.exe, or
#   powershell -ExecutionPolicy Bypass -File install-models.ps1 [-VizxFolder <folder>]
#
# Nothing is installed system-wide and nothing needs administrator rights.
# An interrupted download continues where it stopped the next time.
# The script is plain ASCII on purpose: Windows PowerShell 5.1 reads files
# without a byte order mark in the system code page.

param([string]$VizxFolder = '')

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, System.IO.Compression.FileSystem
[System.Windows.Forms.Application]::EnableVisualStyles()
# GitHub and Hugging Face accept TLS 1.2 only; Windows PowerShell 5.1 may
# not offer it by default.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# ---------------------------------------------------------------------------
# What can be installed. Sha256 and Size are of the file as published; for
# ONNX Runtime they are of the two DLLs taken out of the release archive.
# A component without Sha256 is checked against the hash its Hugging Face
# repository publishes (the Git LFS pointer of the file).
# ---------------------------------------------------------------------------
$components = @(
    [pscustomobject]@{
        Name        = 'HTDemucs (stem separation)'
        Description = 'Splits a song into vocals, drums, bass and other, for visuals that follow ' +
                      'one instrument, and for lyrics recognition. Model by Meta (MIT licence), ' +
                      'converted to ONNX by StemSplitio. Needs ONNX Runtime.'
        Source      = 'huggingface.co/StemSplitio/htdemucs-onnx'
        Kind        = 'File'
        Url         = 'https://huggingface.co/StemSplitio/htdemucs-onnx/resolve/main/htdemucs_fp16weights.onnx'
        Target      = 'models\htdemucs_fp16weights.onnx'
        Sha256      = 'd05c269d0178d2a72ad484b10b11dd370193fc923201c3b27a99f848745db70a'
        Size        = 165612636
        Default     = $true
    }
    [pscustomobject]@{
        Name        = 'ONNX Runtime 1.32.0'
        Description = 'The library that runs the HTDemucs model (Microsoft, MIT licence). Only ' +
                      'onnxruntime.dll and onnxruntime_providers_shared.dll are taken out of the ' +
                      'release archive (92 MB to download, 21 MB installed).'
        Source      = 'github.com/microsoft/onnxruntime/releases'
        Kind        = 'OnnxRuntime'
        Url         = 'https://github.com/microsoft/onnxruntime/releases/download/v1.32.0/onnxruntime-win-x64-1.32.0.zip'
        Target      = 'onnxruntime.dll'
        Sha256      = '348c3d29d6995eee37e3451825eaddb7ea7400be3afda6ca2286b12bc775f042'
        Size        = 20602168
        Download    = 96945131
        Extra       = @{
            'onnxruntime_providers_shared.dll' = '6caf57ac07c9cf72fb4e73c5bb0fe156f02dd60bbbd7385530c9a5654f10f950'
        }
        Default     = $true
    }
    [pscustomobject]@{
        Name        = 'Whisper large-v3-turbo q5_0 (lyrics, recommended)'
        Description = 'Writes down the sung words with their times. OpenAI Whisper (MIT licence) ' +
                      'in the whisper.cpp format. A 4-minute song takes about 14 minutes on a ' +
                      '4-core laptop.'
        Source      = 'huggingface.co/ggerganov/whisper.cpp'
        Kind        = 'File'
        Url         = 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin'
        Target      = 'models\ggml-large-v3-turbo-q5_0.bin'
        Sha256      = '394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2'
        Size        = 574041195
        Default     = $true
    }
    [pscustomobject]@{
        Name        = 'WeSpeaker ResNet34 (tell singers apart)'
        Description = 'Voice prints for Separate Singers in the Subtitle Editor: every singer gets ' +
                      'a subtitle layer of their own. By the WeSpeaker project, trained on ' +
                      'VoxCeleb (CC BY 4.0 licence). Needs ONNX Runtime.'
        Source      = 'huggingface.co/Wespeaker'
        Kind        = 'File'
        Url         = 'https://huggingface.co/Wespeaker/wespeaker-voxceleb-resnet34-LM/resolve/main/voxceleb_resnet34_LM.onnx'
        Target      = 'models\voxceleb_resnet34_LM.onnx'
        Sha256      = '7bb2f06e9df17cdf1ef14ee8a15ab08ed28e8d0ef5054ee135741560df2ec068'
        Size        = 26530309
        Default     = $true
    }
    [pscustomobject]@{
        Name        = 'Whisper small (lyrics, faster, less accurate)'
        Description = 'A much smaller Whisper model for slow computers: several times faster, but ' +
                      'it misses more sung words. VizX uses large-v3-turbo instead when both are ' +
                      'installed.'
        Source      = 'huggingface.co/ggerganov/whisper.cpp'
        Kind        = 'File'
        Url         = 'https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin'
        Pointer     = 'https://huggingface.co/ggerganov/whisper.cpp/raw/main/ggml-small.bin'
        Target      = 'models\ggml-small.bin'
        Sha256      = $null
        Size        = 0
        Default     = $false
    }
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

function Format-Size([double]$bytes) {
    if ($bytes -ge 1GB) { return '{0:N1} GB' -f ($bytes / 1GB) }
    if ($bytes -ge 1MB) { return '{0:N0} MB' -f ($bytes / 1MB) }
    return '{0:N0} KB' -f [math]::Ceiling($bytes / 1KB)
}

# How much is downloaded (and briefly stored) for a component.
function Get-DownloadSize($component) {
    if ($component.Download) { return [long]$component.Download }
    if ($component.Size -gt 0) { return [long]$component.Size }
    return 466MB # about the size of Whisper small; its exact size is published with it
}

function Get-Sha256([string]$path) {
    (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Test-Installed($component, [string]$folder) {
    if (-not $folder) { return $false }
    $path = Join-Path $folder $component.Target
    if (-not (Test-Path -LiteralPath $path)) { return $false }
    # The size is enough to tell at a glance; the hash was checked when the
    # file was installed.
    return ($component.Size -eq 0) -or ((Get-Item -LiteralPath $path).Length -eq $component.Size)
}

# The SHA-256 and size a Hugging Face repository publishes for a large file
# (its Git LFS pointer, a few lines of text).
function Get-LfsPointer([string]$url) {
    $request = [Net.HttpWebRequest]::Create($url)
    $request.UserAgent = 'VizX-model-installer'
    $response = $request.GetResponse()
    try {
        $reader = New-Object IO.StreamReader($response.GetResponseStream())
        $buffer = New-Object char[] 2048
        $text = New-Object string($buffer, 0, $reader.Read($buffer, 0, $buffer.Length))
    }
    finally { $response.Close() }
    if ($text -notmatch 'oid sha256:([0-9a-f]{64})' ) { throw "no checksum published at $url" }
    $hash = $Matches[1]
    $size = 0
    if ($text -match 'size (\d+)') { $size = [long]$Matches[1] }
    return @{ Sha256 = $hash; Size = $size }
}

$script:cancelRequested = $false

# Downloads $url to "$target.part", continuing a part left from before, and
# reports progress in the window. Returns the path of the finished part.
function Receive-File([string]$url, [string]$target, [string]$label) {
    $part = "$target.part"
    $offset = 0L
    if (Test-Path -LiteralPath $part) { $offset = (Get-Item -LiteralPath $part).Length }
    $request = [Net.HttpWebRequest]::Create($url)
    $request.UserAgent = 'VizX-model-installer'
    $request.Timeout = 30000
    $request.ReadWriteTimeout = 60000
    if ($offset -gt 0) { $request.AddRange([long]$offset) }
    try {
        $response = $request.GetResponse()
    }
    catch [Net.WebException] {
        # 416: the part is already complete (or the server changed); start
        # over once.
        if ($offset -gt 0) {
            Remove-Item -LiteralPath $part -Force
            return Receive-File $url $target $label
        }
        throw
    }
    $inStream = $null
    $outStream = $null
    try {
        $continues = ($offset -gt 0) -and ([int]$response.StatusCode -eq 206)
        if (-not $continues) { $offset = 0L }
        $total = $offset + $response.ContentLength
        $mode = if ($continues) { [IO.FileMode]::Append } else { [IO.FileMode]::Create }
        $outStream = New-Object IO.FileStream($part, $mode, [IO.FileAccess]::Write)
        $inStream = $response.GetResponseStream()
        $buffer = New-Object byte[] (1MB)
        $done = $offset
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $lastShown = -1000
        while (($read = $inStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $outStream.Write($buffer, 0, $read)
            $done += $read
            if ($clock.ElapsedMilliseconds - $lastShown -ge 200) {
                $lastShown = $clock.ElapsedMilliseconds
                $speed = ($done - $offset) / [math]::Max(0.001, $clock.Elapsed.TotalSeconds)
                $text = '{0}: {1} of {2}  ({3}/s)' -f $label, (Format-Size $done), (Format-Size $total), (Format-Size $speed)
                if ($speed -gt 0 -and $total -gt $done) {
                    $left = [TimeSpan]::FromSeconds(($total - $done) / $speed)
                    $text += ', {0:hh\:mm\:ss} left' -f $left
                }
                Show-Progress $text ([int](1000 * $done / [math]::Max(1, $total)))
                [System.Windows.Forms.Application]::DoEvents()
                if ($script:cancelRequested) { throw 'Stopped. The download continues from here next time.' }
            }
        }
    }
    finally {
        if ($inStream) { $inStream.Close() }
        if ($outStream) { $outStream.Close() }
        $response.Close()
    }
    return $part
}

function Assert-Hash([string]$path, [string]$expected, [string]$label) {
    Show-Progress "${label}: checking the file..." -1
    [System.Windows.Forms.Application]::DoEvents()
    $actual = Get-Sha256 $path
    if ($actual -ne $expected) {
        Remove-Item -LiteralPath $path -Force
        throw "$label is damaged or not the expected file (SHA-256 $actual, expected $expected). It was deleted; try again."
    }
}

function Install-Component($component, [string]$folder) {
    $label = $component.Name
    $target = Join-Path $folder $component.Target
    New-Item -ItemType Directory -Force (Split-Path -Parent $target) | Out-Null

    if ($component.Kind -eq 'OnnxRuntime') {
        $archive = Join-Path $folder 'onnxruntime-download.zip'
        $part = Receive-File $component.Url $archive $label
        Move-Item -LiteralPath $part $archive -Force
        Show-Progress "${label}: unpacking..." -1
        [System.Windows.Forms.Application]::DoEvents()
        $wanted = @{ 'onnxruntime.dll' = $component.Sha256 } + $component.Extra
        $zip = [IO.Compression.ZipFile]::OpenRead($archive)
        try {
            foreach ($name in $wanted.Keys) {
                $entry = $zip.Entries | Where-Object { $_.FullName -match "[/\\]lib[/\\]$([regex]::Escape($name))$" } | Select-Object -First 1
                if (-not $entry) { throw "$name is not in the ONNX Runtime archive" }
                $unpacked = Join-Path $folder "$name.part"
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $unpacked, $true)
                Assert-Hash $unpacked $wanted[$name] $name
                Move-Item -LiteralPath $unpacked (Join-Path $folder $name) -Force
            }
        }
        finally { $zip.Dispose() }
        Remove-Item -LiteralPath $archive -Force
        return
    }

    $expected = $component.Sha256
    if (-not $expected) {
        Show-Progress "${label}: reading the published checksum..." -1
        [System.Windows.Forms.Application]::DoEvents()
        $expected = (Get-LfsPointer $component.Pointer).Sha256
    }
    $part = Receive-File $component.Url $target $label
    Assert-Hash $part $expected $label
    Move-Item -LiteralPath $part $target -Force
}

# ---------------------------------------------------------------------------
# The window
# ---------------------------------------------------------------------------

if (-not $VizxFolder) {
    # Next to vizx.exe when the installer ships with VizX.
    $VizxFolder = if (Test-Path (Join-Path $PSScriptRoot 'vizx.exe')) { $PSScriptRoot } else { '' }
}

$form = New-Object System.Windows.Forms.Form
$form.Text = 'VizX - Install Models'
$form.Size = New-Object Drawing.Size(760, 560)
$form.MinimumSize = New-Object Drawing.Size(640, 480)
$form.StartPosition = 'CenterScreen'
$form.Font = New-Object Drawing.Font('Segoe UI', 9)

$intro = New-Object Windows.Forms.Label
$intro.Text = 'Downloads the machine learning models VizX uses, from where their authors publish them. ' +
              'Every file is checked (SHA-256) before it is used. Everything runs on this computer.'
$intro.SetBounds(12, 10, 720, 36)
$intro.Anchor = 'Top, Left, Right'
$form.Controls.Add($intro)

$folderLabel = New-Object Windows.Forms.Label
$folderLabel.Text = 'VizX folder:'
$folderLabel.SetBounds(12, 52, 80, 20)
$form.Controls.Add($folderLabel)

$folderBox = New-Object Windows.Forms.TextBox
$folderBox.Text = $VizxFolder
$folderBox.SetBounds(92, 49, 540, 24)
$folderBox.Anchor = 'Top, Left, Right'
$form.Controls.Add($folderBox)

$browse = New-Object Windows.Forms.Button
$browse.Text = 'Browse...'
$browse.SetBounds(640, 48, 92, 26)
$browse.Anchor = 'Top, Right'
$form.Controls.Add($browse)

$list = New-Object Windows.Forms.ListView
$list.View = 'Details'
$list.CheckBoxes = $true
$list.FullRowSelect = $true
$list.HideSelection = $false
$list.SetBounds(12, 82, 720, 170)
$list.Anchor = 'Top, Left, Right'
[void]$list.Columns.Add('Component', 330)
[void]$list.Columns.Add('Size', 80)
[void]$list.Columns.Add('Status', 100)
[void]$list.Columns.Add('Source', 200)
$form.Controls.Add($list)

$details = New-Object Windows.Forms.Label
$details.SetBounds(12, 258, 720, 54)
$details.Anchor = 'Top, Left, Right'
$form.Controls.Add($details)

$spaceLabel = New-Object Windows.Forms.Label
$spaceLabel.SetBounds(12, 316, 720, 20)
$spaceLabel.Anchor = 'Top, Left, Right'
$form.Controls.Add($spaceLabel)

$progress = New-Object Windows.Forms.ProgressBar
$progress.Maximum = 1000
$progress.SetBounds(12, 344, 720, 22)
$progress.Anchor = 'Top, Left, Right'
$form.Controls.Add($progress)

$statusLabel = New-Object Windows.Forms.Label
$statusLabel.SetBounds(12, 372, 720, 40)
$statusLabel.Anchor = 'Top, Left, Right'
$form.Controls.Add($statusLabel)

$installButton = New-Object Windows.Forms.Button
$installButton.Text = 'Install'
$installButton.SetBounds(440, 470, 92, 30)
$installButton.Anchor = 'Bottom, Right'
$form.Controls.Add($installButton)
$form.AcceptButton = $installButton

$stopButton = New-Object Windows.Forms.Button
$stopButton.Text = 'Stop'
$stopButton.Enabled = $false
$stopButton.SetBounds(540, 470, 92, 30)
$stopButton.Anchor = 'Bottom, Right'
$form.Controls.Add($stopButton)

$closeButton = New-Object Windows.Forms.Button
$closeButton.Text = 'Close'
$closeButton.SetBounds(640, 470, 92, 30)
$closeButton.Anchor = 'Bottom, Right'
$form.Controls.Add($closeButton)

function Show-Progress([string]$text, [int]$thousandths) {
    $statusLabel.Text = $text
    if ($thousandths -lt 0) {
        $progress.Style = 'Marquee'
    }
    else {
        $progress.Style = 'Continuous'
        $progress.Value = [math]::Min(1000, [math]::Max(0, $thousandths))
    }
}

$script:busy = $false

function Update-List {
    $folder = $folderBox.Text.Trim()
    $list.BeginUpdate()
    $list.Items.Clear()
    foreach ($component in $components) {
        $installed = Test-Installed $component $folder
        $item = New-Object Windows.Forms.ListViewItem($component.Name)
        $size = Format-Size (Get-DownloadSize $component)
        [void]$item.SubItems.Add($size)
        [void]$item.SubItems.Add($(if ($installed) { 'Installed' } else { 'Not installed' }))
        [void]$item.SubItems.Add($component.Source)
        $item.Checked = (-not $installed) -and $component.Default
        $item.Tag = $component
        [void]$list.Items.Add($item)
    }
    $list.EndUpdate()
    Update-Space
}

function Update-Space {
    $folder = $folderBox.Text.Trim()
    $needed = 0L
    foreach ($item in $list.Items) {
        if ($item.Checked) {
            $needed += Get-DownloadSize $item.Tag
        }
    }
    $text = 'Selected: ' + (Format-Size $needed)
    try {
        $root = [IO.Path]::GetPathRoot($folder)
        if ($root) {
            $free = (New-Object IO.DriveInfo($root)).AvailableFreeSpace
            $text += "    Free on ${root}: " + (Format-Size $free)
            if ($free -lt $needed) { $text += '    - not enough space!' }
        }
    }
    catch { }
    $spaceLabel.Text = $text
}

$list.Add_ItemChecked({ if (-not $script:busy) { Update-Space } })
$list.Add_SelectedIndexChanged({
    if ($list.SelectedItems.Count -gt 0) { $details.Text = $list.SelectedItems[0].Tag.Description }
})
$folderBox.Add_Leave({ Update-List })
$browse.Add_Click({
    $dialog = New-Object Windows.Forms.FolderBrowserDialog
    $dialog.Description = 'Choose the folder with vizx.exe'
    if ($folderBox.Text -and (Test-Path $folderBox.Text)) { $dialog.SelectedPath = $folderBox.Text }
    if ($dialog.ShowDialog($form) -eq 'OK') {
        $folderBox.Text = $dialog.SelectedPath
        Update-List
    }
})
$stopButton.Add_Click({ $script:cancelRequested = $true })
$closeButton.Add_Click({ $form.Close() })
$form.Add_FormClosing({
    param($sender, $event)
    if ($script:busy) {
        # Stop first; the window closes once the download has let go of
        # its file.
        $script:cancelRequested = $true
        $script:closeAfterStop = $true
        $event.Cancel = $true
    }
})

$installButton.Add_Click({
    $folder = $folderBox.Text.Trim()
    if (-not $folder -or -not (Test-Path -LiteralPath $folder -PathType Container)) {
        [void][Windows.Forms.MessageBox]::Show($form, 'Choose the VizX folder first (the one with vizx.exe).', 'VizX')
        return
    }
    if (-not (Test-Path -LiteralPath (Join-Path $folder 'vizx.exe'))) {
        $answer = [Windows.Forms.MessageBox]::Show($form,
            "There is no vizx.exe in $folder. VizX only finds the models next to its executable. Install here anyway?",
            'VizX', 'YesNo', 'Warning')
        if ($answer -ne 'Yes') { return }
    }
    $chosen = @($list.Items | Where-Object { $_.Checked })
    if ($chosen.Count -eq 0) {
        Show-Progress 'Nothing selected.' 0
        return
    }

    $script:busy = $true
    $script:cancelRequested = $false
    $script:closeAfterStop = $false
    $installButton.Enabled = $false
    $browse.Enabled = $false
    $folderBox.Enabled = $false
    $list.Enabled = $false
    $stopButton.Enabled = $true
    $failures = @()
    $installed = 0
    foreach ($item in $chosen) {
        if ($script:cancelRequested) { break }
        $item.SubItems[2].Text = 'Downloading...'
        try {
            Install-Component $item.Tag $folder
            $item.SubItems[2].Text = 'Installed'
            $item.Checked = $false
            $installed++
        }
        catch {
            $message = $_.Exception.Message
            $item.SubItems[2].Text = if ($script:cancelRequested) { 'Stopped' } else { 'Failed' }
            $failures += "$($item.Tag.Name): $message"
        }
    }
    $script:busy = $false
    $installButton.Enabled = $true
    $browse.Enabled = $true
    $folderBox.Enabled = $true
    $list.Enabled = $true
    $stopButton.Enabled = $false
    Update-Space
    if ($failures.Count -eq 0) {
        Show-Progress "Done: $installed installed. Restart VizX if it is open." 1000
    }
    else {
        Show-Progress ($failures -join "`n") 0
    }
    if ($script:closeAfterStop) { $form.Close() }
})

$form.Add_Shown({ Update-List })

try {
    [void]$form.ShowDialog()
}
catch {
    [void][Windows.Forms.MessageBox]::Show("The installer stopped: $($_.Exception.Message)", 'VizX', 'OK', 'Error')
}
