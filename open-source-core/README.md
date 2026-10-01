# open-source-core

The free, reusable filesystem engine shared by TreeMap applications.

## Scope

This crate provides filesystem metadata scanning, mounted-drive discovery, local SQLite indexing, cached directory summaries, and guarded Recycle Bin/permanent removal operations. It does not read file contents or contact a network service.

The crate contains no terminal UI, product consent flow, account system, licensing checks, or paid-tier gates. The free TreeMap application remains usable with this crate alone. A separate Pro application can depend on the same public core API without replacing or restricting the free functionality.

## Modules

- `drives`: mounted-drive discovery, metadata scanning, progress events, and live directory listings.
- `cache`: local SQLite snapshots, directory IDs, cached aggregates, and scan lifecycle operations.
- `file_actions`: shared protected-path checks and Recycle Bin/permanent removal.

The default index location can be overridden with `DIRMAP_DATA_DIR`. Use `cargo test --workspace` from the repository root to validate the app and core together.

## License and branding

The core source code is licensed under MIT. Castron's name, product names, logos, and visual identity are not licensed under MIT; see [BRANDING.md](../BRANDING.md).