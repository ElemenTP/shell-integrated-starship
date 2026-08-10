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
/// Strings returned by ssp_session_render must be freed with ssp_free.
/// ssp_version and ssp_last_error strings are static / thread-local and
/// must NOT be freed.
/// </summary>
internal static unsafe partial class NativeMethods
{
    // Platform-specific library name. .NET runtime resolves these as:
    //   Linux:   libstarship_ffi.so
    //   macOS:   libstarship_ffi.dylib
    //   Windows: starship_ffi.dll
    private const string LibName = "starship_ffi";

    // ── Session lifecycle ──────────────────────────────────────────────

    /// <summary>Create a new prompt rendering session. Returns <see cref="IntPtr.Zero"/> on failure.</summary>
    [LibraryImport(LibName, EntryPoint = "ssp_session_create")]
    internal static partial IntPtr SessionCreate();

    /// <summary>Destroy a session. Passing <see cref="IntPtr.Zero"/> is safe (no-op).</summary>
    [LibraryImport(LibName, EntryPoint = "ssp_session_destroy")]
    internal static partial void SessionDestroy(IntPtr session);

    // ── Prompt rendering ────────────────────────────────────────────────

    /// <summary>
    /// Render a prompt. On success (return 0), writes a Rust-allocated
    /// UTF-8 string to <paramref name="output"/>. The caller must free it
    /// with <see cref="ssp_free"/>. On failure (return &lt;0), check
    /// <see cref="ssp_last_error"/>.
    /// </summary>
    [LibraryImport(LibName, EntryPoint = "ssp_session_render")]
    internal static partial int SessionRender(
        IntPtr session, IntPtr input, out IntPtr output);

    /// <summary>Free a string returned by <see cref="ssp_session_render"/>. NULL-safe.</summary>
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

    /// <summary>
    /// Return the last error as a thread-local, null-terminated UTF-8 string.
    /// Valid until the next FFI call from this thread. Returns
    /// <see cref="IntPtr.Zero"/> if no error. Must NOT be freed.
    /// </summary>
    [LibraryImport(LibName, EntryPoint = "ssp_last_error")]
    internal static partial IntPtr LastError();

    // ── Statistics ──────────────────────────────────────────────────────

    /// <summary>Retrieve cache performance statistics. Returns 0 on success.</summary>
    [LibraryImport(LibName, EntryPoint = "ssp_session_stats")]
    internal static partial int SessionStats(
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
