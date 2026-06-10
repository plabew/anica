# =========================================
# =========================================
# scripts/cargo_run_with_acp_check.ps1
#
# Windows cargo runner: auto-build anica-acp if missing or stale,
# then launch the main binary.
#
# Usage: cargo run passes <compiled-binary> [args...] to this script.

param(
    [Parameter(Position = 0, Mandatory = $true)]
    [string]$ExePath,

    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$Args
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Resolve script directory and repo root.
$scriptDir = Split-Path -Parent $PSCommandPath
$repoRoot = Resolve-Path (Join-Path $scriptDir "..")

# Determine profile (debug or release) from the executable path.
$profileDir = "debug"
if ($ExePath -match "\\release\\") {
    $profileDir = "release"
}

# Resolve bin directory and target directory.
$exeDir = Split-Path -Parent $ExePath
$exeDirName = Split-Path -Leaf $exeDir
if ($exeDirName -eq $profileDir) {
    $binDir = $exeDir
    $targetDir = Split-Path -Parent $binDir
} else {
    $targetDir = Join-Path $repoRoot "target"
    $binDir = Join-Path $targetDir $profileDir
}

# Only auto-build when launching the main app binary (not anica-acp itself).
$exeName = Split-Path -Leaf $ExePath
if ($exeName -eq "anica.exe" -and $env:ANICA_ACP_AUTO_BUILD -ne "0") {
    $acpBin = Join-Path $binDir "anica-acp.exe"
    $needsBuild = $false

    # Check if anica-acp source exists in standard cargo location (src/bin/)
    $acpSourceFile = Join-Path $repoRoot "src\bin\anica-acp.rs"
    $acpSourceDir = Join-Path $repoRoot "src\bin\anica-acp"
    $acpIsCargoTarget = (Test-Path $acpSourceFile) -or (Test-Path $acpSourceDir)

    if (-not $acpIsCargoTarget) {
        # anica-acp is not a cargo target in this codebase, skip auto-build
        Write-Host "[anica-runner] anica-acp is not a cargo target (src/bin/anica-acp.rs not found), skipping auto-build." -ForegroundColor DarkGray
    } elseif (-not (Test-Path $acpBin)) {
        $needsBuild = $true
    } else {
        # Check if any tracked source files are newer than the binary.
        $acpBinTime = (Get-Item $acpBin).LastWriteTime

        $trackedSources = @(
            (Join-Path $repoRoot "Cargo.toml"),
            (Join-Path $repoRoot "Cargo.lock"),
            $acpSourceFile
        )

        # Support future split modules under src/bin/anica-acp/
        if (Test-Path $acpSourceDir) {
            $trackedSources += Get-ChildItem $acpSourceDir -Recurse -Filter "*.rs" | ForEach-Object { $_.FullName }
        }

        foreach ($src in $trackedSources) {
            if (Test-Path $src) {
                $srcTime = (Get-Item $src).LastWriteTime
                if ($srcTime -gt $acpBinTime) {
                    $needsBuild = $true
                    break
                }
            }
        }
    }

    if ($needsBuild) {
        Write-Host "[anica-runner] anica-acp stale/missing; rebuilding..." -ForegroundColor Yellow

        $cargoArgs = @(
            "build",
            "--bin", "anica-acp",
            "--manifest-path", (Join-Path $repoRoot "Cargo.toml"),
            "--target-dir", $targetDir
        )
        if ($profileDir -eq "release") {
            $cargoArgs += "--release"
        }

        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) {
            Write-Host "[anica-runner] WARNING: anica-acp build failed (exit code $LASTEXITCODE)." -ForegroundColor Red
        }
    }
}

# Launch the main binary.
& $ExePath @Args
exit $LASTEXITCODE
