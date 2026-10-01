<div align="center">
	<img src="TreeMapicon.png" alt="TreeMap icon" width="112">
	<h1>TreeMap</h1>
	<p><strong>See what's using your drive space.</strong><br>
	A keyboard-driven, local-first explorer for Windows.</p>
	<p>
		<a href="https://github.com/slacke101/treemap/releases/latest/download/TreeMap-windows-x86_64.zip"><img src="https://img.shields.io/badge/Download-Windows%20x64-087f6e?style=for-the-badge&amp;logo=windows&amp;logoColor=white" alt="Download TreeMap for Windows x64"></a>
		<a href="https://github.com/slacke101/treemap/releases"><img src="https://img.shields.io/github/v/release/slacke101/treemap?label=Latest%20release&amp;style=for-the-badge" alt="Latest TreeMap release"></a>
	</p>
	<p><sub>Windows 10/11 &middot; Portable ZIP &middot; Free and open source</sub></p>
</div>

---

TreeMap helps you find the folders and files taking up space, without uploading your scan or reading file contents.

## Start in 3 steps

1. [Download the latest Windows ZIP](https://github.com/slacke101/treemap/releases/latest/download/TreeMap-windows-x86_64.zip).
2. Extract it anywhere you like.
3. Open `TreeMap.exe` and choose a drive.

No installer or Rust setup needed. The first scan can take a while on large drives; later browsing uses the local index.

## Find space, fast

| Explore | Compare | Take action |
| --- | --- | --- |
| Browse drives, folders, and files, with the largest items first. | Compare indexed file sizes with physical capacity and free space. | Move selected items to the Recycle Bin, or permanently delete them after confirmation. |

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

In the action prompt, `r` moves the item to the Recycle Bin. Choose `p`, then press `y` to permanently delete it. Permanent deletion may be unrecoverable; check the full path before confirming.

## Private by design

TreeMap reads filesystem metadata such as names, types, sizes, and timestamps. It does not read file contents or send scan data over a network. The index is local, separate for each Windows account, and stored under `%LOCALAPPDATA%\DirMap` by default. Set `DIRMAP_DATA_DIR` to move it to another drive.

TreeMap opens at the drive list no matter where you launch it from. Access times may be disabled or imprecise. Links are not followed. Some system and application paths are protected from removal, but no protection list is perfect; review every target and keep backups.

## Windows security

This release is not signed by a trusted publisher, so SmartScreen may show an unknown-publisher or reputation warning. Verify that the ZIP came from the official download link above. If Microsoft Defender reports a specific threat, do not run the file.

## Developers

Install stable Rust, then run from the repository root:

```powershell
cargo run --release
cargo test --workspace
cargo build --locked --release --package DirMap
.\scripts\package-windows.ps1
```

The package script writes `dist/TreeMap-windows-x86_64.zip` and a SHA-256 checksum. A `v*` tag builds and publishes a Windows release. To sign releases, configure the `WINDOWS_SIGNING_CERTIFICATE_PFX` and `WINDOWS_SIGNING_CERTIFICATE_PASSWORD` GitHub Actions secrets with a trusted code-signing certificate.

## License

The app and core are MIT-licensed. See [LICENSE](LICENSE) and [open-source-core/LICENSE](open-source-core/LICENSE).