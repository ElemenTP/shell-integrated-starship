/*
 * module.c — zsh loadable module for in-process Starship prompt rendering.
 *
 * This shim dynamically links against libstarship_ffi.so (Rust cdylib).
 * Both .so files must be placed in the same directory so that $ORIGIN
 * runtime linking works.
 *
 *
 * Loading in zsh:
 *   module_path+=(/path/to/module)
 *   zmodload starship_native
 *   starship_prompt   # renders and sets STARSHIP_PROMPT etc.
 */

#define MODULE
#define IMPORTING_MODULE_zshQsmain 1
#include "zsh.mdh"

#include "ffi.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ------------------------------------------------------------------ */
/* Module metadata                                                    */
/* ------------------------------------------------------------------ */

#define MODNAME "starship_native"
#define BUILTIN_STARSHIP_PROMPT "starship_prompt"
#define BUILTIN_STARSHIP_VERSION "starship_version"
#define BUILTIN_STARSHIP_STATS "starship_stats"

/* Forward declarations */
static int bin_ssp_prompt(char *name, char **argv, Options ops, int func);
static int bin_ssp_version(char *name, char **argv, Options ops, int func);
static int bin_ssp_stats(char *name, char **argv, Options ops, int func);

/* Builtin table */
static struct builtin bintab[] = {
    BUILTIN(BUILTIN_STARSHIP_PROMPT, 0, bin_ssp_prompt, 0, 0, 0, NULL, NULL),
    BUILTIN(BUILTIN_STARSHIP_VERSION, 0, bin_ssp_version, 0, -1, 0, NULL, NULL),
    BUILTIN(BUILTIN_STARSHIP_STATS, 0, bin_ssp_stats, 0, -1, 0, NULL, NULL),
};

/* Features table — only builtins, no conditions/math/params */
static struct features module_features = {
    bintab, sizeof(bintab) / sizeof(*bintab), /* builtins */
    NULL,   0,                                /* conditions */
    NULL,   0,                                /* math functions */
    NULL,   0,                                /* parameter definitions */
    0,                                        /* n_abstract */
};

/* ------------------------------------------------------------------ */
/* Session state (created in boot_, destroyed in cleanup_)            */
/* ------------------------------------------------------------------ */

static ssp_session_t *g_session = NULL;

/* ------------------------------------------------------------------ */
/* Helper: read a zsh string param safely                              */
/* ------------------------------------------------------------------ */

static const char *get_str_param(const char *name) {
  char *val = getsparam((char *)name);
  if (!val || !*val)
    return NULL;
  return val;
}

/* Helper: read a zsh integer param. Returns 0 if unset. */
static long get_int_param(const char *name) { return getiparam((char *)name); }

/* Helper: read a zsh array param. Returns array and sets *len. */
static char **get_arr_param(const char *name, size_t *len) {
  char **arr = getaparam((char *)name);
  if (!arr) {
    *len = 0;
    return NULL;
  }
  *len = arrlen(arr);
  return arr;
}

/* Helper: set a zsh string param.
 * The string from Rust is raw UTF-8. It must be metafied for zsh's internal
 * storage, otherwise bytes above 0x80 (all multi-byte UTF-8 characters and
 * Nerd Font icons) get corrupted by zsh's metafication/unmetafication
 * round-trip. META_DUP tells metafy() to allocate and return a new string. */
static void set_str_param(const char *name, char *val) {
  if (!val)
    return;
  setsparam((char *)name, ztrdup_metafy(val));
}

/* Report an FFI failure using the Rust-side last-error string. */
static void report_ffi_error(const char *operation) {
  char *err = NULL;
  ssp_last_error(&err);
  zwarnnam(MODNAME, "%s: %s", operation, err ? err : "unknown error");
  if (err)
    ssp_free(err);
}

/* ------------------------------------------------------------------ */
/* Builtin: starship_prompt                                           */
/* ------------------------------------------------------------------ */

/**
 * Read zsh prompt parameters, call the FFI renderer three times
 * (Main, Right, Continuation), and store results in shell params:
 *   STARSHIP_PROMPT
 *   STARSHIP_RPROMPT
 *   STARSHIP_PROMPT2
 *
 * Input params read (from the stock starship.zsh precmd/preexec):
 *   STARSHIP_CMD_STATUS   — exit status of last command
 *   STARSHIP_PIPE_STATUS  — array of pipe exit statuses
 *   STARSHIP_DURATION     — execution duration in ms
 *   STARSHIP_JOBS_COUNT   — number of background jobs
 *   KEYMAP                — current ZLE keymap
 *   COLUMNS               — terminal width
 *
 * Usage (no args): starship_prompt
 */
