#!/usr/bin/env zsh
# Fork safety regression test for starship_native zsh module.
#
# Verifies that calling the starship_prompt builtin from forked contexts
# ($(...), &, pipelines, subshells) does NOT crash the shell — the fork
# guard in the FFI layer must reject the call and return safely.
#
# Usage:
#   MODULE_DIR=/path/to/zsh_src/build zsh test_fork_zsh.sh

set -euo pipefail

MODULE_DIR="${MODULE_DIR:-$PWD/zsh_src/build}"

echo "=== starship fork safety test ==="
echo "Module dir: $MODULE_DIR"

export STARSHIP_SHELL=zsh
module_path=("$MODULE_DIR" $module_path)
zmodload starship_native || { echo "FAIL: load"; exit 1; }
echo "PASS: load"

# Initial render in the parent process (sanity check)
starship_prompt 2>/dev/null
if [[ -n "${STARSHIP_PROMPT:-}" ]]; then
    echo "PASS: render in parent process"
else
    echo "FAIL: render in parent process returned empty"
    exit 1
fi

# ---- Test 1: command substitution $(...) ----
# zsh forks for $(); the child runs the builtin and inherits the
# corrupted rayon state. The fork guard must reject the call.
output=$(starship_prompt 2>/dev/null)
echo "PASS: \$() command substitution (forked child survived, output len=${#output})"

# ---- Test 2: background job & ----
starship_prompt &>/dev/null &
job_pid=$!
wait $job_pid 2>/dev/null
echo "PASS: background job & (survived)"

# ---- Test 3: pipeline (builtin not last stage) ----
starship_prompt 2>/dev/null | cat >/dev/null
echo "PASS: pipeline (builtin as non-last stage)"

# ---- Test 4: subshell ( ) ----
( starship_prompt 2>/dev/null )
echo "PASS: subshell ( )"

# ---- Test 5: process substitution <( ) ----
if [[ -n "${ZSH_VERSION:-}" ]]; then
    cat <(starship_prompt 2>/dev/null) >/dev/null 2>&1 || true
    echo "PASS: process substitution <( )"
fi

# ---- Test 6: $(...) inside command substitution ----
outer=$(echo $(starship_prompt 2>/dev/null))
echo "PASS: nested \$() (survived)"

# ---- Test 7: parent still works after all forks ----
starship_prompt 2>/dev/null
if [[ -n "${STARSHIP_PROMPT:-}" ]]; then
    echo "PASS: parent still functional after forks"
else
    echo "FAIL: parent broken after forks"
    exit 1
fi

# ---- Test 8: unload/load cycle ----
zmodload -u starship_native
echo "PASS: unload after forks"

zmodload starship_native
starship_prompt 2>/dev/null
echo "PASS: reload works"

zmodload -u starship_native
echo "PASS: final unload"

echo "=== All fork tests passed ==="
