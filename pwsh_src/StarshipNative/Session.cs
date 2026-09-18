using System.Runtime.InteropServices;
using System.Text;

namespace StarshipNative;

/// <summary>
/// Safe managed wrapper around the starship-ffi native library.
///
/// The native library owns exactly one session per shell process. This class
/// exposes it as static methods; <see cref="Initialize"/> is idempotent and
/// <see cref="Shutdown"/> releases the session (for example when the PowerShell
/// module is removed). A later <see cref="Initialize"/> + <see cref="Render"/>
/// creates a fresh native session.
///
/// Usage:
///   Session.Initialize();
///   string prompt = Session.Render(
///       status: "0",
///       duration: null,
///       jobs: 0,
///       width: 120,
///       keymap: "viins",
///       target: 0  // Main
///   );
///   Session.Shutdown();
///
/// Native failures are surfaced directly as the allocated error string
/// returned by each FFI call; this wrapper frees it and includes its text in
/// the thrown exception.
/// </summary>
public static class Session
{
    /// <summary>
    /// Create the process-wide native session. Idempotent: calling this again
    /// after a successful initialization is a no-op.
    /// </summary>
    public static void Initialize()
    {
        IntPtr error = NativeMethods.Init();
        if (error != IntPtr.Zero)
        {
            throw new InvalidOperationException(
                $"Failed to initialize starship session: {TakeError(error)}");
        }
    }

    /// <summary>
    /// Release the process-wide native session. Idempotent; errors are
    /// consumed because this is normally called during module teardown.
    /// </summary>
    public static void Shutdown()
    {
        IntPtr error = NativeMethods.Shutdown();
        if (error != IntPtr.Zero)
        {
            throw new InvalidOperationException(
                $"Failed to shutdown starship session: {TakeError(error)}");
        }
    }


    /// <summary>
    /// Render a prompt for the given parameters.
    /// </summary>
    /// <param name="status">Exit status code as string, or null.</param>
    /// <param name="pipestatus">Pipe status array, or null.</param>
    /// <param name="duration">Command duration in ms as string, or null.</param>
    /// <param name="jobs">Number of background jobs.</param>
    /// <param name="shlvl">SHLVL value, or 0 for none.</param>
    /// <param name="width">Terminal width in columns, or 0 for auto-detect.</param>
    /// <param name="path">Working directory, or null for process cwd.</param>
    /// <param name="logicalpath">Logical working directory.</param>
    /// <param name="keymap">Current keymap, or null for "viins".</param>
    /// <param name="target">0=Main, 1=Right, 2=Continuation.</param>
    /// <returns>The rendered prompt string.</returns>
    public static string Render(
        string? status = null,
        string?[]? pipestatus = null,
        string? duration = null,
        long jobs = 0,
        long shlvl = 0,
        ulong width = 0,
        string? path = null,
        string? logicalpath = null,
        string? keymap = null,
        int target = 0)
    {
        // BuildInput allocates unmanaged memory for all strings. The returned
        // NativeInput owns every block and is freed in the finally below, so
        // nothing leaks even if the native call throws.
        var input = BuildInput(status, pipestatus, duration, jobs, shlvl,
                               width, path, logicalpath, keymap, target);
        try
        {
            // Pass the struct by address (it's a [StructLayout(Sequential)] value type).
            IntPtr inputPtr = Marshal.AllocHGlobal(Marshal.SizeOf<SspRenderInput>());
            try
            {
                Marshal.StructureToPtr(input.Input, inputPtr, false);
                IntPtr error = NativeMethods.Render(inputPtr, out IntPtr output);
                if (error != IntPtr.Zero)
                {
                    throw new InvalidOperationException(
                        $"starship render failed: {TakeError(error)}");
                }
                if (output == IntPtr.Zero)
                {
                    throw new InvalidOperationException(
                        "starship render succeeded but returned no output");
                }

                try
                {
                    return Marshal.PtrToStringUTF8(output) ?? string.Empty;
                }
                finally
                {
                    NativeMethods.Free(output);
                }
            }
            finally
            {
                Marshal.FreeHGlobal(inputPtr);
            }
        }
        finally
        {
            input.Dispose();
        }
    }

    /// <summary>
    /// Get the library version string.
    /// </summary>
    public static string Version()
    {
        IntPtr ptr = NativeMethods.Version();
        return Marshal.PtrToStringUTF8(ptr) ?? "unknown";
    }

