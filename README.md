<p align="center"><img src="ui/assets/picstome-sync-logo.png" width="120" alt="Picstome Sync"></p>

# Picstome Sync

Desktop app to keep Picstome galleries and local photo folders in sync, while
keeping the Picstome rating workflow.

Built with **Tauri v2 + Rust** and the operating system webview. There is no
Electron, no Node runtime, no daemon and no ZIP step. The Linux release binary is
about 11 MB; bundled exiftool resources are the largest extra asset.

- Rust engine crate: [`crates/picstome-sync-core`](crates/picstome-sync-core) — API client, diff, downloads, uploads, snapshots, fake API. No Tauri dependency, fully testable with `cargo test`.
- Tauri shell: [`src-tauri`](src-tauri) — commands, OS keychain auth, deep links, native dialogs, exiftool rating.
- UI: [`ui`](ui) — small vanilla HTML/CSS/JS (Tauri global API), no bundler.

Implements the desktop side of
[picstome/picstome-sync#1](https://github.com/picstome/picstome-sync/issues/1)
against the backend contract in
[picstome/picstome#288](https://github.com/picstome/picstome/issues/288).

## Features

- **First sync**: `picstome://sync?gallery=G123&name=MiBoda` opens the app,
  asks for a folder once, downloads the manifest with parallel range requests,
  verifies SHA-256 and writes `<galleryId>.json`.
- **Updates**: three-way diff (remote / local / last snapshot) with downloads,
  uploads, local deletions proposed as remote deletions, and conflict reporting.
- **Resume**: `.part` + `.part.json` chunk map; a killed download resumes
  completed chunks before renaming.
- **Safety**: manifest filenames are sanitized, every write is checked to stay
  inside the linked folder, and remote `DELETE` only runs after an explicit
  confirmation with checkboxes.
- **Auth**: browser flow `/authorize` → `picstome://auth` → token exchange,
  token stored in the OS keychain (`keyring`). Dev override: `PICSTOME_TOKEN`
  (debug builds only).
- **Deep links**: `picstome://rate`, `picstome://sync`, `picstome://auth`, with
  single-instance forwarding on Windows/Linux.
- **Rate tab**: same star flow as Picstome Rater, EXIF/XMP in batches of 20
  through bundled exiftool (official Perl script + lib on macOS/Linux, native
  `.exe` on Windows; system `exiftool` is used as a fallback).
- **Fake API**: seeded galleries (50 + 8 photos), signed URLs, Range support,
  upload/version/delete, browser auth page and a fake web UI to simulate remote
  changes.
- **Runtime info for testers**: an **Info** button shows the exact API URL,
  app version, build profile (debug/release) and mode (production / development
  / local / demo / custom). Outside production a small `DEV`, `LOCAL`, `DEMO`
  or `CUSTOM` badge stays visible in the header with the API URL as its title.

## Quick start (no backend)

```bash
cargo run -p picstome-sync -- --demo
```

`--demo` starts the fake API inside the app process, points the app at it and
logs in automatically. Press **Sync demo gallery** (`MiBoda`, 50 fake photos)
and choose an empty folder. Open the printed fake-web URL to add/change/delete
remote photos, then press **Update** in the app.

You can also run the fake API standalone:

```bash
cargo run -p picstome-sync-core --bin picstome-sync-mock
# in another terminal
PICSTOME_API_BASE=http://127.0.0.1:8787 PICSTOME_MOCK=1 cargo run -p picstome-sync
```

## Real API

```bash
PICSTOME_API_BASE=https://devel.picstome.com cargo run -p picstome-sync
```

The app opens the browser authorization page, receives the one-time code
through the `picstome://auth` deep link, exchanges it for a token and stores it
in the OS keychain.

For automation/testing, a token can be supplied directly (debug builds / demo mode only):

```bash
PICSTOME_API_BASE=http://127.0.0.1:8000 PICSTOME_TOKEN='1|...' cargo run -p picstome-sync
```

`devel.picstome.com` was validated end-to-end: browser login + consent +
`picstome://auth` code exchange, token storage, manifest, 50 downloads,
no-op re-sync, upload, remote deletion and logout. The web UI stays behind the
devel HTTP Basic auth; only `/api/v1/*` is exempted so Bearer-token clients can
work (those endpoints still require a valid token).

## Tests

```bash
# engine + transfer tests against the Rust fake API
cargo test -p picstome-sync-core

# app unit tests: rating/exiftool, i18n key parity, auth nonce shape, API mode
cargo test -p picstome-sync --lib

# everything (CI runs this on Linux)
cargo test --workspace --all-targets

# app binary self-test: starts the fake API, syncs 50 photos, verifies resume/no-op
cargo run -p picstome-sync -- --smoke

# full Tauri UI E2E through WebDriver (needs tauri-driver + WebKitWebDriver + display)
cargo build -p picstome-sync
cargo build -p picstome-sync-core --bin picstome-sync-mock
python3 scripts/e2e.py                 # mock mode, upload + delete round-trip
xvfb-run -a python3 scripts/e2e.py     # headless Linux

# engine against a real/local Picstome API (no UI)
PICSTOME_REAL_API=http://127.0.0.1:8000 \
PICSTOME_REAL_TOKEN='1|...' \
PICSTOME_REAL_GALLERY='01...' \
PICSTOME_REAL_WRITE=1 \
cargo run -p picstome-sync-core --bin real-api-smoke

# full UI against a real API (set E2E_WRITE=1 to upload + delete)
E2E_MOCK=0 E2E_WRITE=0 E2E_GALLERY='01...' \
PICSTOME_API_BASE=http://127.0.0.1:8000 PICSTOME_TOKEN='1|...' \
python3 scripts/e2e.py
```

The UI E2E drives the real app: it performs the browser auth contract (nonce →
`/authorize` code → `picstome://auth`), asserts the rendered window state, syncs
50 photos, checks the local files and snapshot, verifies the second sync is a
no-op, uploads a local file, applies a remote deletion, checks the final no-op
state and logs out.

## GitHub Actions

| Workflow | Trigger | What it does |
| --- | --- | --- |
| `ci.yml` | push/PR/manual | Rustfmt + Clippy + engine tests on Linux/macOS/Windows; workspace tests + app `--smoke` + full UI E2E (auth, sync, upload, delete, rate, info panel) on Linux; core coverage report (`lcov`) as an artifact |
| `build.yml` | push to `main`/manual | unsigned Linux, universal macOS and Windows installers uploaded as workflow artifacts |
| `release.yml` | version tag/manual | builds and attaches installers to a **draft** GitHub release |
| `real-api.yml` | manual | engine + UI E2E against a real API (`PICSTOME_REAL_API`, `PICSTOME_REAL_TOKEN`, `PICSTOME_REAL_GALLERY`; optional write round-trip) |

To test an app build without tagging: **Actions → build → Run workflow** and
download the `picstome-sync-*` artifact.

### macOS install note

Builds are **ad-hoc signed** (no Apple Developer ID), so Gatekeeper may still
ask for confirmation:

1. Open the DMG and drag **Picstome Sync** to Applications.
2. Right-click the app in Finder and choose **Open** (or run
   `xattr -cr "/Applications/Picstome Sync.app"`).
3. If macOS still blocks it, allow it in **System Settings → Privacy & Security**.

The release draft body repeats these steps.

## Sync model

```
index.json       [{galleryId,name,localPath,lastSyncAt,remoteRev,parentGalleryId?}]
<galleryId>.json {galleryId,localPath,lastSyncAt,remoteRev,files:[{id,filename,
                  identity_key,hash,size,mtime,version_count,version_number}]}
config.json      {photosDir,lastRating,lastUpdateCheckAt,accountEmail,apiBase?}
```

- Identity is `family:stem` (lowercase, extension ignored for images), so
  `IMG_1.JPG` and `IMG_1.DNG` are one logical photo.
- The app data directory is the OS app-data dir; override for tests with
  `PICSTOME_USER_DATA_DIR`.
- Downloads never trust the manifest alone: bytes are hashed and compared before
  the atomic rename.
- `version_count` changes count as remote changes even when bytes are identical.

## Security

- Folder paths never come from deep links; only from native folder pickers.
- Tauri capabilities are locked down to the `main` window (`core`, `dialog`,
  `shell`, `opener`, `deep-link`, `updater`).
- The webview uses a strict CSP and the UI has no Node access.
- Tokens go to the OS keychain through `keyring`; `PICSTOME_TOKEN` exists only
  as an explicit development/testing override and is ignored in release builds.
- Remote deletion is never automatic and is auditable on the backend.

## Project layout

```
crates/picstome-sync-core/
  src/{api,config,diff,identity,links,localfiles,mock,model,sanitize,sync,transfer}.rs
  src/bin/{mock-server,real-api-smoke}.rs
  tests/integration.rs
src-tauri/
  src/{lib,commands,deep_link,auth,rate,strings,smoke}.rs
  tauri.conf.json, capabilities/, icons/
ui/
  index.html, styles.css, renderer.js, assets/
scripts/e2e.py
resources/exiftool*
```

## Next steps (tracked in GitHub)

- Issue #2: version-history UI (`vN` badges, download old versions, conflict
  resolution actions).
- Issue #3: detect `Editadas/Edits/Edited` folders and propose a
  `<Gallery> Edit` gallery.
- Auto-update: the updater plugin is wired, but disabled in `tauri.conf.json`
  until a signing keypair and release endpoints/secrets are configured.
