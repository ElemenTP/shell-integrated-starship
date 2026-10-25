#!/usr/bin/env zsh
# zsh integration test for starship_native module.
#
# Usage:
#   MODULE_DIR=/path/to/zsh_src/build zsh test_zsh.sh

set -euo pipefail

MODULE_DIR="${MODULE_DIR:-$PWD/zsh_src/build}"

echo "=== zsh integration test ==="
echo "Module dir: $MODULE_DIR"

# Shell type MUST be set before first render (get_shell() OnceLock).
export STARSHIP_SHELL=zsh
# Avoid starship's TERM=dumb disabled fallback so the tests exercise a real prompt.
export TERM=xterm-256color

# Configure module search path
module_path=("$MODULE_DIR" $module_path)

# Attempt to load
if ! zmodload starship_native 2>/dev/null; then
    echo "FAIL: could not load starship_native from $MODULE_DIR"
    echo "  Available modules in path:"
    for d in $module_path; do
        ls -la "$d"/starship_native* 2>/dev/null || true
    done
    exit 1
fi
echo "PASS: module loaded"

# Version: the builtin should report the upstream Starship version embedded in
# the FFI. CMake passes STARSHIP_VERSION for an exact comparison; a plain run
# only checks the semver shape.
expected_starship_version="${STARSHIP_VERSION:-}"
starship_version -q
if [[ -n "$expected_starship_version" ]]; then
    if [[ "$STARSHIP_VERSION" != "$expected_starship_version" ]]; then
        echo "FAIL: starship_version '$STARSHIP_VERSION' != embedded version '$expected_starship_version'"
        zmodload -u starship_native 2>/dev/null || true
        exit 1
    fi
    echo "PASS: version $STARSHIP_VERSION (matches embedded starship)"
elif [[ "$STARSHIP_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
    echo "PASS: version $STARSHIP_VERSION"
else
    echo "FAIL: version '$STARSHIP_VERSION' is not a semver"
    zmodload -u starship_native 2>/dev/null || true
    exit 1
fi

# Render prompt
starship_prompt 2>/dev/null

# Check output
if [[ -n "${STARSHIP_PROMPT:-}" ]]; then
    echo "PASS: prompt rendered (${#STARSHIP_PROMPT} chars)"
else
    echo "FAIL: empty STARSHIP_PROMPT"
    zmodload -u starship_native 2>/dev/null || true
    exit 1
fi

# Verify right prompt and continuation prompt were set
if [[ -n "${STARSHIP_RPROMPT:-}" ]]; then
    echo "PASS: right prompt set"
fi
if [[ -n "${STARSHIP_PROMPT2:-}" ]]; then
    echo "PASS: continuation prompt set"
fi

# Unload
if zmodload -u starship_native 2>/dev/null; then
    echo "PASS: module unloaded"
else
    echo "FAIL: module unload failed"
    exit 1
fi

echo "=== All zsh tests passed ==="
