# SW Explorer

Qt Widgets frontend for the SW Explorer workspace. The UI is a Qt
Widgets / C++ application; all IRIX knowledge lives in the Rust
`sw-core` crate, reached through the `sw-gui-bridge` staticlib and a
CXX-generated bridge.

## Prerequisites

- Rust 1.95+
- `cxxbridge-cmd` 1.0.199 (must match the `cxx` crate version exactly)
- CMake 3.21+
- Qt 6.8+ (Widgets)
- A C++17 compiler

```sh
cargo install cxxbridge-cmd --version 1.0.199 --locked
```

## Build

```sh
cmake -S apps/sw-explorer -B build/sw-explorer \
  -DCMAKE_BUILD_TYPE=Release

cmake --build build/sw-explorer --config Release --parallel
```

CMake generates the CXX bridge sources into the build tree with
`cxxbridge`, builds the `sw_gui_bridge` staticlib with Cargo, and links
it together with the native libraries reported by
`rustc --print native-static-libs`.

Run the resulting `sw-explorer` binary (or `sw-explorer.app` on macOS)
and use *File > Open Distribution...* to open an IRIX `dist` directory.

## Tests

The Qt model tests use Qt Test and CTest. They are GUI-free
(`QTEST_GUILESS_MAIN`) and run headless on every platform:

```sh
ctest --test-dir build/sw-explorer --build-config Release --output-on-failure
```
