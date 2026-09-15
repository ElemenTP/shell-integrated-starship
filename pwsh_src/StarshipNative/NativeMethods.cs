using System.Reflection;
using System.Runtime.InteropServices;

namespace StarshipNative;

/// <summary>
/// P/Invoke declarations for the starship-ffi native library.
///
/// Uses .NET 7+ LibraryImport source generators for compile-time stub
/// generation — faster invocation and AOT-friendly compared to DllImport.
///
/// On Linux/macOS, the native library (libstarship_ffi.so / .dylib) must be
/// placed alongside StarshipNative.dll. On Windows, starship_ffi.dll must be
/// in the same directory.
///
/// A custom <see cref="NativeLibrary.SetDllImportResolver"/> honors the
/// STARSHIP_FFI_PATH environment variable (absolute path to the native lib),
/// falling back to default .NET resolution (which probes the directory of
/// this assembly — exactly where PowerShell Gallery extracts the module).
///
/// Fallible exports return a `char *` error: <see cref="IntPtr.Zero"/> means
/// success, and a non-zero value is an allocated error string that must be
/// freed with <see cref="Free"/>. Prompt strings returned through an out
/// parameter must also be freed with <see cref="Free"/>. ssp_version strings
/// are static and must NOT be freed.
/// </summary>
internal static unsafe partial class NativeMethods
{
    // Platform-specific library name. .NET runtime resolves these as:
    //   Linux:   libstarship_ffi.so
    //   macOS:   libstarship_ffi.dylib
    //   Windows: starship_ffi.dll
    private const string LibName = "starship_ffi";

    static NativeMethods()
    {
        NativeLibrary.SetDllImportResolver(typeof(NativeMethods).Assembly, ResolveNativeLibrary);
    }

    /// <summary>
    /// Resolve the starship-ffi native library. Honors STARSHIP_FFI_PATH;
    /// otherwise defers to the default runtime resolution so the library is
    /// found next to StarshipNative.dll.
    /// </summary>
    private static IntPtr ResolveNativeLibrary(
        string libraryName, Assembly assembly, DllImportSearchPath? searchPath)
    {
        if (!string.Equals(libraryName, LibName, StringComparison.OrdinalIgnoreCase))
            return IntPtr.Zero;

        string? overridePath = System.Environment.GetEnvironmentVariable("STARSHIP_FFI_PATH");
        if (!string.IsNullOrEmpty(overridePath))
        {
            string fullPath = Path.GetFullPath(overridePath);
            if (File.Exists(fullPath))
            {
                return NativeLibrary.Load(fullPath);
            }
        }

        // Fall back to default resolution (probes the assembly directory).
        return IntPtr.Zero;
    }

    // ── Session lifecycle ──────────────────────────────────────────────

    /// <summary>
    /// Create a new prompt rendering session. Returns <see cref="IntPtr.Zero"/>
    /// on success or an allocated error string; on failure
    /// <paramref name="session"/> is set to <see cref="IntPtr.Zero"/>.
    /// </summary>
    [LibraryImport(LibName, EntryPoint = "ssp_session_create")]
    internal static partial IntPtr SessionCreate(out IntPtr session);

    /// <summary>
    /// Destroy a session. Passing <see cref="IntPtr.Zero"/> is a successful
    /// no-op. Returns <see cref="IntPtr.Zero"/> on success or an allocated
    /// error string.
    /// </summary>
    [LibraryImport(LibName, EntryPoint = "ssp_session_destroy")]
    internal static partial IntPtr SessionDestroy(IntPtr session);

    // ── Prompt rendering ────────────────────────────────────────────────

    /// <summary>
    /// Render a prompt. Returns <see cref="IntPtr.Zero"/> on success and
    /// writes a Rust-allocated UTF-8 prompt string to
    /// <paramref name="output"/> (free it with <see cref="Free"/>). On failure
    /// returns an allocated error string and sets <paramref name="output"/> to
    /// <see cref="IntPtr.Zero"/>.
    /// </summary>
    [LibraryImport(LibName, EntryPoint = "ssp_session_render")]
    internal static partial IntPtr SessionRender(
        IntPtr session, IntPtr input, out IntPtr output);

    /// <summary>
    /// Free a string returned by any fallible ssp_* call. NULL-safe. This
    /// function cannot fail and returns void.
    /// </summary>
    [LibraryImport(LibName, EntryPoint = "ssp_free")]
    internal static partial void Free(IntPtr ptr);

    // ── Metadata ────────────────────────────────────────────────────────

    /// <summary>
    /// Return the library version as a static, null-terminated UTF-8 string.
    /// Must NOT be freed. Returns a raw pointer; use
    /// <see cref="Marshal.PtrToStringUTF8"/> to read.
    /// </summary>
    [LibraryImport(LibName, EntryPoint = "ssp_version")]
    internal static partial IntPtr Version();

    // ── Statistics ──────────────────────────────────────────────────────

    /// <summary>
    /// Retrieve cache performance statistics. Returns <see cref="IntPtr.Zero"/>
    /// on success and writes the snapshot to <paramref name="stats"/>; on
    /// failure returns an allocated error string.
    /// </summary>
    [LibraryImport(LibName, EntryPoint = "ssp_session_stats")]
    internal static partial IntPtr SessionStats(
        IntPtr session, out SspStats stats);
}

/// <summary>
/// C-compatible input struct for ssp_session_render.
/// Must exactly match the Rust ssp_render_input layout.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
internal struct SspRenderInput
{
    public IntPtr Status;         // const char* (UTF-8)
    public IntPtr Pipestatus;     // const char* const*
    public UIntPtr PipestatusLen; // size_t
    public UIntPtr TerminalWidth; // size_t
    public IntPtr Path;           // const char*
    public IntPtr LogicalPath;    // const char*
    public IntPtr CmdDuration;    // const char*
    public IntPtr Keymap;         // const char*
    public long Jobs;             // int64
    public long Shlvl;            // int64
    public int Target;            // 0=Main, 1=Right, 2=Continuation
}

/// <summary>
/// C-compatible stats struct for ssp_session_stats.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
internal struct SspStats
{
    public ulong ConfigHits, ConfigMisses;
    public ulong RepoStatusHits, RepoStatusMisses;
    public ulong GitRepoHits, GitRepoMisses;
    public ulong GitMetricsHits, GitMetricsMisses;
    public ulong DirContentsHits, DirContentsMisses;
    public ulong BinaryPathHits, BinaryPathMisses;
    public ulong Renders;
}
