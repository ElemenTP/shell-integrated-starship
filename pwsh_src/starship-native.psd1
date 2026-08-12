@{
    # StarshipNative module manifest.
    #
    # Hybrid script + binary module: the .psm1 contains the prompt logic and is
    # the RootModule; the C# assembly (StarshipNative.dll) is loaded as a
    # NestedModule so its types are available to the .psm1 at import time.

    RootModule           = 'StarshipNative.psm1'
    ModuleVersion        = '0.1.0'
    GUID                 = 'f2dbc61d-129c-43b4-84ed-6d6f8f0ab90e'

    # TODO: replace with your own identity before publishing.
    Author               = 'ElemenTP'
    CompanyName          = 'ElemenTP'
    Copyright            = '(c) ElemenTP. ISC license.'

    Description          = 'In-process Starship prompt for PowerShell (pwsh 7+). Renders the Starship prompt inside the shell process via a Rust FFI library — zero-fork prompt rendering with cross-render caching.'

    PowerShellVersion    = '7.2'
    CompatiblePSEditions = @('Core')

    NestedModules        = @('StarshipNative.dll')

    # prompt is installed as a global function by the .psm1 and must NOT be
    # listed here (it does not exist in module scope).
    FunctionsToExport    = @(
        'Get-StarshipNativeVersion'
        'Get-StarshipNativeStats'
        'Enable-TransientPrompt'
        'Disable-TransientPrompt'
        'Set-StarshipEnv'
        'Remove-StarshipEnv'
    )
    CmdletsToExport      = @()
    VariablesToExport    = @()
    AliasesToExport      = @()

    PrivateData          = @{
        PSData = @{
            Tags = @(
                'starship', 'prompt', 'shell', 'PSReadLine',
                'PSEdition_Core', 'Linux', 'macOS', 'Windows',
                'native', 'in-process', 'rust', 'ffi'
            )
            # TODO: replace with the real repository / license URLs before
            # publishing (PowerShellGet validates these on Publish-Module).
            ProjectUri              = 'https://github.com/ElemenTP/starship'
            LicenseUri              = 'https://github.com/ElemenTP/starship/blob/master/LICENSE'
            IconUri                 = ''
            ReleaseNotes            = @'
## 0.1.0

- Initial release: in-process Starship prompt for PowerShell (pwsh 7+).
- Zero-fork prompt rendering through a Rust FFI library.
- Cross-render caching of git status, dir contents, config, etc.
- `Get-StarshipNativeVersion` / `Get-StarshipNativeStats` helpers.
'@
            Prerelease              = ''
            RequireLicenseAcceptance = $false
        }
    }
}
