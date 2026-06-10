Param(
    [switch]$Yes
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $PSCommandPath
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
$manifestPath = Join-Path $repoRoot "tools" "media_tools_manifest.json"
$runtimeDir = Join-Path $repoRoot "tools" "runtime"

function Log([string]$Message) {
    Write-Host "[setup] $Message"
}

function Warn([string]$Message) {
    Write-Warning "[setup] $Message"
}

function Has-Command([string]$Name) {
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

# Read a runtime version from the manifest.
function Read-ManifestVersion([string]$Tool) {
    if (-not (Test-Path $manifestPath)) {
        return $null
    }
    try {
        $manifest = Get-Content $manifestPath | ConvertFrom-Json
        return $manifest.common.$Tool.version
    } catch {
        return $null
    }
}

# Read the platform download URL from the manifest.
function Read-ManifestDownloadUrl([string]$Os) {
    if (-not (Test-Path $manifestPath)) {
        return $null
    }
    try {
        $manifest = Get-Content $manifestPath | ConvertFrom-Json
        $url = $manifest.platforms.$Os.download_url
        $baseUrl = $manifest.release_base_url
        if ($url -and $baseUrl) {
            return $url -replace "{release_base_url}", $baseUrl
        }
        return $url
    } catch {
        return $null
    }
}

# Download and extract the runtime archive for the current platform.
function Download-Runtime {
    $os = "windows"
    $url = Read-ManifestDownloadUrl $os

    if (-not $url) {
        Warn "No download URL found in manifest for ${os}."
        return
    }

    $destDir = Join-Path $runtimeDir $os
    $archivePath = Join-Path $runtimeDir "anica-runtime-${os}.tar.gz"

    if (Test-Path (Join-Path $destDir "ffmpeg") -and Test-Path (Join-Path $destDir "gstreamer")) {
        Log "Runtime already present: ${destDir}"
        return
    }

    Log "Downloading Anica runtime for ${os}..."
    Log "URL: ${url}"

    if (-not (Has-Command "curl.exe")) {
        Warn "curl.exe not found. Cannot download runtime."
        return
    }

    New-Item -ItemType Directory -Force -Path $runtimeDir | Out-Null

    try {
        curl.exe -fL --progress-bar $url -o $archivePath
    } catch {
        Warn "Download failed: ${url}"
        return
    }

    Log "Extracting runtime to ${destDir}..."
    New-Item -ItemType Directory -Force -Path $destDir | Out-Null

    # Windows 10/11 has built-in tar; use it to extract tar.gz.
    tar -xzf $archivePath -C $destDir
    Remove-Item -Path $archivePath -Force

    Log "Runtime ready: ${destDir}"
}

Log "Anica runtime bootstrap (Windows)"
Log "Manifest: ${manifestPath}"

Download-Runtime

Log "Done."