static int bin_ssp_prompt(UNUSED(char *name), UNUSED(char **argv),
                          UNUSED(Options ops), UNUSED(int func)) {

  if (!g_session) {
    zwarnnam(MODNAME, "%s: session not initialized", BUILTIN_STARSHIP_PROMPT);
    return 1;
  }

  /* --- Read zsh params --- */
  const char *status_str = get_str_param("STARSHIP_CMD_STATUS");
  const char *duration_str = get_str_param("STARSHIP_DURATION");
  const char *keymap_str = get_str_param("KEYMAP");
  long jobs_count = get_int_param("STARSHIP_JOBS_COUNT");
  long shlvl_val = get_int_param("SHLVL");

  /* COLUMNS is set by zsh; use 0 (auto-detect) if unset */
  long columns_val = get_int_param("COLUMNS");
  size_t terminal_width = (columns_val > 0) ? (size_t)columns_val : 0;

  /* Pipe status: STARSHIP_PIPE_STATUS array */
  size_t pipestatus_len = 0;
  char **pipestatus_arr =
      get_arr_param("STARSHIP_PIPE_STATUS", &pipestatus_len);

  /* --- Build render input --- */
  ssp_render_input_t input;
  memset(&input, 0, sizeof(input));
  input.status = status_str;
  input.pipestatus = (const char *const *)pipestatus_arr;
  input.pipestatus_len = pipestatus_len;
  input.terminal_width = terminal_width;
  input.path = NULL; /* Use process cwd (matches stock behavior) */
  input.cmd_duration = duration_str;
  input.keymap = keymap_str ? keymap_str : "viins";
  input.jobs = (long long)jobs_count;
  input.shlvl = (shlvl_val > 0) ? (long long)shlvl_val : -1;

  /* --- Render main prompt --- */
  static const char *param_names[] = {
      "STARSHIP_PROMPT",
      "STARSHIP_RPROMPT",
      "STARSHIP_PROMPT2",
  };

  /* Prompt target: 0 = Main, 1 = Right, 2 = Continuation */
  for (int i = 0; i < 3; i++) {
    char *out = NULL;
    int ret = 0;

    input.target = i;
    ret = ssp_session_render(g_session, &input, &out);
    if (ret == 0) {
      if (out) {
        set_str_param(param_names[i], out);
        ssp_free(out);
      }
    } else {
      report_ffi_error(BUILTIN_STARSHIP_PROMPT);
    }
  }

  return 0;
}

/* ------------------------------------------------------------------ */
/* Builtin: starship_version                                           */
/* ------------------------------------------------------------------ */

/**
 * Show the verion of starship-native, also expose as zsh parameters:
 * STARSHIP_VERSION
 *
 * Usage:
 *   starship_version        — set params + print version
 *   starship_version -q     — quiet (only set params, no output)
 */
static int bin_ssp_version(UNUSED(char *name), char **argv, UNUSED(Options ops),
                           UNUSED(int func)) {
  int quiet = 0;
  while (*argv) {
    if (strcmp(*argv, "-q") == 0)
      quiet = 1;
    argv++;
  }

  const char *version = ssp_version();
  version = version ? version : "unknown";
  setsparam((char *)"STARSHIP_VERSION", ztrdup(version));

  if (!quiet) {
    printf("%s", version);
  }

  return 0;
}

/* ------------------------------------------------------------------ */
/* Builtin: starship_stats                                            */
/* ------------------------------------------------------------------ */

/**
 * Retrieve cache performance statistics from the session and expose them
 * as zsh parameters: STARSHIP_STATS_*.
 *
 * Usage:
 *   starship_stats          — set params + print summary
 *   starship_stats -v       — verbose (also print individual counters)
 *   starship_stats -q       — quiet (only set params, no output)
 */
