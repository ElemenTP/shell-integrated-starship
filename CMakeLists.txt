cmake_minimum_required(VERSION 3.21)
project(starship-native
    VERSION 0.1.0
    DESCRIPTION "Starship prompt as native shell plugins (zsh + pwsh)"
    LANGUAGES C)

# Always generate compile_commands.json for tooling (LSP, clangd, etc.)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# ── Global options ────────────────────────────────────────────────────────
option(BUILD_ZSH_MODULE   "Build the zsh loadable module (Linux/macOS)"     ON)
option(BUILD_PWSH_MODULE  "Build the PowerShell binary module"              ON)
option(BUILD_TESTS        "Build test executables"                          ON)

# Starship source: "AUTO" (clone from GitHub) or a local path.
set(STARSHIP_SOURCE "AUTO" CACHE STRING
    "Starship source: 'AUTO' to clone from GitHub, or a local path")
# zsh source: "AUTO" (download zsh 5.9.2) or a local path to configured tree.
set(ZSH_SOURCE "AUTO" CACHE STRING
    "zsh source: 'AUTO' to download zsh 5.9.2, or path to configured source tree")
set(ZSH_VERSION "5.9.2" CACHE STRING "zsh version to download (when ZSH_SOURCE=AUTO)")

include(FetchContent)

# ── Platform detection ────────────────────────────────────────────────────
get_property(_is_multi_config GLOBAL PROPERTY GENERATOR_IS_MULTI_CONFIG)
if(_is_multi_config)
    message(STATUS "Multi-config generator detected — use --config <Release|Debug>")
endif()

# ── Find required tools ───────────────────────────────────────────────────
find_program(CARGO cargo REQUIRED)
find_program(DOTNET dotnet)

if(BUILD_PWSH_MODULE AND NOT DOTNET)
    message(WARNING "dotnet not found — PowerShell module will not be built")
    set(BUILD_PWSH_MODULE OFF)
endif()

# ══════════════════════════════════════════════════════════════════════════
# Starship source
# ══════════════════════════════════════════════════════════════════════════
set(STARSHIP_LINK "${CMAKE_SOURCE_DIR}/starship")
if(STARSHIP_SOURCE STREQUAL "AUTO")
    message(STATUS "Fetching starship from GitHub (in-process branch)...")
    FetchContent_Declare(starship-src
        GIT_REPOSITORY "https://github.com/ElemenTP/starship"
        GIT_TAG        in-process
        GIT_SHALLOW    TRUE
    )
    FetchContent_MakeAvailable(starship-src)
    set(STARSHIP_SRC_DIR "${starship-src_SOURCE_DIR}")
else()
    get_filename_component(STARSHIP_SRC_DIR "${STARSHIP_SOURCE}" ABSOLUTE)
    if(NOT EXISTS "${STARSHIP_SRC_DIR}/Cargo.toml")
        message(FATAL_ERROR "STARSHIP_SOURCE=${STARSHIP_SOURCE} does not contain Cargo.toml")
    endif()
endif()
message(STATUS "Starship source: ${STARSHIP_SRC_DIR}")

# Create symlink so rust_src/Cargo.toml path dependency resolves correctly.
# rust_src/Cargo.toml expects: starship = { path = "../starship" }
# Guard against self-referencing symlinks (STARSHIP_SRC_DIR == STARSHIP_LINK).
get_filename_component(_starship_real "${STARSHIP_LINK}" REALPATH)
if(NOT _starship_real STREQUAL "${STARSHIP_SRC_DIR}")
    file(REMOVE "${STARSHIP_LINK}")
    file(CREATE_LINK "${STARSHIP_SRC_DIR}" "${STARSHIP_LINK}" SYMBOLIC)
    message(STATUS "Starship symlink: ${STARSHIP_LINK} -> ${STARSHIP_SRC_DIR}")
endif()

