cmake_minimum_required(VERSION 3.21)

project(sw-explorer LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

find_package(Qt6 6.8 REQUIRED COMPONENTS Widgets)

qt_standard_project_setup()

# ---------------------------------------------------------------------------
# Workspace layout
# ---------------------------------------------------------------------------

get_filename_component(SW_WORKSPACE_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
set(SW_BRIDGE_CRATE_DIR "${SW_WORKSPACE_ROOT}/crates/sw-gui-bridge")
set(SW_BRIDGE_RS "${SW_BRIDGE_CRATE_DIR}/src/bridge.rs")

# ---------------------------------------------------------------------------
# Rust staticlib profile
#
# The Rust profile follows the C++ build type. Multi-config generators
# cannot express per-config custom-command outputs, so they always link
# the release staticlib; configure with Release in mind there.
# ---------------------------------------------------------------------------

if(CMAKE_CONFIGURATION_TYPES)
    set(SW_CARGO_PROFILE "release")
elseif(CMAKE_BUILD_TYPE STREQUAL "Debug")
    set(SW_CARGO_PROFILE "debug")
else()
    set(SW_CARGO_PROFILE "release")
endif()

if(SW_CARGO_PROFILE STREQUAL "release")
    set(SW_CARGO_PROFILE_ARGS --release)
else()
    set(SW_CARGO_PROFILE_ARGS "")
endif()

# The user-level Cargo bin directory is a standard install location for
# cxxbridge; look there in addition to PATH.
if(WIN32)
    set(SW_CARGO_BIN_HINT "$ENV{USERPROFILE}/.cargo/bin")
else()
    set(SW_CARGO_BIN_HINT "$ENV{HOME}/.cargo/bin")
endif()

find_program(CARGO_EXECUTABLE NAMES cargo HINTS "${SW_CARGO_BIN_HINT}" REQUIRED)
find_program(CXXBRIDGE_EXECUTABLE NAMES cxxbridge HINTS "${SW_CARGO_BIN_HINT}" REQUIRED)

# CXX requires the standalone cxxbridge-cmd to match the Rust cxx
# crate version exactly; fail at configure time with a clear message
# instead of producing mismatched generated code.
set(SW_CXX_VERSION "1.0.199")

execute_process(
    COMMAND "${CXXBRIDGE_EXECUTABLE}" --version
    RESULT_VARIABLE SW_CXXBRIDGE_VERSION_RESULT
    OUTPUT_VARIABLE SW_CXXBRIDGE_VERSION
    ERROR_VARIABLE SW_CXXBRIDGE_VERSION_ERROR
    OUTPUT_STRIP_TRAILING_WHITESPACE
)
if(NOT SW_CXXBRIDGE_VERSION_RESULT EQUAL 0)
    message(FATAL_ERROR
        "Failed to query cxxbridge version: ${SW_CXXBRIDGE_VERSION_ERROR}")
endif()

if(NOT SW_CXXBRIDGE_VERSION STREQUAL "cxxbridge ${SW_CXX_VERSION}")
    message(FATAL_ERROR
        "cxxbridge version mismatch: expected ${SW_CXX_VERSION}, "
        "got '${SW_CXXBRIDGE_VERSION}'")
endif()

# ---------------------------------------------------------------------------
# CXX bridge code generation (standalone cxxbridge-cmd, build time)
#
# cxxbridge emits self-contained sources, so the outputs can carry the
# plain names below. Everything lands in the build tree; nothing
# generated is committed.
# ---------------------------------------------------------------------------

set(SW_GENERATED_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated")
set(SW_GENERATED_HEADER "${SW_GENERATED_DIR}/sw_gui_bridge.h")
set(SW_GENERATED_SOURCE "${SW_GENERATED_DIR}/sw_gui_bridge.cc")
set(SW_GENERATED_CXXH "${SW_GENERATED_DIR}/rust/cxx.h")

add_custom_command(
    OUTPUT "${SW_GENERATED_HEADER}" "${SW_GENERATED_SOURCE}" "${SW_GENERATED_CXXH}"
    COMMAND "${CMAKE_COMMAND}" -E make_directory "${SW_GENERATED_DIR}/rust"
    COMMAND "${CXXBRIDGE_EXECUTABLE}" "${SW_BRIDGE_RS}" --header -o "${SW_GENERATED_HEADER}"
    COMMAND "${CXXBRIDGE_EXECUTABLE}" "${SW_BRIDGE_RS}" -o "${SW_GENERATED_SOURCE}"
    COMMAND "${CXXBRIDGE_EXECUTABLE}" --header -o "${SW_GENERATED_CXXH}"
    DEPENDS "${SW_BRIDGE_RS}"
    COMMENT "Generating CXX bridge sources"
    VERBATIM
)

# ---------------------------------------------------------------------------
# Rust staticlib build
# ---------------------------------------------------------------------------

if(WIN32)
    set(SW_BRIDGE_LIB_NAME "sw_gui_bridge.lib")
else()
    set(SW_BRIDGE_LIB_NAME "libsw_gui_bridge.a")
endif()
set(SW_BRIDGE_LIB "${SW_WORKSPACE_ROOT}/target/${SW_CARGO_PROFILE}/${SW_BRIDGE_LIB_NAME}")

file(GLOB_RECURSE SW_RUST_SOURCES CONFIGURE_DEPENDS
    "${SW_WORKSPACE_ROOT}/crates/sw-core/src/*.rs"
    "${SW_BRIDGE_CRATE_DIR}/src/*.rs"
)

add_custom_command(
    OUTPUT "${SW_BRIDGE_LIB}"
    COMMAND "${CARGO_EXECUTABLE}" build --package sw-gui-bridge --locked ${SW_CARGO_PROFILE_ARGS}
    WORKING_DIRECTORY "${SW_WORKSPACE_ROOT}"
    DEPENDS
        ${SW_RUST_SOURCES}
        "${SW_WORKSPACE_ROOT}/Cargo.toml"
        "${SW_WORKSPACE_ROOT}/Cargo.lock"
        "${SW_BRIDGE_CRATE_DIR}/Cargo.toml"
    COMMENT "Building Rust staticlib sw_gui_bridge (${SW_CARGO_PROFILE})"
    VERBATIM
)

add_custom_target(sw_gui_bridge_staticlib DEPENDS "${SW_BRIDGE_LIB}")

add_library(sw_gui_bridge STATIC IMPORTED GLOBAL)
set_target_properties(sw_gui_bridge PROPERTIES IMPORTED_LOCATION "${SW_BRIDGE_LIB}")

# ---------------------------------------------------------------------------
# Native libraries required by the Rust staticlib
#
# Ask rustc itself (--print native-static-libs) instead of hard-coding
# flags that only happen to work on one machine. The note is emitted on
# stderr, e.g.:
#   note: native-static-libs: -lgcc_s -lutil -lrt -lpthread -lm -ldl -lc
#   note: native-static-libs: kernel32.lib advapi32.lib ...
# ---------------------------------------------------------------------------

# Force color off: with CARGO_TERM_COLOR=always the note arrives
# wrapped in ANSI escape sequences and the final token ends up
# contaminated ('-lc<ESC>[0m'), which both defeats parsing and
# produces "cannot find -lc" style link errors.
execute_process(
    COMMAND "${CMAKE_COMMAND}" -E env CARGO_TERM_COLOR=never
            "${CARGO_EXECUTABLE}" rustc --package sw-gui-bridge --locked
            ${SW_CARGO_PROFILE_ARGS} -- --print native-static-libs
    WORKING_DIRECTORY "${SW_WORKSPACE_ROOT}"
    RESULT_VARIABLE SW_NATIVE_LIBS_RESULT
    OUTPUT_VARIABLE SW_NATIVE_LIBS_STDOUT
    ERROR_VARIABLE SW_NATIVE_LIBS_STDERR
)
if(NOT SW_NATIVE_LIBS_RESULT EQUAL 0)
    message(FATAL_ERROR
        "cargo rustc --print native-static-libs failed:\n${SW_NATIVE_LIBS_STDERR}")
endif()

string(REGEX MATCH "native-static-libs: ([^\n]+)" SW_NATIVE_LIBS_MATCH "${SW_NATIVE_LIBS_STDERR}")
if(NOT SW_NATIVE_LIBS_MATCH)
    message(FATAL_ERROR
        "Could not find the native-static-libs note in cargo output:\n${SW_NATIVE_LIBS_STDERR}")
endif()

string(STRIP "${CMAKE_MATCH_1}" SW_NATIVE_LIBS_RAW)
separate_arguments(SW_NATIVE_LIB_TOKENS UNIX_COMMAND "${SW_NATIVE_LIBS_RAW}")

# The final link always goes through the C++ compiler driver, which
# already links the C and C++ runtime libraries implicitly. rustc
# lists them anyway because its note targets linkers that lack a
# runtime. Repeating them is at best a duplicate-library warning
# (Apple ld) and at worst a hard error on toolchains where those
# standalone link names do not exist: newer Apple SDKs ship no libm
# linkable as -lm, and MSVC expresses the CRT as /defaultlib:
# directives instead of import libraries.
set(SW_DRIVER_RUNTIME_LIBS c m stdc++ c++ System)

set(SW_NATIVE_LIBS "")
set(SW_NATIVE_LIB_DIRS "")
set(SW_EXPECT_FRAMEWORK OFF)
foreach(token IN LISTS SW_NATIVE_LIB_TOKENS)
    string(TOLOWER "${token}" SW_TOKEN_LOWER)
    if(SW_EXPECT_FRAMEWORK)
        list(APPEND SW_NATIVE_LIBS "-framework ${token}")
        set(SW_EXPECT_FRAMEWORK OFF)
    elseif(token STREQUAL "-framework")
        set(SW_EXPECT_FRAMEWORK ON)
    elseif(SW_TOKEN_LOWER MATCHES "^/defaultlib:")
        # MSVC CRT selection pragma, already handled by the C++
        # runtime flags of the driver.
    elseif(token MATCHES "^-l(.+)" AND CMAKE_MATCH_1 IN_LIST SW_DRIVER_RUNTIME_LIBS)
        # Provided implicitly by the C++ compiler driver.
    elseif(token MATCHES "^-l(.+)")
        list(APPEND SW_NATIVE_LIBS "${CMAKE_MATCH_1}")
    elseif(token MATCHES "^-L(.+)")
        list(APPEND SW_NATIVE_LIB_DIRS "${CMAKE_MATCH_1}")
    elseif(token MATCHES "\\.lib$")
        list(APPEND SW_NATIVE_LIBS "${token}")
    else()
        # rustc warns that token order and duplication can be
        # significant, so silently dropping an unrecognized token is
        # not an option: fail the configure and add proper support.
        message(FATAL_ERROR
            "Unsupported Rust native-static-libs token: '${token}'")
    endif()
endforeach()

if(SW_EXPECT_FRAMEWORK)
    message(FATAL_ERROR
        "Malformed native-static-libs output: -framework without a framework name")
endif()

message(STATUS "Rust native-static-libs (raw): ${SW_NATIVE_LIBS_RAW}")
message(STATUS "Rust staticlib native dependencies (after driver-runtime filter): ${SW_NATIVE_LIBS}")

# ---------------------------------------------------------------------------
# Application
# ---------------------------------------------------------------------------

qt_add_executable(sw-explorer
    src/main.cpp
    src/MainWindow.h
    src/MainWindow.cpp
    src/BackendWorker.h
    src/BackendWorker.cpp
    "${SW_GENERATED_SOURCE}"
)

set_target_properties(sw-explorer PROPERTIES
    WIN32_EXECUTABLE ON
    MACOSX_BUNDLE ON
)

target_include_directories(sw-explorer PRIVATE "${SW_GENERATED_DIR}")

# Build the staticlib before anything of sw-explorer compiles or links.
add_dependencies(sw-explorer sw_gui_bridge_staticlib)

target_link_libraries(sw-explorer PRIVATE
    Qt6::Widgets
    sw_gui_bridge
    ${SW_NATIVE_LIBS}
)

if(SW_NATIVE_LIB_DIRS)
    target_link_directories(sw-explorer PRIVATE ${SW_NATIVE_LIB_DIRS})
endif()