static int bin_ssp_stats(UNUSED(char *name), char **argv, UNUSED(Options ops),
                         UNUSED(int func)) {
  int verbose = 0, quiet = 0;
  while (*argv) {
    if (strcmp(*argv, "-v") == 0)
      verbose = 1;
    else if (strcmp(*argv, "-q") == 0)
      quiet = 1;
    argv++;
  }

  if (!g_session) {
    zwarnnam(MODNAME, "%s: session not initialized", BUILTIN_STARSHIP_STATS);
    return 1;
  }

  ssp_stats_t st;
  memset(&st, 0, sizeof(st));
  if (ssp_session_stats(g_session, &st) != 0) {
    report_ffi_error(BUILTIN_STARSHIP_STATS);
    return 1;
  }

  /* --- Set zsh integer parameters --- */
  char buf[32];
#define SET_INT(name, val)                                                     \
  do {                                                                         \
    snprintf(buf, sizeof(buf), "%llu", (unsigned long long)(val));             \
    setsparam((char *)(name), ztrdup(buf));                                    \
  } while (0)

  SET_INT("STARSHIP_STATS_CONFIG_HITS", st.config_hits);
  SET_INT("STARSHIP_STATS_CONFIG_MISSES", st.config_misses);
  SET_INT("STARSHIP_STATS_REPO_STATUS_HITS", st.repo_status_hits);
  SET_INT("STARSHIP_STATS_REPO_STATUS_MISSES", st.repo_status_misses);
  SET_INT("STARSHIP_STATS_GIT_REPO_HITS", st.git_repo_hits);
  SET_INT("STARSHIP_STATS_GIT_REPO_MISSES", st.git_repo_misses);
  SET_INT("STARSHIP_STATS_GIT_METRICS_HITS", st.git_metrics_hits);
  SET_INT("STARSHIP_STATS_GIT_METRICS_MISSES", st.git_metrics_misses);
  SET_INT("STARSHIP_STATS_DIR_CONTENTS_HITS", st.dir_contents_hits);
  SET_INT("STARSHIP_STATS_DIR_CONTENTS_MISSES", st.dir_contents_misses);
  SET_INT("STARSHIP_STATS_BINARY_PATH_HITS", st.binary_path_hits);
  SET_INT("STARSHIP_STATS_BINARY_PATH_MISSES", st.binary_path_misses);
  SET_INT("STARSHIP_STATS_RENDERS", st.renders);

#undef SET_INT

  /* --- Print summary --- */
  if (!quiet) {
    unsigned long long total_hits = st.config_hits + st.repo_status_hits +
                                    st.git_repo_hits + st.git_metrics_hits +
                                    st.dir_contents_hits + st.binary_path_hits;
    unsigned long long total_misses =
        st.config_misses + st.repo_status_misses + st.git_repo_misses +
        st.git_metrics_misses + st.dir_contents_misses + st.binary_path_misses;

    printf("starship_native session stats (%llu renders):\n", st.renders);
    printf("  total hits=%llu misses=%llu hit_rate=%.1f%%\n", total_hits,
           total_misses,
           (total_hits + total_misses) > 0
               ? 100.0 * (double)total_hits /
                     (double)(total_hits + total_misses)
               : 0.0);

    if (verbose) {
      printf("  config:        hits=%llu misses=%llu\n", st.config_hits,
             st.config_misses);
      printf("  repo_status:   hits=%llu misses=%llu\n", st.repo_status_hits,
             st.repo_status_misses);
      printf("  git_repo:      hits=%llu misses=%llu\n", st.git_repo_hits,
             st.git_repo_misses);
      printf("  git_metrics:   hits=%llu misses=%llu\n", st.git_metrics_hits,
             st.git_metrics_misses);
      printf("  dir_contents:  hits=%llu misses=%llu\n", st.dir_contents_hits,
             st.dir_contents_misses);
      printf("  binary_path:   hits=%llu misses=%llu\n", st.binary_path_hits,
             st.binary_path_misses);
    }
  }

  return 0;
}

/* ------------------------------------------------------------------ */
/* zsh module entry points                                            */
/* ------------------------------------------------------------------ */

/**/
int setup_(UNUSED(Module m)) { return 0; }

/**/
int features_(Module m, char ***features) {
  *features = featuresarray(m, &module_features);
  return 0;
}

/**/
int enables_(Module m, int **enables) {
  return handlefeatures(m, &module_features, enables);
}

/**/
int boot_(UNUSED(Module m)) {
  /* Create the persistent FFI session */
  g_session = ssp_session_create();
  if (!g_session) {
    char *err = NULL;
    ssp_last_error(&err);
    zwarnnam(MODNAME, "failed to create session: %s",
             err ? err : "unknown error");
    if (err) {
      ssp_free(err);
    }
    return 1;
  }
  return 0;
}

/**/
int cleanup_(Module m) {
  /* Shut down the thread pool before freeing the session */
  if (g_session) {
    ssp_session_shutdown(g_session);
    ssp_session_destroy(g_session);
    g_session = NULL;
  }

  /* Disable all features before teardown */
  return setfeatureenables(m, &module_features, NULL);
}

/**/
int finish_(UNUSED(Module m)) { return 0; }