# ══════════════════════════════════════════════════════════════════════════
# zsh source (for building the zsh module — headers only)
# ══════════════════════════════════════════════════════════════════════════
if(BUILD_ZSH_MODULE)
    if(ZSH_SOURCE STREQUAL "AUTO")
        message(STATUS "Fetching zsh ${ZSH_VERSION}...")
        # Official zsh distribution tarball (includes pre-generated configure).
        FetchContent_Declare(zsh-src
            URL "https://sourceforge.net/projects/zsh/files/zsh/${ZSH_VERSION}/zsh-${ZSH_VERSION}.tar.xz/download"
            URL_HASH SHA256=36fa734374b44783582cec09bcd67822e2f992c779ec1624ab5596df078d2f81
        )
        FetchContent_MakeAvailable(zsh-src)
        set(ZSH_SRC_DIR "${zsh-src_SOURCE_DIR}")

        # Auto-configure zsh to generate headers (config.h, zsh.mdh, *.epro).
        set(ZSH_CONFIGURE_STAMP "${zsh-src_BINARY_DIR}/.configured")
        if(NOT EXISTS "${ZSH_CONFIGURE_STAMP}")
            message(STATUS "Configuring zsh (generating headers)...")
            execute_process(
                COMMAND ./configure --disable-gdbm --disable-pcre
                WORKING_DIRECTORY "${ZSH_SRC_DIR}"
                RESULT_VARIABLE _zsh_conf_rc
                OUTPUT_QUIET
            )
            if(NOT _zsh_conf_rc EQUAL 0)
                message(FATAL_ERROR "zsh configure failed (run manually in ${ZSH_SRC_DIR})")
            endif()
            # Generate Makemod and zsh.mdh / *.epro (needed to include "zsh.mdh")
            execute_process(
                COMMAND ${CMAKE_COMMAND} -E env "top_srcdir=${ZSH_SRC_DIR}"
                    bash "${ZSH_SRC_DIR}/Src/mkmakemod.sh" Src Makemod
                WORKING_DIRECTORY "${ZSH_SRC_DIR}"
                RESULT_VARIABLE _zsh_mkm_rc
                OUTPUT_QUIET
            )
            if(NOT _zsh_mkm_rc EQUAL 0)
                message(FATAL_ERROR "zsh mkmakemod.sh failed")
            endif()
            execute_process(
                COMMAND make -f Makemod zsh.mdh
                WORKING_DIRECTORY "${ZSH_SRC_DIR}/Src"
                RESULT_VARIABLE _zsh_mdh_rc
                OUTPUT_QUIET
            )
            if(NOT _zsh_mdh_rc EQUAL 0)
                message(WARNING "make zsh.mdh had errors (headers may be incomplete)")
            endif()
            file(TOUCH "${ZSH_CONFIGURE_STAMP}")
            message(STATUS "zsh headers generated in ${ZSH_SRC_DIR}/Src")
        endif()
    else()
        get_filename_component(ZSH_SRC_DIR "${ZSH_SOURCE}" ABSOLUTE)
    endif()

    if(NOT EXISTS "${ZSH_SRC_DIR}/Src/zsh.mdh")
        message(WARNING "zsh.mdh not found at ${ZSH_SRC_DIR}/Src/ — "
                        "set ZSH_SOURCE to a configured zsh source tree or use AUTO")
        set(BUILD_ZSH_MODULE OFF)
    else()
        set(ZSH_SRC "${ZSH_SRC_DIR}/Src")
        message(STATUS "zsh headers: ${ZSH_SRC}")
    endif()
endif()

# ── Rust FFI crate ─────────────────────────────────────────────────────────
set(RUST_SRC_DIR "${CMAKE_SOURCE_DIR}/rust_src")
set(CARGO_RELEASE_FLAG "$<IF:$<CONFIG:Release>,--release,>")
set(CARGO_PROFILE_DIR "$<IF:$<CONFIG:Release>,release,debug>")

set(FFI_NAME "${CMAKE_SHARED_LIBRARY_PREFIX}starship_ffi${CMAKE_SHARED_LIBRARY_SUFFIX}")
set(FFI_SHARED "${RUST_SRC_DIR}/target/${CARGO_PROFILE_DIR}/${FFI_NAME}")

# Build starship-ffi via a real OUTPUT rule. A plain add_custom_target has no
# up-to-date check and would re-run `cargo build` on EVERY build; by declaring
# the .so as an OUTPUT, Ninja only re-runs cargo when the library is missing or
# one of the inputs below changed.
set(STARSHIP_FFI_DEPENDS
    "${RUST_SRC_DIR}/src"
    "${RUST_SRC_DIR}/Cargo.toml"
    "${RUST_SRC_DIR}/Cargo.lock"
)
if(EXISTS "${STARSHIP_SRC_DIR}/src")
    list(APPEND STARSHIP_FFI_DEPENDS
        "${STARSHIP_SRC_DIR}/src"
        "${STARSHIP_SRC_DIR}/Cargo.toml"
        "${STARSHIP_SRC_DIR}/build.rs")
endif()

