/*
 * ffi_smoke.c — Smoke test for the starship-ffi C API.
 *
 * Error convention exercised here: every fallible ssp_* call returns `char *`.
 * NULL means success; a non-NULL value is an error string the caller frees
 * with ssp_free().
 *
 * Build:
 *   gcc -O2 -o ffi_smoke ffi_smoke.c -ldl
 *
 * Usage:
 *   ./ffi_smoke [path/to/libstarship_ffi.so]
 */

#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct ssp_session ssp_session_t;

typedef struct {
  const char *status;
  const char *const *pipestatus;
  size_t pipestatus_len;
  size_t terminal_width;
  const char *path;
  const char *logical_path;
  const char *cmd_duration;
  const char *keymap;
  long long jobs;
  long long shlvl;
  int target;
} ssp_render_input_t;

typedef struct {
  unsigned long long config_hits, config_misses;
  unsigned long long repo_status_hits, repo_status_misses;
  unsigned long long git_repo_hits, git_repo_misses;
  unsigned long long git_metrics_hits, git_metrics_misses;
  unsigned long long dir_contents_hits, dir_contents_misses;
  unsigned long long binary_path_hits, binary_path_misses;
  unsigned long long renders;
} ssp_stats_t;

/* Function pointers loaded via dlopen */
static void *lib_handle = NULL;
static char *(*fn_session_create)(ssp_session_t **out);
static char *(*fn_session_destroy)(ssp_session_t *);
static char *(*fn_session_render)(ssp_session_t *, const ssp_render_input_t *,
                                  char **);
static void (*fn_free)(char *);
static const char *(*fn_version)(void);
static char *(*fn_session_stats)(ssp_session_t *, ssp_stats_t *);

static int passed = 0, failed = 0;

#define TEST(name) printf("  %-50s ", name)
#define PASS()                                                                 \
  do {                                                                         \
    printf("PASS\n");                                                          \
    passed++;                                                                  \
  } while (0)
#define FAIL(msg)                                                              \
  do {                                                                         \
    printf("FAIL: %s\n", msg);                                                 \
    failed++;                                                                  \
  } while (0)

