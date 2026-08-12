# starship-native.plugin.zsh — in-process Starship prompt for zsh.
#
# A plugin-manager friendly loader, compatible with oh-my-zsh, zinit, antigen,
# zplug, zgen, sheldon, ... — just point your plugin manager at this repository
# and the *.plugin.zsh naming convention makes it pick this file up.
#
# Before this plugin can do anything you must have built & installed the
# compiled module (see README):
#
#     cmake -B build -S .
#     cmake --build build --config Release
#     cmake --install build --config Release --prefix ~/.local
#
# Compiled module discovery order:
#   1. $STARSHIP_NATIVE_DIR           — explicit override
#   2. this script's own directory    — `cmake --install` layout (lib/zsh/)
#   3. common install prefixes        — ~/.local/lib/zsh, /usr/local/lib/zsh, ...
#
# The plugin mirrors the official starship/src/init/starship.zsh script,
# replacing `$(starship prompt ...)` with the in-process `starship_prompt`
# builtin (loaded via zmodload).

# Prevent double-loading
if (( ${+_STARSHIP_NATIVE_LOADED} )); then
  return 0
fi
typeset -g _STARSHIP_NATIVE_LOADED=1

# Locate this script's directory (works under any plugin manager).
0="${${ZERO:-${0:#$ZSH_ARGZERO}}:-${(%):-%N}}"
0="${${(M)0:#/*}:-$PWD/$0}"
typeset -g _STARSHIP_NATIVE_SCRIPT_DIR="${0:h}"

# ---- Compiled module discovery ---------------------------------------------
# Platform-specific file names.
case "${OSTYPE:-}" in
  darwin*)
    _starship_native_mods=(starship_native.dylib)
    _starship_native_ffis=(libstarship_ffi.dylib)
    ;;
  *)
    _starship_native_mods=(starship_native.so)
    _starship_native_ffis=(libstarship_ffi.so)
    ;;
esac

typeset -ga _starship_native_dirs
if [[ -n "${STARSHIP_NATIVE_DIR:-}" ]]; then
  _starship_native_dirs+=("$STARSHIP_NATIVE_DIR")
fi
_starship_native_dirs+=(
  "$_STARSHIP_NATIVE_SCRIPT_DIR"
  "${XDG_DATA_HOME:-$HOME/.local/share}/zsh"
  "$HOME/.local/lib/zsh"
  "/usr/local/lib/zsh"
  "/usr/lib/zsh"
  "/opt/starship-native/lib/zsh"
)

# STARSHIP_NATIVE_DIR is also the resolved module directory (exported so users
# can see where the module was loaded from); STARSHIP_NATIVE_FFI is the ffi lib
# when it is colocated with the module (optional — the module RPATH covers it).
typeset -g STARSHIP_NATIVE_DIR=""
typeset -g STARSHIP_NATIVE_FFI=""

typeset _dir _mod _ffi _found=0
for _dir in "${_starship_native_dirs[@]}"; do
  [[ -d "$_dir" ]] || continue
  for _mod in "${_starship_native_mods[@]}"; do
    if [[ -f "$_dir/$_mod" ]]; then
      STARSHIP_NATIVE_DIR="$_dir"
      _found=1
      for _ffi in "${_starship_native_ffis[@]}"; do
        if [[ -f "$_dir/$_ffi" ]]; then
          STARSHIP_NATIVE_FFI="$_dir/$_ffi"
          break
        fi
      done
      break
    fi
  done
  (( _found )) && break
done

if [[ -z "$STARSHIP_NATIVE_DIR" ]]; then
  print -u2 "starship-native: compiled module (starship_native) not found."
  print -u2 "  Build it first:"
  print -u2 "    cmake -B build -S . && cmake --build build --config Release"
  print -u2 "    cmake --install build --config Release --prefix \$HOME/.local"
  print -u2 "  Or point STARSHIP_NATIVE_DIR at the installed lib/zsh directory."
  unset _starship_native_dirs _starship_native_mods _starship_native_ffis
  return 1
fi

# ---- Load the native module ----
module_path=("$STARSHIP_NATIVE_DIR" $module_path)
zmodload starship_native || {
  print -u2 "starship-native: failed to load starship_native from $STARSHIP_NATIVE_DIR"
  return 1
}

