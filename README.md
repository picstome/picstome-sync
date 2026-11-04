<p align="center"><img src="ui/assets/picstome-sync-logo.png" width="120" alt="Picstome Sync"></p>

# Picstome Sync

Desktop client that keeps [Picstome](https://picstome.com) galleries and local
photo folders in sync.

It downloads remote photos, uploads new or modified local files, handles remote
and local deletions with confirmation, and integrates the Picstome favorites
and star-rating workflow.

> **Test build.** Use test galleries and keep backups. The app can delete
> photos from Picstome and move local files to the trash.

## What it does

- Syncs one local folder with one Picstome gallery.
- Downloads remote photos, uploads local photos, and updates changed files.
- Detects local deletions and proposes deleting the remote copy.
- Detects remote deletions and asks whether to trash the local copy, keep it,
  or upload it again.
- Writes 1–5 star ratings to local files with exiftool and exports favorite
  lists for Lightroom, Capture One, Finder/Explorer, or plain text.
- Shows photo count, disk usage, sync progress, and a per-gallery activity log.
- Never overwrites conflicts automatically.

## How it works

- **Tauri v2 + Rust** desktop app. The sync engine lives in
  [`crates/picstome-sync-core`](crates/picstome-sync-core) and does not depend on
  Tauri. The UI is static HTML/CSS/JS in [`ui`](ui).
- **Authentication:** the browser authorization flow returns to
  `picstome://auth`; the token is stored in `auth.json` (mode `0600`) in the
  app data directory.
- **Sync state:** each gallery has a JSON snapshot in the app data directory.
  Remote changes are detected by `version_count`; local changes by the file
  hash, size, and modification time. This avoids treating a resized web copy as
  a conflict with the original local file.
- **Downloads:** signed URLs with parallel range requests and resume support.
  Verification uses the manifest `download_hash` when available.
- **Safety:** writes stay inside the linked folder, filenames are sanitized,
  and destructive actions require explicit confirmation. Local deletions go to
  the OS trash or a `.picstome-trash` folder inside the gallery.
- **Logs:** per-gallery JSONL logs at `logs/<gallery-id>.log`, rotated at
  512 KB. Uploads, downloads, ratings, deletions, trash locations, errors,
  warnings, and conflicts are recorded.

## Requirements

- Rust **1.82+**.
- Tauri v2 prerequisites for your OS:
  <https://v2.tauri.app/start/prerequisites/>.
- Linux E2E additionally needs `tauri-driver`, `WebKitWebDriver`, and Xvfb.
- exiftool is bundled for macOS/Windows; Linux can use the bundled build or a
  system `exiftool`.

## Run it

```bash
git clone https://github.com/picstome/picstome-sync.git
cd picstome-sync
```

### Demo, no backend

```bash
cargo run -p picstome-sync -- --demo
```

This starts a seeded fake Picstome API, logs in automatically, and lets you
sync a 50-photo demo gallery into an empty folder.

### Against Picstome

```bash
# production
cargo run -p picstome-sync

# development
PICSTOME_API_BASE=https://devel.picstome.com cargo run -p picstome-sync

# local API
PICSTOME_API_BASE=http://127.0.0.1:8000 cargo run -p picstome-sync
```

The app opens the Picstome authorization page. After approving, the
`picstome://auth` deep link completes token exchange.

Useful developer variables:

| Variable | Purpose |
| --- | --- |
| `PICSTOME_API_BASE` | API URL; defaults to `https://app.picstome.com`. |
| `PICSTOME_USER_DATA_DIR` | Override the app data directory (useful in tests). |
| `PICSTOME_TOKEN` | Pre-supply a token in debug/demo builds only. |
| `PICSTOME_MOCK=1` | Use mock mode. |

### Standalone mock API

```bash
cargo run -p picstome-sync-core --bin picstome-sync-mock
# another terminal
PICSTOME_API_BASE=http://127.0.0.1:8787 PICSTOME_MOCK=1 \
  cargo run -p picstome-sync
```

## Tests and builds

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets

# app smoke test: fake API, download, resume/no-op verification
cargo run -p picstome-sync -- --smoke

# full UI E2E through WebDriver
cargo build -p picstome-sync
cargo build -p picstome-sync-core --bin picstome-sync-mock
xvfb-run -a python3 scripts/e2e.py
```

Build local installers and upload them to the current draft release:

```bash
scripts/release-local.sh
```

- Linux: `.deb` + `.AppImage`.
- macOS: universal `.dmg` (ad-hoc signed, so Gatekeeper may require
  right-click → Open).
- Windows: unsigned NSIS `.exe` and MSI installers; SmartScreen may warn.

Pushing to `main` runs Linux CI and builds macOS and Windows test installers
with GitHub Actions. Final tagged releases build Linux, macOS, and Windows.

## Supported files

Local uploads support:

`jpg`, `jpeg`, `png`, `tiff`, `dng`, `cr2`, `cr3`, `nef`, `arw`, `orf`, `rw2`,
`pef`, `srw`, `mos`, `mrw`, `3fr`, `mp4`, `mov`, `avi`, `mkv`, `webm`, `m4v`,
`ogg`, and `pdf`.

Matching is case-insensitive. HEIC is not read. Files with the same stem are one
logical photo (`IMG_1.JPG` + `IMG_1.DNG`); only the preferred file is synced.
If two local files have the same stem and conflicting content, the app warns
and leaves the ignored file on disk.

## Project layout

```text
crates/picstome-sync-core/   API, diff, sync engine, transfers, mock API
src-tauri/                   Tauri shell, commands, auth, deep links, exiftool
ui/                          desktop UI
scripts/e2e.py               WebDriver E2E
resources/                   bundled exiftool
```

The API contract follows
[picstome/picstome#288](https://github.com/picstome/picstome/issues/288).
