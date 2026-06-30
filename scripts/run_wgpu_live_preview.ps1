param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $PreviewArgs
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")
Set-Location $repoRoot

Get-Process -Name "wgpu_live_preview" -ErrorAction SilentlyContinue | Stop-Process -Force

cargo run --release -p motionloom --example wgpu_live_preview -- @PreviewArgs
