<p align="center"><img src="ui/assets/picstome-sync-logo.png" width="120" alt="Picstome Sync"></p>

> **Development / testing build — not production software.**
> This repository is published to test and iterate on Picstome Sync. The app can
> read photo metadata, download, upload and delete photos, and write EXIF/XMP
> ratings. Use test galleries and keep backups. Do not point it at a production
> photo library without a backup and an explicit review.

# Picstome Sync

Desktop app to keep Picstome galleries and local photo folders in sync, while
keeping the Picstome rating workflow.

Built with **Tauri v2 + Rust** and the operating system webview. There is no
Electron, no Node runtime, no daemon and no ZIP step. The Linux release binary is
about 11 MB; bundled exiftool resources are the largest extra asset.

- Rust engine crate: [`crates/picstome-sync-core`](crates/picstome-sync-core) — API client, diff, downloads, uploads, snapshots, fake API. No Tauri dependency, fully testable with `cargo test`.
- Tauri shell: [`src-tauri`](src-tauri) — commands, persistent auth, deep links, native dialogs, exiftool rating.
- UI: [`ui`](ui) — small vanilla HTML/CSS/JS (Tauri global API), no bundler.

Implements the desktop side of
[picstome/picstome-sync#1](https://github.com/picstome/picstome-sync/issues/1)
against the backend contract in
[picstome/picstome#288](https://github.com/picstome/picstome/issues/288).

## Features

- **First sync**: `picstome://sync?gallery=G123&name=MiBoda` opens the app,
  asks for a folder once, downloads the manifest with parallel range requests,
  verifies SHA-256 and writes `<galleryId>.json`.
- **Updates**: explicit three-way state transitions (remote / local / last
  snapshot) with downloads, uploads, local deletions proposed as remote
  deletions, and conflict reporting. The snapshot is updated from confirmed
  transfer results, without a second full local scan or a second manifest
  request.
- **Remote deletions are explicit**: if a photo disappears from Picstome, Sync
  opens a dialog with **Move to Trash / Keep local / Upload again**. Local files
  are moved to the OS Trash/Recycle Bin, or to a `.picstome-trash` folder inside
  the gallery if the OS trash is unavailable. It never re-uploads a remotely
  deleted photo silently; the choice is persisted in the snapshot so the same
  photo is not proposed again after resolving it.
- **Destructive actions need confirmation**: local-file moves to trash and
  remote deletions both open a second confirmation dialog with the exact count.
  Confirmed actions and failures are written to the gallery activity log.
- **Conflicts are not auto-resolved**: when both sides changed, Sync keeps the
  last common baseline, reports the conflict, and never overwrites either copy
  until it can be resolved explicitly.
- **Resume**: `.part` + `.part.json` chunk map; a killed download resumes
  completed chunks before renaming.
- **Safety**: manifest filenames are sanitized, every write is checked to stay
  inside the linked folder, and remote `DELETE` only runs after an explicit
  confirmation with checkboxes.
- **Auth**: browser flow `/authorize` → `picstome://auth` → token exchange,
  token stored in `auth.json` inside the app data directory with `0600`
  permissions and reused across launches. The keychain is intentionally not
  used in ad-hoc test builds because macOS prompts for the system password on
  every launch/update; a notarized build can switch back to Keychain later.
  Dev override: `PICSTOME_TOKEN` (debug builds only). The API issues tokens
  with `EXTERNAL_TOKEN_DAYS` (default 365); there is no refresh token in v1, so
  only an expired/revoked token asks to reconnect.
- **Deep links**: `picstome://rate`, `picstome://sync`, `picstome://auth`, with
  single-instance forwarding on Windows/Linux.
- **Rating integrated in Sync**: the old Rate tab is gone. `picstome://rate`
  links open a rating dialog in the same desktop interface, and when the
  manifest exposes `favorited_at` the app offers to write a chosen 1–5 star
  rating for those favorite photos after syncing. EXIF/XMP is written in
  batches of 20 through bundled exiftool (system `exiftool` fallback on Linux).
- **Desktop-first interface**: fixed sidebar + main pane, no page scroll,
  explicit **Sync** button, live progress for checking/scanning/downloading/
  uploading, manual folder actions, and account/profile access from the
  sidebar. English is the default; the UI switches to Spanish only when the
  current keyboard layout is Spanish.
- **Fast gallery search**: filter the sidebar by name or path, with `Ctrl+F` /
  `Cmd+F` focusing the search field and `Enter` selecting the first match.
- **Sync evidence**: the report lists every uploaded/downloaded filename, and
  each gallery has its own log at `logs/<gallery-id>.log` in the app data
  directory. Star ratings are logged too (`kind: "rating"`, stars, source
  link/favorites, files rated and errors). The same history is shown per
  gallery in the collapsible **Activity log** panel, which also logs deletions,
  trash destinations, conflicts and sync failures, and has a **Clear log**
  button. Logs rotate automatically at 512 KB, keeping the last
  500 entries; the old single `sync.log` is imported into these files once and
  removed on startup. The app data directory (not the photo folder) contains
  `index.json`, the per-gallery snapshots, `auth.json` and `logs/`; no hidden
  state file is added to the photos folder.
- **Sensible rating target**: a `picstome://rate` link uses the selected
  gallery's folder automatically; the folder picker only appears when no
  gallery is linked/selected.
- **Fake API**: seeded galleries (50 + 8 photos), signed URLs, Range support,
  upload/version/delete, browser auth page and a fake web UI to simulate remote
  changes.
- **Runtime info for testers**: an **Info** button shows the exact API URL,
  app version, build profile (debug/release), account and mode (production /
  development / local / demo / custom). A small `PROD`, `DEV`, `LOCAL`, `DEMO`
  or `CUSTOM` badge stays visible in the sidebar. The connected account shows
  the Picstome display name; clicking it opens `/settings/profile`.

### Lightroom and Capture One workflow

- Sync writes `Rating`/`RatingPercent` into the local files with exiftool (embedded XMP or sidecar, depending on the format). Favorites are matched recursively inside the linked folder (up to 8 levels); hidden and `Editadas/Edits` folders are skipped.
- The **Favorites list** button copies the current gallery favorites in the same formats as the web app: App deep link, Lightroom (comma-separated filenames without extension), Capture One (space-separated), Finder/Explorer (`OR` query), and a plain list of full filenames.
- **Lightroom Classic does not automatically re-read metadata** for photos that are already imported. After applying ratings, select the photos and use **Metadata → Read Metadata from File**. If the photos are not imported yet, importing the synced folder is enough.
- Sync cannot trigger that Lightroom command itself: Lightroom Classic does not expose a supported external API for it without a plug-in.
- **Lightroom Cloud** does not re-read external file metadata after import. Automatic catalog updates there require the Adobe Lightroom partner API (not implemented yet).

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

## Testing a downloaded build

1. Open **GitHub → Releases** and download the latest **draft** asset (maintainers
   see drafts on the releases page), e.g. `Picstome.Sync_<version>_universal.dmg`.
   With the CLI: `gh release download v0.1.18 --pattern '*.dmg'`.
   New test builds are produced locally with `scripts/release-local.sh`.
2. Install it, open **Info** in the header and check **Version**, **Build**,
   **Mode** and **API target**.
3. Use **Test API target → Production / Development** to switch between
   `https://app.picstome.com` and `https://devel.picstome.com`. Switching signs
   you out; press **Open Picstome** to authorize again.
4. The header badge always shows `PROD`, `DEV`, `LOCAL`, `DEMO` or `CUSTOM`, so
   you can tell at a glance which backend the app is using.

Production is the default: `https://app.picstome.com`.

## Real API

```bash
# production (default)
cargo run -p picstome-sync

# development
PICSTOME_API_BASE=https://devel.picstome.com cargo run -p picstome-sync
```

The app opens the browser authorization page, receives the one-time code
through the `picstome://auth` deep link, exchanges it for a token and stores it
in the app data directory (`0600`).

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
| `ci.yml` | push/PR/manual | Linux-only Rustfmt + Clippy + engine tests, workspace tests, app `--smoke`, full UI E2E and core coverage report. |
| `build.yml` | push to `main`/manual | macOS universal test DMG uploaded to the `v<version>` draft release (and as a workflow artifact). Windows test builds stay disabled until the app is stable. |
| `release.yml` | version tag/manual (final releases) | builds and attaches Linux/macOS/Windows installers to a **draft** GitHub release. Windows belongs here only for stable/final tags. |
| `real-api.yml` | manual | engine + UI E2E against a real API (`PICSTOME_REAL_API`, `PICSTOME_REAL_TOKEN`, `PICSTOME_REAL_GALLERY`; optional write round-trip) |

## Local tests and builds (no Actions minutes)

Run the checks locally:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo run -p picstome-sync -- --smoke
# full UI E2E (Linux):
xvfb-run -a python3 scripts/e2e.py
```

Until the final release, build the current platform locally and upload it to a
draft release:

```bash
scripts/release-local.sh            # current version, builds + uploads
scripts/release-local.sh --no-build # upload already-built bundles
scripts/release-local.sh v0.1.7     # explicit draft tag
```

- On **Linux** it builds `.deb` + `.AppImage`.
- On **macOS** it builds a universal `.dmg` (run it on the Mac), or trigger the
  manual **build** workflow and pass the draft tag (e.g. `v0.1.7`).
- Windows builds are intentionally reserved for the final `release.yml` run.

The script uses `gh release upload --clobber`, so re-running it replaces the
same draft assets instead of creating a new release.

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

## Security and repository visibility

The repository is safe to make public **after reviewing these points**:

- No credentials, tokens, `.env` files or devel Basic Auth passwords are
  committed (checked across the git history). Workflows reference GitHub
  secrets by name only; public visibility does not reveal their values.
- The only automation that can write to a release runs on tags/manual dispatch
  and uses `GITHUB_TOKEN`; fork pull requests do not receive repository secrets.
- Commit metadata is public: the current history is authored with
  `neo22s@gmail.com`. If you do not want that address on a public repository,
  rewrite the author before changing visibility or use a GitHub noreply address
  for future commits.
- `resources/exiftool*` are third-party binaries; their license files are kept
  alongside them. The app logo is the Picstome Sync logo supplied by the project.
- This repository is public: standard GitHub-hosted Actions runners are free,
  subject to GitHub's fair-use policy. CI runs on pushes/PRs and `build.yml`
  produces a macOS test DMG on every push to `main`. Windows test builds remain
  disabled until the app is stable; final tagged releases build Windows.
- Making a repository public does not expose its secrets, but it does expose the
  full source and history. Review the code for anything you consider private
  before changing visibility.

- Folder paths never come from deep links; only from native folder pickers.
- Tauri capabilities are locked down to the `main` window (`core`, `dialog`,
  `shell`, `opener`, `deep-link`, `updater`).
- The webview uses a strict CSP and the UI has no Node access.
- Tokens are stored in `auth.json` under the app data directory with `0600`
  permissions; `PICSTOME_TOKEN` exists only as an explicit development/testing
  override and is ignored in release builds. Keychain storage can be re-enabled
  once builds are signed/notarized with a Developer ID.
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
