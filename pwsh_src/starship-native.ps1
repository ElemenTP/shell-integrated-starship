# Starship Native — In-process prompt for PowerShell (pwsh 7+).
#
# This script mirrors the official starship/src/init/starship.ps1 script,
# replacing the `starship prompt` process invocation with the in-process
# [StarshipNative.PromptSession] managed wrapper.
#
# Import the module:
#   Import-Module /path/to/module

# Use the cross-platform helper so the native Rust library can see this variable.
[StarshipNative.StarshipEnvironment]::Set("STARSHIP_SHELL", "pwsh")

# Create a new dynamic module so we don't pollute the global namespace
$null = New-Module starship_native {
    # ---- Native session (created once, lives for the pwsh process) ----
    $script:__Session = $null

    function Get-Session {
        if ($null -eq $script:__Session) {
            $script:__Session = [StarshipNative.PromptSession]::new()
        }
        return $script:__Session
    }

    # ---- The rest is adapted from starship/src/init/starship.ps1 ----

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
        Set-PSReadLineKeyHandler -Key Enter -Function AcceptLine
        $script:TransientPrompt = $false
    }

    function global:prompt {
        $origDollarQuestion = $global:?
        $origLastExitCode = $global:LASTEXITCODE

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
    $ENV:STARSHIP_SESSION_KEY = -join ((48..57) + (65..90) + (97..122) |
        Get-Random -Count 16 | ForEach-Object { [char]$_ })

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
    function global:Set-StarshipEnv {
        param([string]$Name, [string]$Value)
        [StarshipNative.StarshipEnvironment]::Set($Name, $Value)
    }

    function global:Remove-StarshipEnv {
        param([string]$Name)
        [StarshipNative.StarshipEnvironment]::Remove($Name)
    }

    Export-ModuleMember -Function @(
        "Enable-TransientPrompt"
        "Disable-TransientPrompt"
        "Set-StarshipEnv"
        "Remove-StarshipEnv"
    )
}