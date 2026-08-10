@{
    # Module manifest for StarshipNative
    RootModule           = 'StarshipNative.dll'
    ModuleVersion        = '0.1.0'
    GUID                 = 'f2dbc61d-129c-43b4-84ed-6d6f8f0ab90e'
    Author               = 'Starship Contributors'
    CompanyName          = 'Starship'
    Copyright            = '(c) Starship Contributors. ISC license.'
    Description          = 'In-process Starship prompt for PowerShell (pwsh). Uses the starship-ffi native library for zero-fork prompt rendering.'
    PowerShellVersion    = '7.2'
    CompatiblePSEditions = @('Core')
    FunctionsToExport    = @('prompt')
    CmdletsToExport      = @()
    VariablesToExport    = @()
    AliasesToExport      = @()
    PrivateData          = @{
        PSData = @{
            Tags       = @('prompt', 'starship', 'shell')
            ProjectUri = 'https://starship.rs'
        }
    }
}
