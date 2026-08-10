using System.Runtime.InteropServices;

namespace StarshipNative;

/// <summary>
/// Cross-platform environment variable helper.
///
/// On Linux/macOS, .NET's <c>Environment.SetEnvironmentVariable</c> and PowerShell's
/// <c>$env:</c> only affect the .NET-managed environment block — they do NOT call the
/// native C library's <c>setenv()</c>. This means in-process native code (like the
/// Rust starship-ffi library loaded via P/Invoke) cannot see those variables via
/// <c>getenv()</c> / <c>std::env::var()</c>.
///
/// This class provides a unified <c>Set</c> method that writes to BOTH environment
/// blocks, ensuring starship environment variables (STARSHIP_CONFIG, STARSHIP_CACHE,
/// STARSHIP_NATIVE_TTL_MS, etc.) are visible to managed code, native Rust code,
/// and child processes.
/// </summary>
public static partial class StarshipEnvironment
{
    /// <summary>
    /// Set an environment variable visible to both .NET and native in-process code.
    /// This should be used instead of <c>$env:NAME = "value"</c> for any variable
    /// that the embedded starship-ffi library needs to read.
    /// </summary>
    public static void Set(string name, string value)
    {
        // 1. .NET environment (for subprocess compatibility, $env: reads, etc.)
        System.Environment.SetEnvironmentVariable(name, value);

        // 2. Native OS environment (for in-process getenv() / std::env::var())
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            SetEnv(name, value, 1);
        }
    }

    /// <summary>
    /// Remove an environment variable from both blocks.
    /// </summary>
    public static void Remove(string name)
    {
        System.Environment.SetEnvironmentVariable(name, null);

        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            UnsetEnv(name);
        }
    }

    private const string LibC = "libc";

    [LibraryImport(LibC, EntryPoint = "setenv", StringMarshalling = StringMarshalling.Utf8)]
    private static partial int SetEnv(string name, string value, int overwrite);

    [LibraryImport(LibC, EntryPoint = "unsetenv", StringMarshalling = StringMarshalling.Utf8)]
    private static partial int UnsetEnv(string name);
}