static int load_library(const char *path) {
  lib_handle = dlopen(path, RTLD_NOW | RTLD_GLOBAL);
  if (!lib_handle) {
    fprintf(stderr, "dlopen: %s\n", dlerror());
    return -1;
  }
#define L(sym)                                                                 \
  fn_##sym = dlsym(lib_handle, "ssp_" #sym);                                   \
  if (!fn_##sym) {                                                             \
    fprintf(stderr, "dlsym(ssp_%s): %s\n", #sym, dlerror());                   \
    return -1;                                                                 \
  }
  L(session_create);
  L(session_destroy);
  L(session_render);
  L(free);
  L(version);
  L(session_stats);
#undef L
  return 0;
}

/* Create a session, aborting the current test on failure. */
static ssp_session_t *make_session(void) {
  ssp_session_t *session = NULL;
  char *err = fn_session_create(&session);
  if (err) {
    fprintf(stderr, "session_create: %s\n", err);
    fn_free(err);
    return NULL;
  }
  return session;
}

int main(int argc, char **argv) {
  const char *libpath =
      (argc > 1) ? argv[1] : "rust_src/target/release/libstarship_ffi.so";
  printf("Starship FFI Smoke Test\nLibrary: %s\n\n", libpath);
  if (load_library(libpath) != 0)
    return 1;

  /* Test: version */
  TEST("version");
  {
    const char *v = fn_version();
    if (v && strlen(v) > 0)
      PASS();
    else
      FAIL("bad version");
  }

  /* Test: create/destroy */
  TEST("create/destroy");
  {
    ssp_session_t *s = make_session();
    if (s) {
      char *err = fn_session_destroy(s);
      if (err) {
        FAIL(err);
        fn_free(err);
      } else {
        PASS();
      }
    } else {
      FAIL("create returned NULL");
    }
  }

  /* Test: destroy(NULL) */
  TEST("destroy(NULL)");
  {
    char *err = fn_session_destroy(NULL);
    if (err) {
      FAIL(err);
      fn_free(err);
    } else {
      PASS();
    }
  }

  /* Test: render main prompt */
  TEST("render main prompt");
  {
    ssp_session_t *s = make_session();
    if (!s) {
      FAIL("create returned NULL");
    } else {
      ssp_render_input_t in = {0};
      in.terminal_width = 80;
      in.target = 0;
      char *out = NULL;
      char *err = fn_session_render(s, &in, &out);
      if (!err && out && strlen(out) > 0) {
        PASS();
      } else {
        FAIL(err ? err : "render returned no output");
      }
      if (err)
        fn_free(err);
      fn_free(out);
      fn_session_destroy(s);
    }
  }

  /* Test: render right prompt */
  TEST("render right prompt");
  {
    ssp_session_t *s = make_session();
    if (!s) {
      FAIL("create returned NULL");
    } else {
      ssp_render_input_t in = {0};
      in.terminal_width = 80;
      in.target = 1;
      char *out = NULL;
      char *err = fn_session_render(s, &in, &out);
      if (!err)
        PASS();
      else
        FAIL(err);
      if (err)
        fn_free(err);
      fn_free(out);
      fn_session_destroy(s);
    }
  }

  /* Test: render null args returns an allocated error string */
  TEST("render null args (error expected)");
  {
    char *out = NULL;
    char *err = fn_session_render(NULL, NULL, &out);
    if (err && !out)
      PASS();
    else
      FAIL("expected an error and cleared *out");
    fn_free(err);
  }

  /* Test: errors are call-local, not stored anywhere */
  TEST("errors are call-local");
  {
    ssp_session_t *a = make_session();
    ssp_session_t *b = make_session();
    if (!a || !b) {
      FAIL("create returned NULL");
    } else {
      ssp_render_input_t in = {0};
      in.terminal_width = 80;
      in.target = 0;

      char *out = NULL;
      char *err_a = fn_session_render(a, NULL, &out);
      int ok = (err_a != NULL) && (out == NULL);
      fn_free(err_a);

      char *out_b = NULL;
      char *err_b = fn_session_render(b, &in, &out_b);
      ok = ok && (err_b == NULL) && (out_b != NULL);
      fn_free(err_b);
      fn_free(out_b);

      char *out_a = NULL;
      char *err_a2 = fn_session_render(a, &in, &out_a);
      ok = ok && (err_a2 == NULL) && (out_a != NULL);
      fn_free(err_a2);
      fn_free(out_a);

      if (ok)
        PASS();
      else
        FAIL("error handling is not call-local");
    }
    if (a)
      fn_session_destroy(a);
    if (b)
      fn_session_destroy(b);
  }

  /* Test: free(NULL) */
  TEST("free(NULL)");
  {
    fn_free(NULL);
    PASS();
  }

  /* Test: stats */
  TEST("stats");
  {
    ssp_session_t *s = make_session();
    if (!s) {
      FAIL("create returned NULL");
    } else {
      ssp_render_input_t in = {0};
      in.terminal_width = 80;
      in.target = 0;
      char *out = NULL;
      char *err = fn_session_render(s, &in, &out);
      fn_free(err);
      fn_free(out);

      ssp_stats_t st = {0};
      err = fn_session_stats(s, &st);
      if (!err && st.renders > 0)
        PASS();
      else
        FAIL(err ? err : "stats failed");
      fn_free(err);
      fn_session_destroy(s);
    }
  }

  /* Test: cache hit */
  TEST("cache hit across renders");
  {
    ssp_session_t *s = make_session();
    if (!s) {
      FAIL("create returned NULL");
    } else {
      ssp_render_input_t in = {0};
      in.terminal_width = 80;
      in.target = 0;
      char *o1 = NULL, *o2 = NULL;
      char *err1 = fn_session_render(s, &in, &o1);
      char *err2 = fn_session_render(s, &in, &o2);
      if (!err1 && !err2 && o1 && o2 && strcmp(o1, o2) == 0)
        PASS();
      else
        FAIL("outputs differ or render failed");
      fn_free(err1);
      fn_free(err2);
      fn_free(o1);
      fn_free(o2);
      fn_session_destroy(s);
    }
  }

  printf("\nResults: %d passed, %d failed\n", passed, failed);
  dlclose(lib_handle);
  return failed > 0 ? 1 : 0;
}
