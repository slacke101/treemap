# TreeMap

**Find what is using your drive space.**

TreeMap is a keyboard-driven Windows app for exploring drives, folders, and files by size.

[Download TreeMap for Windows](https://github.com/slacke101/treemap/releases/latest/download/TreeMap-windows-x86_64.zip) | [See all releases](https://github.com/slacke101/treemap/releases)

**Windows 10/11 | 64-bit | Portable ZIP | Free and open source**

## Get started

1. Download the ZIP above.
2. Extract it to a folder.
3. Open `TreeMap.exe`.

No installer or Rust toolchain is needed. The first scan can take a while on large drives.

## What you can do

- Browse drives, folders, and files, with the largest items shown first.
- Compare indexed file sizes with physical drive capacity and free space.
- View recent files and modified or accessed timestamps.
- Use the local index for faster browsing later.

## Keyboard guide

| Key | Action |
| --- | --- |
| `Up` / `Down`, `j` / `k` | Move through the list |
| `Enter` | Open a drive or folder |
| `Backspace`, `Left` | Go up one folder |
| `b` | Toggle size-share bars and percentages |
| `r` | Refresh the drive scan |
| `d` | Open actions for the selected item |
| `q` | Quit |

In the action prompt, `r` moves an item to the Recycle Bin. Choose `p`, then press `y` to permanently delete it. Permanent deletion may be unrecoverable; review the full path before confirming.

## Privacy and safety

TreeMap reads filesystem metadata such as names, types, sizes, and timestamps. It does not read file contents or send scan data over a network. Its local SQLite index is not encrypted, so protect it like other files in your account.

Access times may be disabled, delayed, or imprecise. Links are not followed. Some system and application paths are protected from removal, but no path list is perfect; always check the target before taking an action.

## Troubleshooting

- **"Database or disk is full"**: free space on the drive holding the local index, or set `DIRMAP_DATA_DIR` to a folder on a drive with more room.
- **Windows shows an unknown-publisher warning**: this release is not code-signed. Only run a copy downloaded from the official release page above.

## For developers

Install a current stable Rust toolchain, then run these commands from the repository root:

```powershell
cargo run --release
cargo test --workspace
```

Build and package the Windows release locally:

```powershell
cargo build --locked --release --package DirMap
.\scripts\package-windows.ps1
```

The executable is written to `target/release/DirMap.exe`. The package script creates `dist/TreeMap-windows-x86_64.zip` and a SHA-256 checksum. Pushing a version tag such as `v0.1.1` runs the Windows release workflow.

## License

The app and core are MIT-licensed. See [LICENSE](LICENSE) and [open-source-core/LICENSE](open-source-core/LICENSE).