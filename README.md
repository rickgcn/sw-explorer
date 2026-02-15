# sw-explorer

`sw-explorer` is a Qt desktop app for browsing and extracting SGI IRIX dist packages (`.idb` + subproduct payload files).
It provides a Explorer-like workflow: menu bar, icon toolbar, directory-style file list, and batch extraction.

## Features

- Parse IRIX `product.idb` files (Latin-1 safe parsing for offset consistency).
- Open either:
  - a full dist directory, or
  - a single `.idb` file (auto-resolves product and dist path).
- File-manager style browsing (directory tree view behavior, not flat listing).
- Wildcard subgroup mask filtering and filename contains filtering.
- Symbolic link awareness in browser and extraction.
- Robust payload re-sync when offsets drift:
  - scans around expected offsets,
  - supports name variants (`fname`, `./fname`, `/fname`).
- Built-in `.Z` (Unix compress/LZW) decompression with ncompress-compatible code-width transitions.
- Extraction controls:
  - `No Decompress (.Z only)`
  - `Keep .Z files`
  - `Continue on error`
- Context menu on file list (`Open`, `Up`, `Extract Selected`, `Extract Here Tree`, `Copy Path`).
- Async product scanning and throttled progress UI updates for smoother performance.

## Project Layout

```text
sw-explorer/
  app/                    # Qt Widgets application (UI, actions, model)
  core/                   # Parsing and extraction engine
    include/swcore/
    src/
  CMakeLists.txt
```

## Requirements

- CMake >= 3.16
- C++17 compiler
- Qt 6 (Widgets, Core, Concurrent)

## Build

From repository root:

```bash
cmake -S . -B build
cmake --build build --config Release
```

Main executable (MSVC multi-config):

```text
build/app/Release/sw-explorer.exe
```

## Quick Start

1. Launch `sw-explorer`.
2. Click **Open Dist...** and select an IRIX dist directory (or use **Open IDB...**).
3. Choose a **Product** in toolbar.
4. Use:
   - **Mask** for subgroup wildcard filtering (default `*`),
   - **Filter** for filename contains filtering.
5. Browse folders in the table (double-click folder or symlinked folder).
6. Extract using:
   - **Extract Selected...** for selected rows,
   - **Extract Here Tree...** for current directory subtree.

## Notes on Extraction Behavior

- For file entries, compressed payload is first materialized as `target.Z`.
- If decompression is enabled and payload is a valid `.Z` stream, output file is written as decompressed content.
- If `Keep .Z files` is unchecked, temporary `.Z` files are removed after successful extraction.
- On systems where symlink creation is unavailable, link targets are saved as `*.link.txt` fallback files.

## License

GPL-3.0-only (see `LICENSE`).
