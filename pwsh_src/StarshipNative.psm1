#Requires -Version 7.2
<#
.SYNOPSIS
    StarshipNative — in-process Starship prompt for PowerShell (pwsh 7+).

.DESCRIPTION
    This module mirrors the official starship/src/init/starship.ps1 script,
    replacing the `starship prompt` process invocation with the in-process
    [StarshipNative.Session] managed wrapper (see StarshipNative.dll).
    Importing this module installs a global `prompt` function that renders the
    prompt inside the pwsh process — zero fork, with cross-render caching.

    The native library (libstarship_ffi.so / libstarship_ffi.dylib /
    starship_ffi.dll) must be present next to StarshipNative.dll. To load it
    from another location, set the STARSHIP_FFI_PATH environment variable.

.EXAMPLE
    Import-Module StarshipNative
    Get-StarshipNativeVersion
#>

Set-StrictMode -Version Latest

# ---- Ensure the binary assembly is available --------------------------------
# Normally it is loaded as a NestedModule from the manifest; this fallback only
# matters when the .psm1 is imported directly during development.
if (-not ('StarshipNative.Session' -as [type])) {
    Import-Module (Join-Path $PSScriptRoot 'StarshipNative.dll') -ErrorAction Stop
}

# ---- Point the FFI resolver at a colocated native library -------------------
# (The C# resolver also falls back to default .NET probing of the assembly dir.)
$script:NativeLibName = switch ($true) {
    $IsWindows { 'starship_ffi.dll' }
    $IsMacOS   { 'libstarship_ffi.dylib' }
    default    { 'libstarship_ffi.so' }
}
if (-not $env:STARSHIP_FFI_PATH) {
    $colocated = Join-Path $PSScriptRoot $script:NativeLibName
    if (Test-Path -LiteralPath $colocated -PathType Leaf) {
        $env:STARSHIP_FFI_PATH = $colocated
    }
}

# The native Rust library must see STARSHIP_SHELL before the first render.
[StarshipNative.StarshipEnvironment]::Set('STARSHIP_SHELL', 'pwsh')

# ---- Native session (created once, lives for the pwsh process) ---------------
$script:__Session = $null
$script:__NativeWarned = $false

function Get-Session {
    if ($null -eq $script:__Session) {
        $script:__Session = [StarshipNative.Session]::new()
    }
    return $script:__Session
}

# ---- Public metadata helpers -------------------------------------------------

function Get-StarshipNativeVersion {
    <#
    .SYNOPSIS
        Returns the version of the embedded starship-ffi native library.
    #>
    return [StarshipNative.Session]::Version()
}

function Get-StarshipNativeStats {
    <#
    .SYNOPSIS
        Returns cache performance statistics for the current session.
    #>
    return (Get-Session).GetStatsReport()
}

# ---- The rest is adapted from starship/src/init/starship.ps1 -----------------

function Get-Cwd {
    $cwd = Get-Location
    $provider_prefix = "$($cwd.Provider.ModuleName)\$($cwd.Provider.Name)::"
    return @{
        Path = $cwd.ProviderPath;
        LogicalPath =
            if ($cwd.Path.StartsWith($provider_prefix)) {
                $cwd.Path.Substring($provider_prefix.Length)
            } else {
                $cwd.Path
            };
    }
}

