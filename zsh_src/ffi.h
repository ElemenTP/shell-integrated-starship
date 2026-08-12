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

/* Opaque session handle. */
typedef struct ssp_session ssp_session_t;

/* Input parameters for a single prompt render. */
typedef struct {
  const char *status;            /* NULL or status-code string (e.g. "0") */
  const char *const *pipestatus; /* NULL or array of pipe-status strings */
  size_t pipestatus_len;         /* Number of entries in pipestatus */
  size_t terminal_width;         /* 0 = auto-detect from terminal */
  const char *path;              /* Logical cwd; NULL = process cwd */
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

/* Session lifecycle. */
ssp_session_t *ssp_session_create(void);
void ssp_session_shutdown(ssp_session_t *s);
void ssp_session_destroy(ssp_session_t *s);

/* Render a prompt. Returns 0 on success, <0 on error.
 * The caller must free *out with ssp_free(). */
int ssp_session_render(ssp_session_t *s, const ssp_render_input_t *in,
                       char **out);

/* Free a string returned by ssp_session_render. NULL is safe. */
void ssp_free(char *ptr);

/* Return the library version string (static, no free needed). */
const char *ssp_version(void);

/* Return the last error message (static mutex guarded, copy out, should be freed with ssp_free). */
void ssp_last_error(char **out);

/* Retrieve session statistics. Returns 0 on success. */
int ssp_session_stats(const ssp_session_t *s, ssp_stats_t *out);

#ifdef __cplusplus
}
#endif

#endif /* SSP_FFI_H */