# ---- Shell detection — MUST be set before first render (get_shell() OnceLock) ----
export STARSHIP_SHELL="zsh"

# ---- The rest is adapted verbatim from starship/src/init/starship.zsh ----

zmodload zsh/parameter  # Needed to access jobstates variable for STARSHIP_JOBS_COUNT

# Defines a function `__starship_get_time` that sets the time since epoch in millis in STARSHIP_CAPTURED_TIME.
if [[ $ZSH_VERSION == ([1-4]*) ]]; then
    # ZSH <= 5; Does not have a built-in variable so we will rely on Starship's inbuilt time function.
    __starship_get_time() {
        STARSHIP_CAPTURED_TIME=$(::STARSHIP:: time)
    }
else
    zmodload zsh/datetime
    zmodload zsh/mathfunc
    __starship_get_time() {
        (( STARSHIP_CAPTURED_TIME = int(rint(EPOCHREALTIME * 1000)) ))
    }
fi

# The two functions below follow the naming convention `prompt_<theme>_<hook>`
# for compatibility with Zsh's prompt system.

# Runs before each new command line.
prompt_starship_precmd() {
    # Save the status, because subsequent commands in this function will change $?
    STARSHIP_CMD_STATUS=$? STARSHIP_PIPE_STATUS=(${pipestatus[@]})

    # Calculate duration if a command was executed
    if (( ${+STARSHIP_START_TIME} )); then
        __starship_get_time && STARSHIP_DURATION=$(( STARSHIP_CAPTURED_TIME - STARSHIP_START_TIME ))
        unset STARSHIP_START_TIME
    # Drop status and duration otherwise
    else
        unset STARSHIP_DURATION STARSHIP_CMD_STATUS STARSHIP_PIPE_STATUS
    fi

    # Use length of jobstates array as number of jobs. Expansion fails inside
    # quotes so we set it here and then use the value later on.
    STARSHIP_JOBS_COUNT="${#jobstates[*]}"

    # ---- Native hook: render the prompt in-process ----
    starship_prompt
}

# Runs after the user submits the command line, but before it is executed and
# only if there's an actual command to run
prompt_starship_preexec() {
    __starship_get_time && STARSHIP_START_TIME=$STARSHIP_CAPTURED_TIME
}

# Add hook functions
autoload -Uz add-zsh-hook
add-zsh-hook precmd prompt_starship_precmd
add-zsh-hook preexec prompt_starship_preexec

# Set up a function to redraw the prompt if the user switches vi modes
starship_zle-keymap-select() {
    zle reset-prompt
}

## Check for existing keymap-select widget.
if [[ -v widgets[zle-keymap-select] ]]; then
    # zle-keymap-select is a special widget so it'll be "user:fnName" or nothing. Let's get fnName only.
    __starship_preserved_zle_keymap_select=${widgets[zle-keymap-select]#user:}
fi

if [[ -z ${__starship_preserved_zle_keymap_select:-} ]]; then
    zle -N zle-keymap-select starship_zle-keymap-select;
else
    # Define a wrapper fn to call the original widget fn and then Starship's.
    starship_zle-keymap-select-wrapped() {
        $__starship_preserved_zle_keymap_select "$@";
        starship_zle-keymap-select "$@";
    }
    zle -N zle-keymap-select starship_zle-keymap-select-wrapped;
fi

# Set up the session key that will be used to store logs
STARSHIP_SESSION_KEY="$RANDOM$RANDOM$RANDOM$RANDOM$RANDOM"; # Random generates a number b/w 0 - 32767
STARSHIP_SESSION_KEY="${STARSHIP_SESSION_KEY}0000000000000000" # Pad it to 16+ chars.
export STARSHIP_SESSION_KEY=${STARSHIP_SESSION_KEY:0:16}; # Trim to 16-digits if excess.

VIRTUAL_ENV_DISABLE_PROMPT=1

setopt promptsubst

# Use the params written by the starship_prompt builtin.
PROMPT='$STARSHIP_PROMPT'
RPROMPT='$STARSHIP_RPROMPT'
PROMPT2='$STARSHIP_PROMPT2'
