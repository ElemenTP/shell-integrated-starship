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
Write-Host "DLL dir: $DllDir"

if (-not $DllDir) {
    $DllDir = "$PSScriptRoot/../pwsh_src/StarshipNative/bin/Release/net8.0"
}

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
    $ver = [StarshipNative.PromptSession]::Version()
    Write-Host "PASS: version $ver"
} catch {
    Write-Host "FAIL: could not get version: $_"
    exit 1
}

# Create session and render
try {
    $session = New-Object StarshipNative.PromptSession
    Write-Host "PASS: session created"

    # Render main prompt
    $result = $session.Render($null, $null, $null, 0, 0, 80, $null, $null, 0)
    if ($result.Length -gt 0) {
        Write-Host "PASS: prompt rendered ($($result.Length) chars)"
    } else {
        Write-Host "FAIL: empty prompt"
        $session.Dispose()
        exit 1
    }

    # Render right prompt
    $rightResult = $session.Render($null, $null, $null, 0, 0, 80, $null, $null, 1)
    Write-Host "PASS: right prompt rendered ($($rightResult.Length) chars)"

    # Get stats
    $stats = $session.GetStatsReport()
    Write-Host "PASS: stats retrieved — $stats"

    $session.Dispose()
    Write-Host "PASS: session disposed"
} catch {
    Write-Host "FAIL: $_"
    exit 1
}

Write-Host "=== All pwsh tests passed ==="
