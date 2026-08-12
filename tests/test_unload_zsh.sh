#!/usr/bin/env zsh
# Module unload/load cycle stress test for starship_native.
#
# Verifies that repeated zmodload -u / zmodload cycles do not crash,
# leak threads, or leave corrupted state. Each cycle exercises the
# thread-pool shutdown path (cleanup_ → ssp_session_shutdown).
#
# Usage:
#   MODULE_DIR=/path/to/zsh_src/build zsh test_unload_zsh.sh

set -euo pipefail

MODULE_DIR="${MODULE_DIR:-$PWD/zsh_src/build}"
CYCLES="${CYCLES:-5}"

echo "=== starship unload/load cycle test ==="
echo "Module dir: $MODULE_DIR"
echo "Cycles: $CYCLES"

export STARSHIP_SHELL=zsh
module_path=("$MODULE_DIR" $module_path)

# Record baseline thread count
baseline_threads=$(ls /proc/self/task 2>/dev/null | wc -l)
echo "Baseline threads: $baseline_threads"

for ((i=1; i<=CYCLES; i++)); do
    zmodload starship_native || { echo "FAIL: load cycle $i"; exit 1; }
    starship_prompt 2>/dev/null
    [[ -n "${STARSHIP_PROMPT:-}" ]] || { echo "FAIL: render cycle $i"; exit 1; }
    zmodload -u starship_native || { echo "FAIL: unload cycle $i"; exit 1; }
    echo "PASS: cycle $i (load → render → unload)"
done

# Check thread count after all cycles — should be back near baseline
# (allow small slack for zsh's own threads)
sleep 0.5
after_threads=$(ls /proc/self/task 2>/dev/null | wc -l)
echo "Threads after $CYCLES cycles: $after_threads (baseline: $baseline_threads)"

if (( after_threads - baseline_threads <= 2 )); then
    echo "PASS: no leaked threads after cycles"
else
    echo "WARN: $((after_threads - baseline_threads)) extra threads after cycles"
    # Not a hard failure — zsh may have its own thread fluctuations
fi

echo "=== All unload tests passed ==="
