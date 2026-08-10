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