function Enable-TransientPrompt {
    <#
    .SYNOPSIS
        Shows a transient prompt after the current line is executed.
    #>
    Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
        $previousOutputEncoding = [Console]::OutputEncoding
        try {
            $parseErrors = $null
            [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState(
                [ref]$null, [ref]$null, [ref]$parseErrors, [ref]$null)
            if ($parseErrors.Count -eq 0) {
                $script:TransientPrompt = $true
                [Console]::OutputEncoding = [Text.Encoding]::UTF8
                [Microsoft.PowerShell.PSConsoleReadLine]::InvokePrompt()
            }
        } finally {
            if ($script:DoesUseLists) {
                [Microsoft.PowerShell.PSConsoleReadLine]::Insert(
                    "`n" * [math]::Min(
                        $Host.UI.RawUI.WindowSize.Height - $Host.UI.RawUI.CursorPosition.Y - 1, 12))
                [Microsoft.PowerShell.PSConsoleReadLine]::Undo()
            }
            [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
            [Console]::OutputEncoding = $previousOutputEncoding
        }
    }
}

function Disable-TransientPrompt {
    <#
    .SYNOPSIS
        Disables the transient prompt behavior.
    #>
    Set-PSReadLineKeyHandler -Key Enter -Function AcceptLine
    $script:TransientPrompt = $false
}

# ---- Preserve any pre-existing prompt so we can restore it on removal --------
$script:starshipOriginalPrompt = $null
if (Test-Path function:\prompt) {
    $script:starshipOriginalPrompt = $function:prompt
}

function global:prompt {
    $origDollarQuestion = $global:?
    # LASTEXITCODE may not exist yet on a fresh pwsh (no native command has
    # run); guard the read so StrictMode does not throw.
    $origLastExitCode = try { $global:LASTEXITCODE } catch { $null }

    # Invoke precmd, if specified
    try {
        if (Test-Path function:Invoke-Starship-PreCommand) {
            Invoke-Starship-PreCommand
        }
    } catch {}

    # @ makes sure the result is an array even if single or no values are returned
    $jobs = @(Get-Job | Where-Object { $_.State -eq 'Running' }).Count

    $cwd = Get-Cwd

    # We start from the premise that the command executed correctly
    $lastExitCodeForPrompt = 0
    $cmdDuration = $null
    if ($lastCmd = Get-History -Count 1) {
        if (-not $origDollarQuestion) {
            $lastCmdletError = try {
                $global:error[0] | Where-Object { $_ -ne $null } |
                    Select-Object -ExpandProperty InvocationInfo
            } catch { $null }
            $lastExitCodeForPrompt = if ($null -ne $lastCmdletError -and
                $lastCmd.CommandLine -eq $lastCmdletError.Line) { 1 } else { $origLastExitCode }
        }
        $duration = [math]::Round(
            ($lastCmd.EndExecutionTime - $lastCmd.StartExecutionTime).TotalMilliseconds)
        $cmdDuration = [string]$duration
    }

    $statusCode = if ($lastExitCodeForPrompt -ne 0) {
        [string]$lastExitCodeForPrompt
    } else { $null }

    # Keymap from PSReadLine
    $keymap = $null
    try {
        if ([Microsoft.PowerShell.PSConsoleReadLine]::InViCommandMode()) {
            $keymap = "vi"
        }
    } catch {}

    # Terminal width
    $width = 0
    try {
        $width = $Host.UI.RawUI.WindowSize.Width
    } catch {}

    # ---- Native: render prompt in-process ----
    try {
        $session = Get-Session
        $promptText = if ($script:TransientPrompt) {
            $script:TransientPrompt = $false
            if (Test-Path function:Invoke-Starship-TransientFunction) {
                Invoke-Starship-TransientFunction
            } else {
                "$([char]0x1B)[1;32m❯$([char]0x1B)[0m "
            }
        } else {
            $session.Render(
                $statusCode,        # status
                $null,              # pipestatus
                $cmdDuration,       # duration
                [long]$jobs,        # jobs
                0,                  # shlvl
                [ulong]$width,      # width
                $cwd.LogicalPath,   # path
                $keymap,            # keymap
                0                   # target: Main
            )
        }
    } catch {
        # Fail gracefully (e.g. native library missing) — warn once, then fall
        # back to a plain prompt so the shell stays usable.
        if (-not $script:__NativeWarned) {
            $script:__NativeWarned = $true
            Write-Warning "StarshipNative: prompt render failed, falling back to default. $_"
        }
        $promptText = "PS $($executionContext.SessionState.Path.CurrentLocation)> "
    }

    # Set the number of extra lines in the prompt for PSReadLine prompt redraw.
    Set-PSReadLineOption -ExtraPromptLineCount ($promptText.Split("`n").Length - 1)

    # Return the prompt
    $promptText

    # Propagate the original $LASTEXITCODE
    $global:LASTEXITCODE = $origLastExitCode

    # Propagate the original $?
    if ($global:? -ne $origDollarQuestion) {
        if ($origDollarQuestion) { 1+1 } else {
            Write-Error '' -ErrorAction 'Ignore'
        }
    }
}

# Disable virtualenv prompt, it breaks starship
$ENV:VIRTUAL_ENV_DISABLE_PROMPT=1

$script:TransientPrompt = $false
$script:DoesUseLists = (Get-PSReadLineOption).PredictionViewStyle -eq 'ListView'

# Set up the session key that will be used to store logs
$STARSHIP_SESSION_KEY = -join ((48..57) + (65..90) + (97..122) |
    Get-Random -Count 16 | ForEach-Object { [char]$_ })
[StarshipNative.StarshipEnvironment]::Set('STARSHIP_SESSION_KEY', $STARSHIP_SESSION_KEY)

# ---- Native: set continuation prompt ----
try {
    $session = Get-Session
    $contPrompt = $session.Render(
        $null, $null, $null, 0, 0, 0,  # no props needed for continuation
        $null, $null, 2)                # target: Continuation
    Set-PSReadLineOption -ContinuationPrompt $contPrompt
} catch {
    Set-PSReadLineOption -ContinuationPrompt "> "
}

# Vi mode change handler
try {
    if ((Get-PSReadLineOption).ViModeChangeHandler) {
        & {
            $originalHandler = (Get-PSReadLineOption).ViModeChangeHandler
            Set-PSReadLineOption -ViModeChangeHandler {
                [Microsoft.PowerShell.PSConsoleReadLine]::InvokePrompt()
                & $originalHandler @args
            }.GetNewClosure()
        }
    } else {
        Set-PSReadLineOption -ViModeIndicator script -ViModeChangeHandler {
            [Microsoft.PowerShell.PSConsoleReadLine]::InvokePrompt()
        }
    }
} catch {}

# ---- Environment helper for users ----
# Usage: Set-StarshipEnv STARSHIP_NATIVE_TTL_MS 10000
# This ensures the variable is visible to the in-process Rust library.
function Set-StarshipEnv {
    <#
    .SYNOPSIS
        Sets an environment variable so it is visible to the in-process native
        Rust library (also updates the .NET environment block).
    .EXAMPLE
        Set-StarshipEnv STARSHIP_NATIVE_TTL_MS 10000
    #>
    param([string]$Name, [string]$Value)
    [StarshipNative.StarshipEnvironment]::Set($Name, $Value)
}

function Remove-StarshipEnv {
    <#
    .SYNOPSIS
        Removes an environment variable from both the .NET and native blocks.
    .EXAMPLE
        Remove-StarshipEnv STARSHIP_NATIVE_NO_CACHE
    #>
    param([string]$Name)
    [StarshipNative.StarshipEnvironment]::Remove($Name)
}

# ---- Export the public API ---------------------------------------------------
Export-ModuleMember -Function @(
    "Get-StarshipNativeVersion"
    "Get-StarshipNativeStats"
    "Enable-TransientPrompt"
    "Disable-TransientPrompt"
    "Set-StarshipEnv"
    "Remove-StarshipEnv"
)

# ---- Restore the original prompt when the module is removed ------------------
$MyInvocation.MyCommand.ScriptBlock.Module.OnRemove = {
    if ($null -ne $script:starshipOriginalPrompt) {
        Set-Item -Path function:\prompt -Value $script:starshipOriginalPrompt
    }
    if ($null -ne $script:__Session) {
        try { $script:__Session.Dispose() } catch {}
        $script:__Session = $null
    }
}
