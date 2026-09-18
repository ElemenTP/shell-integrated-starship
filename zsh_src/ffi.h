/*
 * ffi.h — C declarations for the starship-ffi library (ssp_* API).
 *
 * Include this header in the zsh module shim (module.c) or in C test harnesses.
 */
#ifndef SSP_FFI_H
#define SSP_FFI_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Input parameters for a single prompt render. */
typedef struct {
  const char *status;            /* NULL or status-code string (e.g. "0") */
  const char *const *pipestatus; /* NULL or array of pipe-status strings */
  size_t pipestatus_len;         /* Number of entries in pipestatus */
  size_t terminal_width;         /* 0 = auto-detect from terminal */
  const char *path;              /* cwd path; NULL = process cwd */
  const char *logical_path;      /* logical cwd path; NULL = process cwd */
  const char *cmd_duration;      /* Duration string in ms; NULL = none */
  const char *keymap;            /* Keymap name; NULL = "viins" */
  long long jobs;                /* Number of background jobs */
  long long shlvl;               /* SHLVL value; -1 = none */
  int target;                    /* 0 = Main, 1 = Right, 2 = Continuation */
} ssp_render_input_t;

/* Cache performance counters. */
typedef struct {
  unsigned long long config_hits, config_misses;
  unsigned long long repo_status_hits, repo_status_misses;
  unsigned long long git_repo_hits, git_repo_misses;
  unsigned long long git_metrics_hits, git_metrics_misses;
  unsigned long long dir_contents_hits, dir_contents_misses;
  unsigned long long binary_path_hits, binary_path_misses;
  unsigned long long renders;
} ssp_stats_t;

/*
 * Error protocol for all fallible functions below:
 *   NULL  -> success
 *   other -> pointer to a NUL-terminated UTF-8 error message allocated by the
 *            library. The caller MUST release it with ssp_free().
 *
 * There are no global or per-session error slots to read after a call;
 * the error is carried directly by the return value.
 *
 * There is exactly one global session per shell process. Create it once before
 * rendering; destroy it during module unload. A later ssp_init()
 * after ssp_shutdown() creates a fresh session with empty caches.
 */

/* Create the global session. Returns NULL on success, or an allocated error
 * string if a session already exists or creation fails. */
char *ssp_init(void);

/* Destroy the global session. Returns NULL on success, including when no
 * session exists (idempotent). */
char *ssp_shutdown(void);

/* Render a prompt. On success writes the prompt to *out and returns NULL.
 * On failure sets *out to NULL and returns an allocated error string. */
char *ssp_render(const ssp_render_input_t *in, char **out);

/* Retrieve cache statistics. On success writes the snapshot to *out and
 * returns NULL, otherwise returns an allocated error string. */
char *ssp_stats(ssp_stats_t *out);

/* Free a string returned by any fallible ssp_* function, or a prompt string
 * returned by ssp_render. NULL is safe. This function cannot fail and
 * therefore returns void rather than an error string. */
void ssp_free(char *ptr);

/* Return the library version string (static, no free needed). This accessor
 * cannot fail; it is not part of the error protocol. */
const char *ssp_version(void);

#ifdef __cplusplus
}
#endif

#endif /* SSP_FFI_H */