    /// <summary>
    /// Get cache performance statistics as a human-readable string.
    /// </summary>
    public static string GetStatsReport()
    {
        IntPtr error = NativeMethods.Stats(out var stats);
        if (error != IntPtr.Zero)
        {
            throw new InvalidOperationException(
                $"starship stats failed: {TakeError(error)}");
        }
        return $"Renders: {stats.Renders}, " +
               $"Config: {stats.ConfigHits}h/{stats.ConfigMisses}m, " +
               $"RepoStatus: {stats.RepoStatusHits}h/{stats.RepoStatusMisses}m, " +
               $"GitRepo: {stats.GitRepoHits}h/{stats.GitRepoMisses}m, " +
               $"GitMetrics: {stats.GitMetricsHits}h/{stats.GitMetricsMisses}m, " +
               $"DirContents: {stats.DirContentsHits}h/{stats.DirContentsMisses}m, " +
               $"BinaryPath: {stats.BinaryPathHits}h/{stats.BinaryPathMisses}m";
    }


    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    /// <summary>
    /// Consume an error pointer returned by a native call: NULL means success
    /// (returns null), otherwise the message is copied into a managed string
    /// and the native allocation is released with zo_free.
    /// </summary>
    private static string? TakeError(IntPtr error)
    {
        if (error == IntPtr.Zero)
            return null;
        string? message = Marshal.PtrToStringUTF8(error);
        NativeMethods.Free(error);
        return message;
    }

    private static NativeInput BuildInput(
        string? status,
        string?[]? pipestatus,
        string? duration,
        long jobs,
        long shlvl,
        ulong width,
        string? path,
        string? logicalpath,
        string? keymap,
        int target)
    {
        var native = new NativeInput();
        try
        {
            native.Input = new SspRenderInput
            {
                Status = native.AllocUtf8(status),
                Pipestatus = native.AllocUtf8Array(pipestatus),
                PipestatusLen = (UIntPtr)(pipestatus?.Length ?? 0),
                TerminalWidth = (UIntPtr)width,
                Path = native.AllocUtf8(path),
                LogicalPath = native.AllocUtf8(logicalpath),
                CmdDuration = native.AllocUtf8(duration),
                Keymap = native.AllocUtf8(keymap),
                Jobs = jobs,
                Shlvl = shlvl > 0 ? shlvl : -1,
                Target = target,
            };
            return native;
        }
        catch
        {
            // If any allocation fails partway through, free what was already
            // allocated before propagating the exception.
            native.Dispose();
            throw;
        }
    }

    /// <summary>
    /// Owns the unmanaged memory backing an <see cref="SspRenderInput"/> and
    /// frees every block on <see cref="Dispose"/>. Strings are copied into
    /// unmanaged memory with <see cref="Marshal.AllocHGlobal"/>, so the native
    /// call can read them without GC pinning and nothing leaks.
    /// </summary>
    private sealed class NativeInput : IDisposable
    {
        private readonly List<IntPtr> _blocks = new();

        /// <summary>The input struct whose pointers reference the allocated blocks.</summary>
        public SspRenderInput Input;

        /// <summary>Copy a null-terminated UTF-8 string into unmanaged memory. Null-safe.</summary>
        public IntPtr AllocUtf8(string? s)
        {
            if (s == null) return IntPtr.Zero;
            byte[] bytes = Encoding.UTF8.GetBytes(s + '\0');
            IntPtr ptr = Marshal.AllocHGlobal(bytes.Length);
            _blocks.Add(ptr);
            Marshal.Copy(bytes, 0, ptr, bytes.Length);
            return ptr;
        }

        /// <summary>Copy an array of strings into an unmanaged array of char* pointers. Null-safe.</summary>
        public IntPtr AllocUtf8Array(string?[]? values)
        {
            if (values is not { Length: > 0 }) return IntPtr.Zero;

            IntPtr array = Marshal.AllocHGlobal(IntPtr.Size * values.Length);
            _blocks.Add(array);
            for (int i = 0; i < values.Length; i++)
            {
                // If this throws, the array and prior blocks are already
                // registered and will be freed by Dispose().
                Marshal.WriteIntPtr(array, i * IntPtr.Size, AllocUtf8(values[i]));
            }
            return array;
        }

        public void Dispose()
        {
            foreach (IntPtr ptr in _blocks)
                Marshal.FreeHGlobal(ptr);
            _blocks.Clear();
        }
    }
}
