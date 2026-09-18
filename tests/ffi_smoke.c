/*
 * ffi_smoke.c — Smoke test for the starship-ffi C API.
 *
 * Error convention exercised here: every fallible ssp_* call returns `char *`.
 * NULL means success; a non-NULL value is an error string the caller frees
 * with ssp_free().
 *
 * The library owns exactly one global session per process. The test covers
 * create/destroy/render/stats, repeated initialization, and destroy→create.
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
static char *(*fn_init)(void);
static char *(*fn_shutdown)(void);
static char *(*fn_render)(const ssp_render_input_t *, char **);
static void (*fn_free)(char *);
static const char *(*fn_version)(void);
static char *(*fn_stats)(ssp_stats_t *);

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
  L(init);
  L(shutdown);
  L(render);
  L(free);
  L(version);
  L(stats);
#undef L
  return 0;
}

static void fail_with_error(const char *what, char *err) {
  fprintf(stderr, "  %s: %s\n", what, err ? err : "unknown error");
  fn_free(err);
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

  /* Test: no session yet */
  TEST("render before create (error expected)");
  {
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 0;
    char *out = NULL;
    char *err = fn_render(&in, &out);
    if (err && !out) {
      fn_free(err);
      PASS();
    } else {
      fail_with_error("expected error", err);
      fn_free(out);
      FAIL("render without session should error");
    }
  }

  /* Test: create */
  TEST("create");
  {
    char *err = fn_init();
    if (!err)
      PASS();
    else {
      fail_with_error("session_create", err);
      FAIL("create failed");
    }
  }

  /* Test: duplicate create is no-op */
  TEST("create twice (no error expected)");
  {
    char *err = fn_init();
    if (err)
      FAIL("second create should not return an error");
    else
      PASS();
  }

  /* Test: render main prompt */
  TEST("render main prompt");
  {
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 0;
    char *out = NULL;
    char *err = fn_render(&in, &out);
    if (!err && out && strlen(out) > 0)
      PASS();
    else {
      fail_with_error("render main", err);
      FAIL("render failed");
    }
    fn_free(out);
  }

  /* Test: render right prompt */
  TEST("render right prompt");
  {
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 1;
    char *out = NULL;
    char *err = fn_render(&in, &out);
    if (!err)
      PASS();
    else {
      fail_with_error("render right", err);
      FAIL("right render failed");
    }
    fn_free(out);
  }

  /* Test: render null args */
  TEST("render null args (error expected)");
  {
    char *out = NULL;
    char *err = fn_render(NULL, &out);
    if (err && !out) {
      fn_free(err);
      PASS();
    } else {
      fail_with_error("expected error", err);
      fn_free(out);
      FAIL("null input should error");
    }
  }

  /* Test: render null out-slot */
  TEST("render null out-slot (error expected)");
  {
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 0;
    char *err = fn_render(&in, NULL);
    if (err) {
      fn_free(err);
      PASS();
    } else {
      FAIL("null out should error");
    }
  }

  /* Test: stats */
  TEST("stats");
  {
    ssp_stats_t st = {0};
    char *err = fn_stats(&st);
    if (!err && st.renders > 0)
      PASS();
    else {
      fail_with_error("stats", err);
      FAIL("stats failed");
    }
  }

  /* Test: cache hit across renders */
  TEST("cache hit across renders");
  {
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 0;
    char *o1 = NULL, *o2 = NULL;
    char *err1 = fn_render(&in, &o1);
    char *err2 = fn_render(&in, &o2);
    if (!err1 && !err2 && o1 && o2 && strcmp(o1, o2) == 0)
      PASS();
    else {
      fail_with_error("first render", err1);
      fail_with_error("second render", err2);
      FAIL("outputs differ or render failed");
    }
    fn_free(o1);
    fn_free(o2);
  }

  /* Test: destroy */
  TEST("destroy");
  {
    char *err = fn_shutdown();
    if (!err)
      PASS();
    else {
      fail_with_error("shutdown", err);
      FAIL("destroy failed");
    }
  }

  /* Test: destroy is idempotent */
  TEST("destroy twice");
  {
    char *err = fn_shutdown();
    if (!err)
      PASS();
    else {
      fail_with_error("shutdown", err);
      FAIL("second destroy should succeed");
    }
  }

  /* Test: render after destroy fails */
  TEST("render after destroy (error expected)");
  {
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 0;
    char *out = NULL;
    char *err = fn_render(&in, &out);
    if (err && !out) {
      fn_free(err);
      PASS();
    } else {
      fail_with_error("expected error", err);
      fn_free(out);
      FAIL("render after destroy should error");
    }
  }

  /* Test: destroy then create yields a fresh session */
  TEST("destroy then create resets stats");
  {
    char *err = fn_init();
    if (err) {
      fail_with_error("session_create", err);
      FAIL("recreate failed");
    } else {
      ssp_stats_t st = {0};
      err = fn_stats(&st);
      if (err) {
        fail_with_error("stats", err);
        FAIL("stats after recreate failed");
      } else if (st.renders != 0) {
        FAIL("new session should start with zero renders");
      } else {
        PASS();
      }
    }
  }

  /* Test: render after recreate */
  TEST("render after recreate");
  {
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 0;
    char *out = NULL;
    char *err = fn_render(&in, &out);
    if (!err && out && strlen(out) > 0)
      PASS();
    else {
      fail_with_error("render after recreate", err);
      FAIL("render after recreate failed");
    }
    fn_free(out);
  }

  /* Test: free(NULL) */
  TEST("free(NULL)");
  {
    fn_free(NULL);
    PASS();
  }

  /* Final cleanup */
  fn_shutdown();

  printf("\nResults: %d passed, %d failed\n", passed, failed);
  dlclose(lib_handle);
  return failed > 0 ? 1 : 0;
}
