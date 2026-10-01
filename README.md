# TreeMap

TreeMap is a local-first terminal app for exploring drive and folder space usage. It indexes filesystem metadata in a local SQLite database and keeps logical file sizes separate from physical drive capacity.

## Features

- Browse drives, folders, and files, with folder totals and largest-first ordering.
- Press `b` to toggle proportional logical-size bars and percentage shares.
- Press `d` on a selected file or folder to open its action prompt. Press `r` to move it to the Recycle Bin, or `p` and then `y` to confirm permanent deletion.
- View modified and accessed timestamps. Filesystem access times may be unavailable or delayed.
- Use the local index for subsequent browsing; TreeMap does not read file contents or send scan data over a network.

## Build and Run

Install a current stable Rust toolchain, then run:

```powershell
cargo run --release
```

To build the Windows executable without launching it:

```powershell
cargo build --release --package DirMap
```

The executable is written to `target/release/DirMap.exe` unless `CARGO_TARGET_DIR` is set.

## Download and Launch on Windows

Download `TreeMap-windows-x86_64.zip` from the project's GitHub Releases page, extract it, and launch `TreeMap.exe`. The release package includes the app, this README, and both license files; Rust is not required to run it.

To create the same package locally after building:

```powershell
cargo build --locked --release --package DirMap
.\scripts\package-windows.ps1
```

This writes `dist/TreeMap-windows-x86_64.zip` and its `.sha256` checksum. Pushing a version tag such as `v0.1.0` runs the Windows release workflow and attaches both files to a GitHub Release.

## Local Data

The index is stored in the operating system user data directory by default. Set `DIRMAP_DATA_DIR` to choose a different location. The index is not encrypted; its privacy depends on operating-system account and filesystem permissions.

## Workspace

- `src/`: TreeMap terminal app, consent flow, and interface.
- `open-source-core/`: reusable filesystem scanner, local index, and guarded file actions.

Run `cargo test --workspace` from the repository root to test both packages.

## License

MIT. See [LICENSE](LICENSE).