# On macOS, rustc's default LC_ID_DYLIB is the ABSOLUTE cargo target path,
# which gets baked into any consumer (starship_native.so) as its LC_LOAD_DYLIB,
# leaking the build machine's path into the artifact. Rewrite it to a
# relocatable @rpath reference right after `cargo build` so the zsh module
# resolves the ffi library through its own LC_RPATH (set to @loader_path at
# install time) instead of an absolute path.
if(APPLE)
    set(STARSHIP_FFI_FIX
        COMMAND install_name_tool -id
            "@rpath/${FFI_NAME}" "${FFI_SHARED}")
else()
    set(STARSHIP_FFI_FIX "")
endif()

add_custom_command(
    OUTPUT "${FFI_SHARED}"
    COMMAND ${CARGO} build ${CARGO_RELEASE_FLAG}
    ${STARSHIP_FFI_FIX}
    WORKING_DIRECTORY "${RUST_SRC_DIR}"
    DEPENDS ${STARSHIP_FFI_DEPENDS}
    COMMENT "Building starship-ffi (Rust crate, $<CONFIG>)..."
    USES_TERMINAL
)

add_custom_target(starship_ffi ALL DEPENDS "${FFI_SHARED}")

# ── zsh module ─────────────────────────────────────────────────────────────
if(BUILD_ZSH_MODULE)
    set(ZSH_MODULE_NAME "starship_native")

    add_library(${ZSH_MODULE_NAME} SHARED
        "${CMAKE_SOURCE_DIR}/zsh_src/module.c"
    )

    target_include_directories(${ZSH_MODULE_NAME} PRIVATE
        "${ZSH_SRC}"
        "${ZSH_SRC}/Zle"
        "${ZSH_SRC}/Modules"
        "${ZSH_SRC_DIR}"  # for config.h
    )

    target_compile_definitions(${ZSH_MODULE_NAME} PRIVATE
        IMPORTING_MODULE_zshQsmain=1
    )

    target_compile_options(${ZSH_MODULE_NAME} PRIVATE
        -fPIC -O3 -flto -Wall -Wextra
    )

    add_dependencies(${ZSH_MODULE_NAME} starship_ffi)
    target_link_libraries(${ZSH_MODULE_NAME} PRIVATE
        "${FFI_SHARED}"
        pthread dl m
    )

    if(APPLE)
        target_link_options(${ZSH_MODULE_NAME} PRIVATE
            -Wl,-undefined,dynamic_lookup
        )
    else()
        target_link_options(${ZSH_MODULE_NAME} PRIVATE
            -Wl,--allow-shlib-undefined -Wl,-z,lazy
        )
    endif()

    # zsh hardcodes its loadable-module suffix to ".so" on EVERY platform
    # (DL_EXT=so in zsh's configure.ac), including macOS. Even though this
    # target is a Mach-O dylib on macOS, it MUST be named starship_native.so
    # or `zmodload` will refuse to find it.
    set_target_properties(${ZSH_MODULE_NAME} PROPERTIES
        LIBRARY_OUTPUT_DIRECTORY "${CMAKE_SOURCE_DIR}/zsh_src/build"
        PREFIX ""
        SUFFIX ".so"
        BUILD_RPATH "${RUST_SRC_DIR}/target/$<IF:$<CONFIG:Release>,release,debug>"
    )
    if(APPLE)
        # macOS has no $ORIGIN. @loader_path makes the installed module resolve
        # libstarship_ffi.dylib in its own directory (the ffi dylib is
        # installed alongside it under lib/zsh/starship-native).
        set_target_properties(${ZSH_MODULE_NAME} PROPERTIES
            INSTALL_RPATH "@loader_path"
        )
    else()
        set_target_properties(${ZSH_MODULE_NAME} PROPERTIES
            INSTALL_RPATH "$ORIGIN"
        )
    endif()

    if(APPLE)
        add_custom_command(TARGET ${ZSH_MODULE_NAME} POST_BUILD
            COMMAND ${CMAKE_STRIP} -u "$<TARGET_FILE:${ZSH_MODULE_NAME}>"
            COMMAND_EXPAND_LISTS
        )
    else()
        add_custom_command(TARGET ${ZSH_MODULE_NAME} POST_BUILD
            COMMAND ${CMAKE_STRIP} "$<TARGET_FILE:${ZSH_MODULE_NAME}>"
            COMMAND_EXPAND_LISTS
        )
    endif()

    if(UNIX)
        install(TARGETS ${ZSH_MODULE_NAME}
            LIBRARY DESTINATION "lib/zsh/starship-native"
        )
        install(FILES "${FFI_SHARED}"
            DESTINATION "lib/zsh/starship-native"
        )
        # The canonical plugin file lives at the repo root so zsh plugin
        # managers (oh-my-zsh, zinit, antigen, zplug, ...) can pick it up;
        # the same file is installed here for direct `source` usage.
        install(FILES "${CMAKE_SOURCE_DIR}/zsh_src/starship-native.plugin.zsh"
            DESTINATION "lib/zsh/starship-native"
        )
    endif()
