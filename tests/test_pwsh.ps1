# PowerShell integration test for StarshipNative module.
#
# Usage:
#   $env:DLL_DIR = "/path/to/pwsh_src/StarshipNative/bin/Release/net8.0"
#   pwsh -NoProfile -File tests/test_pwsh.ps1

param(
    [string]$DllDir = $env:DLL_DIR
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Write-Host "=== pwsh integration test ==="
if (-not $DllDir) {
    $DllDir = "$PSScriptRoot/../pwsh_src/StarshipNative/bin/Release/net8.0"
}
Write-Host "DLL dir: $DllDir"

# Verify the assembly exists
$asmPath = Join-Path $DllDir "StarshipNative.dll"
if (-not (Test-Path $asmPath)) {
    Write-Host "FAIL: StarshipNative.dll not found at $asmPath"
    exit 1
}
Write-Host "PASS: assembly file found"

# Load the assembly
try {
    Add-Type -Path $asmPath
    Write-Host "PASS: assembly loaded"
} catch {
    Write-Host "FAIL: could not load assembly: $_"
    exit 1
}

# Check version
try {
    $ver = [StarshipNative.Session]::Version()
    Write-Host "PASS: version $ver"
} catch {
    Write-Host "FAIL: could not get version: $_"
    exit 1
}

# Initialize the process-wide session.
try {
    [StarshipNative.Session]::Initialize()
    Write-Host "PASS: session initialized"

    # Initialize is idempotent.
    [StarshipNative.Session]::Initialize()
    Write-Host "PASS: second Initialize() is a no-op"

    # Render main prompt
    $result = [StarshipNative.Session]::Render($null, $null, $null, 0, 0, 80, $null, $null, $null, 0)
    if ($result.Length -gt 0) {
        Write-Host "PASS: prompt rendered ($($result.Length) chars)"
    } else {
        Write-Host "FAIL: empty prompt"
        [StarshipNative.Session]::Shutdown()
        exit 1
    }

    # Render right prompt
    $rightResult = [StarshipNative.Session]::Render($null, $null, $null, 0, 0, 80, $null, $null, $null, 1)
    Write-Host "PASS: right prompt rendered ($($rightResult.Length) chars)"

    # Render continuation prompt
    $rightResult = [StarshipNative.Session]::Render($null, $null, $null, 0, 0, 80, $null, $null, $null, 2)
    Write-Host "PASS: continuation prompt rendered ($($rightResult.Length) chars)"

    # Get stats
    $stats = [StarshipNative.Session]::GetStatsReport()
    Write-Host "PASS: stats retrieved — $stats"

    # Shutdown and recreate: the same process must be able to start a fresh session.
    [StarshipNative.Session]::Shutdown()
    Write-Host "PASS: session shutdown"

    [StarshipNative.Session]::Initialize()
    $recreated = [StarshipNative.Session]::Render($null, $null, $null, 0, 0, 80, $null, $null, $null, 0)
    if ($recreated.Length -gt 0) {
        Write-Host "PASS: prompt rendered after recreate ($($recreated.Length) chars)"
    } else {
        Write-Host "FAIL: empty prompt after recreate"
        [StarshipNative.Session]::Shutdown()
        exit 1
    }

    [StarshipNative.Session]::Shutdown()
    Write-Host "PASS: final session shutdown"
} catch {
    Write-Host "FAIL: $_"
    try { [StarshipNative.Session]::Shutdown() } catch {}
    exit 1
}

Write-Host "=== All pwsh tests passed ==="
