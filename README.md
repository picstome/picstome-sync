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
- On the first sync of a folder that already contains photos, previews what
  exists only on disk and what exists only in Picstome. You choose whether to
  upload, download, keep each direction local, or move local-only files to the
  trash. Nothing is deleted without choosing it. Skipped items are tracked in
  `.picstome-ignore.json` and become sync candidates again if the file or the
  remote version changes.
- Downloads remote photos, uploads local photos, and updates changed files.
- Detects local deletions and proposes deleting the remote copy.
- Detects remote deletions and asks whether to trash the local copy, keep it,
  or upload it again.
- Shows up to five recent unlinked galleries from Picstome on startup.
- Searches synced and remote Picstome galleries with pagination; choosing a
  remote result asks for the local folder and links it.
- The `+` button opens the gallery-creation page on the current API target.
- Syncs the photographer's Picstome star rating (1–5) to the local files with
  exiftool, always after asking first. Photos with a heart but no rating open a
  single prompt: the stars chosen there apply to all of them.
- Writes manual 1–5 star ratings to local files with exiftool and exports
  favorite lists for Lightroom, Capture One, Finder/Explorer, or plain text.
- Shows photo count, disk usage, and a per-gallery activity log; a centered
  progress modal reports the current sync phase, file and percentage. Local
  folders report how many entries have been scanned and how many photos have
  been fingerprinted, so a large gallery never looks frozen.
- Opens the linked gallery in Picstome from the title link (falls back to the
  galleries page when the API does not expose `web_url`).
- Checks GitHub for a newer release on startup, at most once every 15 days,
  and offers a download link when available.
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
- **First sync:** without a snapshot the app cannot tell a new local photo from
  one deleted remotely, so it shows a preview before transferring. Local-only
  files can be uploaded, kept only on disk, or moved to the OS trash (with the
  `.picstome-trash` folder as fallback). Choices for files kept on only one side
  are stored in `.picstome-ignore.json` inside the gallery folder; changing the
  file or receiving a new remote version makes it eligible again. Use **Include
  ignored again** in the report to clear them.
- **Downloads:** signed URLs with parallel range requests and resume support.
  Verification uses the manifest `download_hash` when available.
- **Ratings:** the manifest `rating` (1–5) is the source of truth and is copied
  to the local file metadata via exiftool after a confirmation. A rating-only
  change never re-downloads the file. `is_favorite` drives the heart; a heart
  without a rating is queued in a single "favourite photos" prompt instead of
  being assumed to be five stars. `rating_applied` in the snapshot remembers
  what was written, so re-syncs do not ask or rewrite again. The manual Rate tab
  (deep link or manual pick) is separate and only changes local metadata.
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

# developer mode; shows a DEV badge in the sidebar
cargo run -p picstome-sync -- --devel

# local API
PICSTOME_API_BASE=http://127.0.0.1:8000 cargo run -p picstome-sync
```

Normal app builds always use production (`https://app.picstome.com`). A
previous development target stored by an older version is ignored. The
**Info → Test mode** button switches to devel for the current session and
shows a DEV badge; it returns to production on restart. `--devel` or
`PICSTOME_API_BASE` remain available for development.

The app opens the Picstome authorization page. After approving, the
`picstome://auth` deep link completes token exchange.

Useful developer variables:

| Variable | Purpose |
| --- | --- |
| `PICSTOME_API_BASE` | API URL; defaults to `https://app.picstome.com`. |
| `PICSTOME_USER_DATA_DIR` | Override the app data directory (useful in tests). |
| `PICSTOME_TOKEN` | Pre-supply a token in debug/demo builds only. |
| `PICSTOME_MOCK=1` | Use mock mode. |
| `PICSTOME_DISABLE_UPDATE_CHECK=1` | Skip the GitHub update check (tests/E2E). |

### Standalone mock API

```bash
cargo run -p picstome-sync-core --bin picstome-sync-mock
# another terminal
PICSTOME_API_BASE=http://127.0.0.1:8787 PICSTOME_MOCK=1 \
  cargo run -p picstome-sync
```

## Tests and builds

Run tests locally:

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

CI runs the same tests on pull requests and manual dispatches only. Pushing to
`main` does not build installers or create releases automatically.

> **Constrained machines:** the development container is limited to 3 GiB of
> RAM. `.cargo/config.toml` caps Cargo at one local job, and dev/test builds
> keep line tables instead of full debug info. For quick checks use
> `cargo test -p picstome-sync-core`; the full workspace and E2E also build
> Tauri. Override with `CARGO_BUILD_JOBS=4` on a larger machine.

To build test installers, open **GitHub → Actions → build → Run workflow**.
This builds Linux, macOS, and Windows on GitHub and can optionally update the
draft release:

- Linux: `.deb` + `.AppImage`.
- macOS: universal `.dmg` (ad-hoc signed, so Gatekeeper may require
  right-click → Open).
- Windows: unsigned NSIS `.exe` and MSI; SmartScreen may warn.

`scripts/release-local.sh` remains available as a local fallback. Final releases
are created manually through **Actions → release**.

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
