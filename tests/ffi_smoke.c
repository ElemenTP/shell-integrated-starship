/*
 * ffi_smoke.c — Smoke test for the starship-ffi C API.
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
static ssp_session_t *(*fn_session_create)(void);
static void (*fn_session_destroy)(ssp_session_t *);
static int (*fn_session_render)(ssp_session_t *, const ssp_render_input_t *,
                                char **);
static void (*fn_free)(char *);
static const char *(*fn_version)(void);
static const char *(*fn_last_error)(void);
static int (*fn_session_stats)(const ssp_session_t *, ssp_stats_t *);

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
  L(last_error);
  L(session_stats);
#undef L
  return 0;
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
    ssp_session_t *s = fn_session_create();
    if (s) {
      fn_session_destroy(s);
      PASS();
    } else
      FAIL("create returned NULL");
  }

  /* Test: destroy(NULL) */
  TEST("destroy(NULL)");
  {
    fn_session_destroy(NULL);
    PASS();
  }

  /* Test: render main prompt */
  TEST("render main prompt");
  {
    ssp_session_t *s = fn_session_create();
    if (!s) {
      FAIL("create returned NULL");
      goto next1;
    }
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 0;
    char *out = NULL;
    int rc = fn_session_render(s, &in, &out);
    if (rc == 0 && out && strlen(out) > 0) {
      PASS();
    } else
      FAIL("render failed");
    fn_free(out);
    fn_session_destroy(s);
  }
next1:

  /* Test: render right prompt */
  TEST("render right prompt");
  {
    ssp_session_t *s = fn_session_create();
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 1;
    char *out = NULL;
    int rc = fn_session_render(s, &in, &out);
    if (rc == 0)
      PASS();
    else
      FAIL("right render failed");
    fn_free(out);
    fn_session_destroy(s);
  }

  /* Test: render null args */
  TEST("render null args (error expected)");
  {
    char *out = NULL;
    int rc = fn_session_render(NULL, NULL, &out);
    if (rc < 0)
      PASS();
    else
      FAIL("should error");
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
    ssp_session_t *s = fn_session_create();
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 0;
    char *out = NULL;
    fn_session_render(s, &in, &out);
    fn_free(out);
    ssp_stats_t st = {0};
    int rc = fn_session_stats(s, &st);
    if (rc == 0 && st.renders > 0)
      PASS();
    else
      FAIL("stats failed");
    fn_session_destroy(s);
  }

  /* Test: cache hit */
  TEST("cache hit across renders");
  {
    ssp_session_t *s = fn_session_create();
    ssp_render_input_t in = {0};
    in.terminal_width = 80;
    in.target = 0;
    char *o1 = NULL, *o2 = NULL;
    fn_session_render(s, &in, &o1);
    fn_session_render(s, &in, &o2);
    if (o1 && o2 && strcmp(o1, o2) == 0)
      PASS();
    else
      FAIL("outputs differ");
    fn_free(o1);
    fn_free(o2);
    fn_session_destroy(s);
  }

  printf("\nResults: %d passed, %d failed\n", passed, failed);
  dlclose(lib_handle);
  return failed > 0 ? 1 : 0;
}
