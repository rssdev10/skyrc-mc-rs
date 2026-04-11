# Packaging Assets

This directory contains auxiliary files used to assemble portable release artifacts.

- `common/` holds files included in more than one operating system package.
- `linux/` holds Linux-specific packaging notes.
- `macos/` holds macOS bundle metadata such as the `Info.plist` template.
- `windows/` holds Windows-specific packaging notes.

The release workflow in `.github/workflows/release.yml` consumes these files directly.
