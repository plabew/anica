Param(
    [switch]$Yes,
    [switch]$SyncOnly,
    [string]$Mode = "local-lgpl",
    [string]$ToolsHome
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $PSCommandPath
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
$toolsDir = Join-Path $repoRoot "tools"
$manifestPath = Join-Path $toolsDir "media_tools_manifest.json"
$runtimeDir = if ($ToolsHome) { $ToolsHome } else { Join-Path $toolsDir "runtime" }

function Log([string]$Message) { Write-Host "[setup] $Message" }
function Warn([string]$Message) { Write-Warning "[setup] $Message" }

function Read-ManifestValue([string]$Path) {
    if (-not (Test-Path $manifestPath)) { return $null }
    $manifest = Get-Content $manifestPath | ConvertFrom-Json
    $node = $manifest
    foreach ($part in $Path.Split('.')) {
        if (-not $node.PSObject.Properties[$part]) { return $null }
        $node = $node.$part
    }
    return $node
}

function Read-PlatformManifestValue([string]$Platform, [string]$Name, [string]$FallbackPath) {
    $platformValue = Read-ManifestValue "platforms.$Platform.$Name"
    if ($platformValue) { return $platformValue }
    return Read-ManifestValue $FallbackPath
}

function Copy-RuntimeTree([string]$Source, [string]$Destination) {
    $parent = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
    if (Test-Path $Destination) { Remove-Item -Recurse -Force -Path $Destination }
    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    Copy-Item -Recurse -Force -Path (Join-Path $Source "*") -Destination $Destination
}

function Resolve-ToolTree([string]$Root, [string]$Binary, [string]$Version) {
    $direct = Join-Path $Root "bin\$Binary"
    if (Test-Path $direct) { return $Root }
    if ($Version) {
        $versionedRoot = Join-Path $Root $Version
        if (Test-Path (Join-Path $versionedRoot "bin\$Binary")) { return $versionedRoot }
    }
    if (Test-Path $Root) {
        $child = Get-ChildItem -Path $Root -Directory | Sort-Object Name | Where-Object { Test-Path (Join-Path $_.FullName "bin\$Binary") } | Select-Object -First 1
        if ($child) { return $child.FullName }
    }
    throw "Could not resolve runtime tool tree at ${Root} for ${Binary}."
}

function Find-RuntimeRootInStaging([string]$Staging) {
    if (Test-Path (Join-Path $Staging "ffmpeg")) { return $Staging }
    $subdir = Get-ChildItem -Path $Staging -Directory -Recurse -Depth 1 | Where-Object { $_.Name -eq "ffmpeg" } | Select-Object -First 1
    if ($subdir) { return Split-Path -Parent $subdir.FullName }
    return $null
}

function Expand-RuntimeArchive([string]$Archive, [string]$Destination) {
    if ($Archive.EndsWith(".tar.gz") -or $Archive.EndsWith(".tgz")) {
        tar -xzf $Archive -C $Destination
        if ($LASTEXITCODE -ne 0) { throw "Failed to extract tar.gz runtime archive." }
        return
    }
    Expand-Archive -Path $Archive -DestinationPath $Destination -Force
}

function Download-Runtime {
    $os = "windows"
    $platformDir = Join-Path $runtimeDir $os
    $currentDir = Join-Path (Join-Path $runtimeDir "current") $os
    $ffmpegVersion = Read-PlatformManifestValue $os "ffmpeg_version" "common.ffmpeg.version"
    if (-not $ffmpegVersion) { Warn "No FFmpeg version in manifest."; return }

    if ($SyncOnly) {
        $src = Join-Path $platformDir "ffmpeg\$ffmpegVersion"
        if (-not (Test-Path $src)) { throw "Versioned FFmpeg runtime not found." }
        Copy-RuntimeTree (Resolve-ToolTree $src "ffmpeg.exe" $ffmpegVersion) (Join-Path $currentDir "ffmpeg")
        Log "Runtime ready: $currentDir"
        return
    }

    if (Test-Path (Join-Path $currentDir "ffmpeg\bin\ffmpeg.exe")) {
        Log "Runtime already present at $currentDir"
        return
    }
    $versioned = Join-Path $platformDir "ffmpeg\$ffmpegVersion"
    if (Test-Path $versioned) {
        Copy-RuntimeTree (Resolve-ToolTree $versioned "ffmpeg.exe" $ffmpegVersion) (Join-Path $currentDir "ffmpeg")
        Log "Runtime ready: $currentDir"
        return
    }

    $url = Read-ManifestValue "platforms.windows.download_url"
    $baseUrl = Read-ManifestValue "release_base_url"
    if (-not $url) { throw "No download URL for windows." }
    $url = $url -replace "\{release_base_url\}", $baseUrl
    $tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("anica-runtime-" + [guid]::NewGuid())
    $archive = Join-Path $tmp "runtime.tar.gz"
    $staging = Join-Path $tmp "staging"
    New-Item -ItemType Directory -Force -Path $staging | Out-Null
    Log "Downloading $url"
    Invoke-WebRequest -Uri $url -OutFile $archive
    Expand-RuntimeArchive $archive $staging
    $runtimeRoot = Find-RuntimeRootInStaging $staging
    if (-not $runtimeRoot) { throw "Could not find FFmpeg inside extracted archive." }
    New-Item -ItemType Directory -Force -Path (Join-Path $platformDir "ffmpeg") | Out-Null
    Copy-RuntimeTree (Resolve-ToolTree (Join-Path $runtimeRoot "ffmpeg") "ffmpeg.exe" $ffmpegVersion) (Join-Path $platformDir "ffmpeg\$ffmpegVersion")
    Copy-RuntimeTree (Join-Path $platformDir "ffmpeg\$ffmpegVersion") (Join-Path $currentDir "ffmpeg")
    Remove-Item -Recurse -Force -Path $tmp
    Log "Runtime ready: $currentDir"
}

Download-Runtime