endif()

# ── PowerShell module ──────────────────────────────────────────────────────
if(BUILD_PWSH_MODULE)
    set(PWSH_PROJECT "${CMAKE_SOURCE_DIR}/pwsh_src/StarshipNative/StarshipNative.csproj")
    set(PWSH_OUT_DIR "${CMAKE_SOURCE_DIR}/pwsh_src/StarshipNative/bin/$<CONFIG>/net8.0")

    add_custom_target(starship_pwsh ALL
        COMMAND ${DOTNET} build -c "$<CONFIG>" "${PWSH_PROJECT}"
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
            "${FFI_SHARED}"
            "${PWSH_OUT_DIR}/"
        COMMENT "Building StarshipNative (C# + P/Invoke, $<CONFIG>)..."
        DEPENDS starship_ffi
        USES_TERMINAL
    )

    install(DIRECTORY "${PWSH_OUT_DIR}/"
        DESTINATION "share/pwsh/modules/starship-native"
        FILES_MATCHING
        PATTERN "*.dll"
        PATTERN "*.psd1"
        PATTERN "*.psm1"
        PATTERN "*.so"
        PATTERN "*.dylib"
    )
    # The manifest (.psd1) and script module (.psm1) live in pwsh_src/ and are
    # copied into the dotnet output dir by the csproj (CopyToOutputDirectory),
    # so the install rule above picks them up automatically. No extra install
    # of pwsh_src files is needed.
endif()

# ── Tests ──────────────────────────────────────────────────────────────────
if(BUILD_TESTS)
    add_executable(ffi_smoke "${CMAKE_SOURCE_DIR}/tests/ffi_smoke.c")
    target_link_libraries(ffi_smoke PRIVATE ${CMAKE_DL_LIBS})
    add_dependencies(ffi_smoke starship_ffi)

    enable_testing()
    add_test(NAME ffi_smoke
        COMMAND ffi_smoke "${FFI_SHARED}"
        WORKING_DIRECTORY "${CMAKE_BINARY_DIR}"
    )

    add_custom_target(test-rust
        COMMAND ${CARGO} test ${CARGO_RELEASE_FLAG}
        WORKING_DIRECTORY "${RUST_SRC_DIR}"
        USES_TERMINAL
    )

    if(BUILD_ZSH_MODULE AND TARGET ${ZSH_MODULE_NAME})
        add_custom_target(test-zsh
            COMMAND ${CMAKE_COMMAND} -E env
                "MODULE_DIR=${CMAKE_SOURCE_DIR}/zsh_src/build/$<CONFIG>"
                zsh "${CMAKE_SOURCE_DIR}/tests/test_zsh.sh"
            DEPENDS ${ZSH_MODULE_NAME}
            USES_TERMINAL
        )
    endif()

    if(BUILD_PWSH_MODULE)
        add_custom_target(test-pwsh
            COMMAND ${CMAKE_COMMAND} -E env
                "DLL_DIR=${PWSH_OUT_DIR}"
                pwsh -NoProfile -File "${CMAKE_SOURCE_DIR}/tests/test_pwsh.ps1"
            DEPENDS starship_pwsh
            USES_TERMINAL
        )
    endif()

    add_custom_target(check
        COMMAND ${CMAKE_CTEST_COMMAND} --output-on-failure -C "$<CONFIG>"
        DEPENDS ffi_smoke
    )
endif()

# ── Print configuration summary ────────────────────────────────────────────
message(STATUS "")
message(STATUS "starship-native ${PROJECT_VERSION} — Build Configuration")
message(STATUS "──────────────────────────────────────────────────")
message(STATUS "Generator:         ${CMAKE_GENERATOR}")
message(STATUS "Starship source:   ${STARSHIP_SRC_DIR}")
message(STATUS "Zsh source:        ${ZSH_SRC_DIR}")
message(STATUS "Zsh module:        ${BUILD_ZSH_MODULE}")
message(STATUS "PowerShell module: ${BUILD_PWSH_MODULE}")
message(STATUS "Tests:             ${BUILD_TESTS}")
message(STATUS "Cargo:             ${CARGO}")
message(STATUS "dotnet:            ${DOTNET}")
message(STATUS "compile_commands:  ${CMAKE_EXPORT_COMPILE_COMMANDS}")
message(STATUS "")
message(STATUS "Build with: cmake --build <dir> --config <Release|Debug>")
message(STATUS "")