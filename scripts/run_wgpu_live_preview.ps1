param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $PreviewArgs
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
Set-Location $repoRoot

Get-Process -Name "wgpu_live_preview" -ErrorAction SilentlyContinue | Stop-Process -Force

cargo build --release -p motionloom --example wgpu_live_preview
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$previewExe = Join-Path $repoRoot "target\release\examples\wgpu_live_preview.exe"
if (-not (Test-Path $previewExe)) {
    Write-Error "wgpu_live_preview.exe was not built at $previewExe"
    exit 1
}

$resolvedArgs = @()
foreach ($arg in $PreviewArgs) {
    if ($arg -and -not $arg.StartsWith("-") -and $arg.EndsWith(".motionloom")) {
        if (-not (Test-Path $arg)) {
            $normalized = $arg -replace "\\", "/"
            $marker = "motionloom-example/"
            $markerIndex = $normalized.IndexOf($marker, [System.StringComparison]::OrdinalIgnoreCase)
            if ($markerIndex -ge 0) {
                $relative = $normalized.Substring($markerIndex + $marker.Length).TrimStart("/")
                $rawUrl = "https://raw.githubusercontent.com/LOVELYZOMBIEYHO/motionloom-example/refs/heads/main/$relative"
                Write-Host "[wgpu-preview] Local motionloom-example file not found; using GitHub raw URL: $rawUrl" -ForegroundColor Yellow
                $resolvedArgs += $rawUrl
                continue
            } else {
                Write-Error "MotionLoom file not found: $arg. Current directory is $repoRoot. Make sure motionloom-example is cloned next to anica, pass an absolute path, or pass a GitHub raw URL."
                exit 1
            }
        }
        $resolvedArgs += (Resolve-Path $arg).Path
    } else {
        $resolvedArgs += $arg
    }
}

& $previewExe @resolvedArgs
exit $LASTEXITCODE
