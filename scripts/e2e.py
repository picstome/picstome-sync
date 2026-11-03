#!/usr/bin/env python3
"""End-to-end test for the real Tauri app through WebDriver.

Modes
-----
Mock mode (default, CI-friendly):
    E2E_MOCK=1 python3 scripts/e2e.py
  Starts `target/debug/picstome-sync-mock`, launches the app in demo mode,
  links gallery G123 to a temp folder, syncs 50 fake photos, uploads one,
  deletes it and verifies the final no-op sync.

Real API mode:
    E2E_GALLERY=<ulid> PICSTOME_API_BASE=https://devel.picstome.com \
    PICSTOME_TOKEN=... E2E_EXPECTED_PHOTOS=50 E2E_WRITE=1 \
    python3 scripts/e2e.py
  Set E2E_WRITE=0 (default) to avoid uploading/deleting on the real gallery.

Requirements: Rust app built (`cargo build -p picstome-sync`), tauri-driver,
WebKitWebDriver, and a display (use xvfb-run on headless Linux).
"""

import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from urllib.parse import parse_qs, urlparse

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
APP = os.environ.get("E2E_APP", os.path.join(ROOT, "target/debug/picstome-sync"))
DRIVER = os.environ.get("E2E_DRIVER", os.path.expanduser("~/.cargo/bin/tauri-driver"))
NATIVE_DRIVER = os.environ.get("E2E_NATIVE_DRIVER", "/usr/bin/WebKitWebDriver")
MOCK_BIN = os.environ.get("E2E_MOCK_BIN", os.path.join(ROOT, "target/debug/picstome-sync-mock"))
MOCK = os.environ.get("E2E_MOCK", "1") == "1"
WRITE = os.environ.get("E2E_WRITE", "1" if MOCK else "0") == "1"
EXPECTED = int(os.environ.get("E2E_EXPECTED_PHOTOS", "50"))
GALLERY = os.environ.get("E2E_GALLERY", "G123")
DEBUG_PORT = int(os.environ.get("E2E_DRIVER_PORT", "4444"))

BASE = f"http://127.0.0.1:{DEBUG_PORT}"
DRIVER_LOG_PATH = None
DRIVER_PROCESS = None


def log(*args):
    print(*args, flush=True)


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def wait_http(url, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2) as response:
                if response.status < 500:
                    return
        except Exception:
            pass
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for {url}")


def request(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(
        BASE + path,
        data=data,
        method=method,
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as response:
            return json.loads(response.read())
    except urllib.error.HTTPError as err:
        raise RuntimeError(
            f"{method} {path} -> {err.code}: {err.read().decode('utf-8', 'replace')}"
        )
    except Exception as err:
        code = DRIVER_PROCESS.poll() if DRIVER_PROCESS else None
        raise RuntimeError(f"{method} {path} failed: {err} (driver exit: {code})")


def run_async(session, script):
    time.sleep(0.05)
    result = request(
        "POST",
        f"/session/{session}/execute/async",
        {"script": script, "args": []},
    )
    value = result.get("value")
    if value is None and result.get("error"):
        raise RuntimeError(result)
    return value


def js_invoke(session, command, args):
    script = """
    const done = arguments[arguments.length - 1];
    window.__TAURI__.core.invoke(%s, %s)
      .then(r => done(JSON.stringify({ok: true, value: r})))
      .catch(e => done(JSON.stringify({ok: false, error: e && (e.message || e)})));
    """ % (json.dumps(command), json.dumps(args))
    result = json.loads(run_async(session, script))
    if not result.get("ok"):
        raise RuntimeError(f"{command} failed: {result.get('error')}")
    return result["value"]


def prepare_state(user_data, photos, gallery, rating_dir):
    os.makedirs(user_data, exist_ok=True)
    os.makedirs(photos, exist_ok=True)
    os.makedirs(rating_dir, exist_ok=True)
    galleries = [
        {
            "galleryId": gallery,
            "name": "E2E Gallery",
            "localPath": photos,
        }
    ]
    if MOCK:
        galleries.append(
            {
                "galleryId": "G456",
                "name": "Other Gallery",
                "localPath": rating_dir,
            }
        )
    with open(os.path.join(user_data, "index.json"), "w", encoding="utf-8") as handle:
        json.dump(galleries, handle)


def start_mock():
    port = free_port()
    process = subprocess.Popen(
        [MOCK_BIN],
        env={**os.environ, "PORT": str(port), "PHOTO_SIZE": "8192"},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    )
    wait_http(f"http://127.0.0.1:{port}/")
    return process, f"http://127.0.0.1:{port}"


def main():
    if not os.path.exists(APP):
        raise SystemExit(f"App binary not found: {APP}. Run cargo build -p picstome-sync first.")
    if not os.path.exists(DRIVER):
        raise SystemExit(f"tauri-driver not found: {DRIVER}")

    root = tempfile.mkdtemp(prefix="picstome-e2e-")
    user_data = os.path.join(root, "userData")
    photos = os.path.join(root, "photos")
    rating_dir = os.path.join(root, "rating")
    gallery = GALLERY
    mock_process = None
    env = {**os.environ}

    if MOCK:
        if not os.path.exists(MOCK_BIN):
            raise SystemExit(f"Mock server not found: {MOCK_BIN}")
        mock_process, api_base = start_mock()
        gallery = "G123"
        env.update(
            {
                "PICSTOME_API_BASE": api_base,
                "PICSTOME_TOKEN": "",
                "PICSTOME_USER_DATA_DIR": user_data,
            }
        )
        log("mock API:", api_base)
    else:
        token = os.environ.get("PICSTOME_TOKEN")
        api_base = os.environ.get("PICSTOME_API_BASE")
        if not token or not api_base:
            raise SystemExit("Real API mode needs PICSTOME_API_BASE and PICSTOME_TOKEN")
        env["PICSTOME_USER_DATA_DIR"] = user_data
        log("real API:", api_base)

    prepare_state(user_data, photos, gallery, rating_dir)
    env.setdefault("WEBKIT_DISABLE_COMPOSITING_MODE", "1")
    env.setdefault("LIBGL_ALWAYS_SOFTWARE", "1")
    env.setdefault("PICSTOME_DISABLE_BROWSER_OPEN", "1")
    env.setdefault("RUST_BACKTRACE", "1")

    global DRIVER_LOG_PATH, DRIVER_PROCESS
    DRIVER_LOG_PATH = os.path.join(root, "driver.log")
    driver_log = open(DRIVER_LOG_PATH, "w", encoding="utf-8")
    driver = subprocess.Popen(
        [DRIVER, "--port", str(DEBUG_PORT), "--native-driver", NATIVE_DRIVER],
        env=env,
        stdout=driver_log,
        stderr=subprocess.STDOUT,
    )
    DRIVER_PROCESS = driver
    session = None
    try:
        wait_http(BASE + "/status")
        created = request(
            "POST",
            "/session",
            {"capabilities": {"alwaysMatch": {"tauri:options": {"application": APP}}}},
        )
        session = created["value"]["sessionId"]
        log("session:", session)

        state = None
        for _ in range(60):
            state = js_invoke(session, "get_state", {})
            if state["auth"]["connected"] and state["galleries"]:
                break
            time.sleep(0.25)
        log(
            "state:",
            {
                "lang": state["lang"],
                "mock": state["mock"],
                "connected": state["auth"]["connected"],
                "galleries": len(state["galleries"]),
            },
        )
        assert state["galleries"], "gallery was not linked"

        if MOCK and not state["auth"]["connected"]:
            # Exercise the real browser auth contract against the fake API:
            # connect -> nonce -> /authorize code -> picstome://auth deep link.
            connect = js_invoke(session, "connect", {})
            nonce = parse_qs(urlparse(connect["url"]).query)["nonce"][0]
            page = urllib.request.urlopen(
                f"{api_base}/authorize?nonce={nonce}&client_name=picstome-sync",
                timeout=30,
            ).read().decode("utf-8", "replace")
            match = re.search(r"code=([A-Za-z0-9]+)", page)
            assert match, "authorize page did not contain a code"
            deep_link = f"picstome://auth?code={match.group(1)}&state={nonce}"
            js_invoke(session, "dev_handle_deep_link", {"url": deep_link})
            deadline = time.time() + 20
            while time.time() < deadline:
                state = js_invoke(session, "get_state", {})
                if state["auth"]["connected"]:
                    break
                time.sleep(0.2)
            log("auth:", {"connected": state["auth"]["connected"], "email": state["auth"]["email"]})

        assert state["auth"]["connected"], "app is not connected"
        assert not state["mock"], "E2E should not use the auto-login demo mode"

        dom = json.loads(
            run_async(
                session,
                """
                const done = arguments[arguments.length - 1];
                document.getElementById('btnInfo').click();
                const modal = document.getElementById('infoModal');
                const info = {
                  account: document.getElementById('accountChip').textContent,
                  rows: document.querySelectorAll('.gallery-row').length,
                  tabs: document.querySelectorAll('.tab').length,
                  syncLabel: document.getElementById('btnSync').textContent.trim(),
                  emptyHidden: document.getElementById('syncEmpty').classList.contains('hidden'),
                  badge: document.getElementById('envBadge').textContent,
                  badgeHidden: document.getElementById('envBadge').classList.contains('hidden'),
                  infoVisible: !modal.classList.contains('hidden'),
                  infoText: modal.innerText,
                  title: document.title
                };
                document.getElementById('btnInfoClose').click();
                done(JSON.stringify(info));
                """,
            )
        )
        log("dom:", {k: v for k, v in dom.items() if k != "infoText"})
        assert dom["rows"] >= 1, "renderer did not draw the gallery list"
        assert dom["account"].strip(), "renderer did not show the account chip"
        assert dom["emptyHidden"], "empty state should be hidden with a linked gallery"
        assert dom["tabs"] == 0, "the desktop UI must not have web-style tabs"
        assert dom["syncLabel"].lower() == "sync", dom["syncLabel"]
        assert dom["title"] == "Picstome Sync", dom["title"]
        assert dom["infoVisible"], "info modal did not open"
        assert state["config"]["apiBase"] in dom["infoText"], "info modal must show the API URL"
        assert state["config"]["appVersion"] in dom["infoText"], "info modal must show the version"
        kind = state["config"]["apiKind"]
        expected_badge = {
            "production": "PROD",
            "development": "DEV",
            "local": "LOCAL",
            "demo": "DEMO",
            "custom": "CUSTOM",
        }[kind]
        assert dom["badge"] == expected_badge, (kind, dom["badge"])
        assert not dom["badgeHidden"], "environment badge should always be visible"

        if MOCK:
            search_state = json.loads(
                run_async(
                    session,
                    """
                    const done = arguments[arguments.length - 1];
                    const input = document.getElementById('gallerySearch');
                    const setQuery = (value) => {
                      input.value = value;
                      input.dispatchEvent(new Event('input', { bubbles: true }));
                    };
                    setQuery('Other');
                    const otherRows = document.querySelectorAll('.gallery-row').length;
                    setQuery('zzz');
                    const emptyVisible = !document.getElementById('searchEmpty').classList.contains('hidden');
                    setQuery('');
                    const allRows = document.querySelectorAll('.gallery-row').length;
                    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true }));
                    const focused = document.activeElement && document.activeElement.id;
                    done(JSON.stringify({ otherRows, emptyVisible, allRows, focused }));
                    """,
                )
            )
            log("search:", search_state)
            assert search_state["otherRows"] == 1, search_state
            assert search_state["emptyVisible"], search_state
            assert search_state["allRows"] == 2, search_state
            assert search_state["focused"] == "gallerySearch", search_state
            run_async(
                session,
                "const done = arguments[arguments.length - 1]; document.getElementById('gallerySearch').blur(); done('blurred');",
            )

        first = js_invoke(session, "sync_gallery", {"galleryId": gallery})
        log("first:", first["counts"], "errors:", first["errors"])
        assert not first["errors"], first["errors"]
        assert len(first["downloaded"]) == EXPECTED, first["counts"]
        files = [name for name in os.listdir(photos) if name.endswith(".jpg")]
        assert len(files) == EXPECTED, f"expected {EXPECTED} files, got {len(files)}"
        assert os.path.exists(os.path.join(user_data, f"{gallery}.json")), "snapshot missing"

        second = js_invoke(session, "sync_gallery", {"galleryId": gallery})
        log("second:", second["counts"], "skipped:", second["skipped"])
        assert second["skipped"] and not second["downloaded"], second["counts"]

        if WRITE:
            source = next(name for name in files if name.endswith(".jpg"))
            upload_name = "e2e-upload.jpg"
            with open(os.path.join(photos, source), "rb") as src, open(
                os.path.join(photos, upload_name), "wb"
            ) as out:
                out.write(src.read())

            third = js_invoke(session, "sync_gallery", {"galleryId": gallery})
            log("upload:", third["counts"], "errors:", third["errors"])
            assert not third["errors"], third["errors"]
            assert len(third["uploaded"]) == 1, third["counts"]
            sync_log_path = os.path.join(user_data, "logs", f"{gallery}.log")
            assert os.path.exists(sync_log_path), "per-gallery log was not written"
            with open(sync_log_path, encoding="utf-8") as handle:
                sync_log = handle.read()
            assert upload_name in sync_log, sync_log[-500:]
            log_entries = js_invoke(
                session, "get_sync_log", {"galleryId": gallery}
            )
            assert any(
                item.get("filename") == upload_name
                for entry in log_entries
                for item in entry.get("uploaded", [])
            ), log_entries
            log_dom = json.loads(
                run_async(
                    session,
                    """
                    const done = arguments[arguments.length - 1];
                    const card = document.getElementById('syncLog');
                    done(JSON.stringify({
                      exists: !!card,
                      count: document.getElementById('syncLogCount').textContent
                    }));
                    """,
                )
            )
            assert log_dom["exists"], log_dom
            assert int(log_dom["count"] or 0) > 0, log_dom

            os.remove(os.path.join(photos, upload_name))
            run_async(
                session,
                "const done = arguments[arguments.length - 1]; document.getElementById('btnSync').click(); done('clicked');",
            )
            deadline = time.time() + 60
            proposal_state = None
            while time.time() < deadline:
                proposal_state = json.loads(
                    run_async(
                        session,
                        """
                        const done = arguments[arguments.length - 1];
                        const checks = Array.from(
                          document.querySelectorAll('#deleteList input[type=checkbox]')
                        );
                        done(JSON.stringify({
                          visible: !document.getElementById('deleteBox').classList.contains('hidden'),
                          ids: checks.map((item) => item.dataset.photoId),
                          names: checks.map((item) => item.nextElementSibling.textContent)
                        }));
                        """,
                    )
                )
                if proposal_state["visible"]:
                    break
                time.sleep(0.2)
            log("proposal:", proposal_state)
            assert proposal_state and proposal_state["visible"], "deletion proposal did not render"
            ids = proposal_state["ids"]
            assert len(ids) == 1, proposal_state

            run_async(
                session,
                "const done = arguments[arguments.length - 1]; document.getElementById('btnDeleteRemote').click(); done('clicked');",
            )
            confirm_state = json.loads(
                run_async(
                    session,
                    """
                    const done = arguments[arguments.length - 1];
                    done(JSON.stringify({
                      visible: !document.getElementById('confirmModal').classList.contains('hidden'),
                      message: document.getElementById('confirmMessage').textContent
                    }));
                    """,
                )
            )
            log("remote delete confirmation:", confirm_state)
            assert confirm_state["visible"], "remote delete confirmation did not open"
            assert "1" in confirm_state["message"], confirm_state

            run_async(
                session,
                "const done = arguments[arguments.length - 1]; document.getElementById('btnConfirmCancel').click(); done('cancelled');",
            )
            cancelled_state = json.loads(
                run_async(
                    session,
                    """
                    const done = arguments[arguments.length - 1];
                    done(JSON.stringify({
                      confirm: !document.getElementById('confirmModal').classList.contains('hidden'),
                      deleteBox: !document.getElementById('deleteBox').classList.contains('hidden')
                    }));
                    """,
                )
            )
            assert not cancelled_state["confirm"], cancelled_state
            assert cancelled_state["deleteBox"], "cancel must keep the deletion proposal"

            run_async(
                session,
                "const done = arguments[arguments.length - 1]; document.getElementById('btnDeleteRemote').click(); done('clicked');",
            )
            run_async(
                session,
                "const done = arguments[arguments.length - 1]; document.getElementById('btnConfirmOk').click(); done('confirmed');",
            )
            deadline = time.time() + 30
            while time.time() < deadline:
                deleted_state = json.loads(
                    run_async(
                        session,
                        """
                        const done = arguments[arguments.length - 1];
                        done(JSON.stringify({
                          deleteBox: !document.getElementById('deleteBox').classList.contains('hidden')
                        }));
                        """,
                    )
                )
                if not deleted_state["deleteBox"]:
                    break
                time.sleep(0.2)
            assert not deleted_state["deleteBox"], "confirmed remote deletion did not run"
            deletion_log = js_invoke(session, "get_sync_log", {"galleryId": gallery})
            assert any(
                entry.get("kind") == "deletion"
                and entry.get("action") == "remote_delete"
                and entry.get("resolved") == 1
                for entry in deletion_log
            ), deletion_log
            log_text = json.loads(
                run_async(
                    session,
                    """
                    const done = arguments[arguments.length - 1];
                    done(JSON.stringify(
                      document.getElementById('syncLogEntries').textContent
                    ));
                    """,
                )
            )
            assert "Deleted in Picstome" in log_text, log_text
            assert upload_name in log_text, log_text

            final = js_invoke(session, "sync_gallery", {"galleryId": gallery})
            log("final:", final["counts"], "skipped:", final["skipped"])
            assert final["skipped"], final["counts"]

        if MOCK:
            # Remote deletion: Sync must ask what to do and never re-upload silently.
            manifest_request = urllib.request.Request(
                f"{api_base}/api/v1/manifest?gallery=G123",
                headers={
                    "Accept": "application/json",
                    "Authorization": "Bearer demo-token",
                },
            )
            with urllib.request.urlopen(manifest_request, timeout=30) as response:
                remote_manifest = json.loads(response.read())
            removed = remote_manifest["photos"][0]
            remove_request = urllib.request.Request(
                f"{api_base}/api/v1/dev/remove",
                data=json.dumps(
                    {"gallery_id": "G123", "photo_id": removed["id"]}
                ).encode(),
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            urllib.request.urlopen(remove_request, timeout=30).read()

            run_async(
                session,
                "const done = arguments[arguments.length - 1]; document.getElementById('btnSync').click(); done('clicked');",
            )
            deadline = time.time() + 60
            remote_prompt = None
            while time.time() < deadline:
                remote_prompt = json.loads(
                    run_async(
                        session,
                        """
                        const done = arguments[arguments.length - 1];
                        const modal = document.getElementById('remoteDeletedModal');
                        done(JSON.stringify({
                          visible: !modal.classList.contains('hidden'),
                          title: document.getElementById('remoteDeletedTitle').textContent,
                          hint: document.getElementById('remoteDeletedHint').textContent,
                          list: document.getElementById('remoteDeletedList').textContent
                        }));
                        """,
                    )
                )
                if remote_prompt["visible"]:
                    break
                time.sleep(0.4)
            log("remote deletion prompt:", remote_prompt)
            assert remote_prompt and remote_prompt["visible"], "remote deletion prompt did not open"
            assert "deleted" in remote_prompt["title"].lower(), remote_prompt
            assert removed["filename"] in remote_prompt["list"], remote_prompt

            removed_path = os.path.join(photos, removed["filename"])
            assert os.path.exists(removed_path), "photo should exist before resolving"
            run_async(
                session,
                "const done = arguments[arguments.length - 1]; document.getElementById('btnRemoteDeleteLocal').click(); done('clicked');",
            )
            local_confirm = json.loads(
                run_async(
                    session,
                    """
                    const done = arguments[arguments.length - 1];
                    done(JSON.stringify({
                      visible: !document.getElementById('confirmModal').classList.contains('hidden'),
                      message: document.getElementById('confirmMessage').textContent
                    }));
                    """,
                )
            )
            log("local delete confirmation:", local_confirm)
            assert local_confirm["visible"], "local delete confirmation did not open"
            run_async(
                session,
                "const done = arguments[arguments.length - 1]; document.getElementById('btnConfirmOk').click(); done('confirmed');",
            )
            deadline = time.time() + 30
            while time.time() < deadline:
                state_after = json.loads(
                    run_async(
                        session,
                        """
                        const done = arguments[arguments.length - 1];
                        done(JSON.stringify({
                          remote: !document.getElementById('remoteDeletedModal').classList.contains('hidden'),
                          rating: !document.getElementById('ratingModal').classList.contains('hidden')
                        }));
                        """,
                    )
                )
                if not state_after["remote"]:
                    break
                time.sleep(0.2)
            assert not os.path.exists(removed_path), "Delete locally should remove the file"
            local_delete_log = js_invoke(session, "get_sync_log", {"galleryId": gallery})
            local_delete_entry = next(
                (
                    entry
                    for entry in local_delete_log
                    if entry.get("kind") == "deletion"
                    and entry.get("action") == "delete_local"
                ),
                None,
            )
            assert local_delete_entry, local_delete_log
            assert local_delete_entry.get("trashed"), local_delete_entry
            for trashed in local_delete_entry["trashed"]:
                assert trashed["location"] in {"system", "gallery"}, trashed
                if trashed["location"] == "gallery":
                    assert os.path.exists(trashed["trashPath"]), trashed
            log_text = json.loads(
                run_async(
                    session,
                    """
                    const done = arguments[arguments.length - 1];
                    done(JSON.stringify(
                      document.getElementById('syncLogEntries').textContent
                    ));
                    """,
                )
            )
            assert "Moved local files to trash" in log_text, log_text
            assert removed["filename"] in log_text, log_text
            run_async(
                session,
                "const done = arguments[arguments.length - 1]; if (!document.getElementById('ratingModal').classList.contains('hidden')) document.getElementById('btnRatingClose').click(); done('closed');",
            )
            after_remote = js_invoke(session, "sync_gallery", {"galleryId": gallery})
            log("after remote delete:", after_remote["counts"], "skipped:", after_remote["skipped"])
            assert not after_remote["uploaded"], "remote-deleted photo must not be uploaded"
            assert not after_remote["remoteDeleted"], after_remote["remoteDeleted"]

        # Rating modal: deep link with no gallery must fall back to the selected gallery.
        rate_name = next(
            name
            for name in files
            if name.endswith(".jpg") and os.path.exists(os.path.join(photos, name))
        )
        rate_stem = os.path.splitext(rate_name)[0]
        js_invoke(
            session,
            "dev_handle_deep_link",
            {"url": f"picstome://rate?files={rate_stem}&rating=4"},
        )
        rating_target = run_async(
            session,
            """
            const done = arguments[arguments.length - 1];
            done(document.getElementById('ratingTarget').textContent);
            """,
        )
        log("rating target:", rating_target)
        assert photos in rating_target, rating_target
        rate_result = run_async(
            session,
            """
            const done = arguments[arguments.length - 1];
            const btn = document.getElementById('btnRatingApply');
            if (btn.disabled) { done('DISABLED'); }
            else {
              btn.click();
              const deadline = Date.now() + 30000;
              const timer = setInterval(() => {
                const result = document.getElementById('ratingResult');
                const visible = !result.classList.contains('hidden');
                if (visible && result.textContent) {
                  clearInterval(timer);
                  done(result.textContent);
                } else if (Date.now() > deadline) {
                  clearInterval(timer);
                  done('TIMEOUT');
                }
              }, 200);
            }
            """,
        )
        log("rate:", rate_result.splitlines()[0] if rate_result else rate_result)
        assert not rate_result.startswith("DISABLED"), rate_result
        assert "1 file" in rate_result.lower() and "rated" in rate_result.lower(), rate_result
        deadline = time.time() + 10
        while time.time() < deadline:
            modal_state = run_async(
                session,
                "const done = arguments[arguments.length - 1]; done(JSON.stringify({ hidden: document.getElementById('ratingModal').classList.contains('hidden') }));",
            )
            if json.loads(modal_state)["hidden"]:
                break
            time.sleep(0.2)

        # Favorites: Sync from the UI, detect remote favorites and apply a rating.
        run_async(
            session,
            "const done = arguments[arguments.length - 1]; document.getElementById('btnSync').click(); done('clicked');",
        )
        deadline = time.time() + 60
        favorite_prompt = None
        while time.time() < deadline:
            favorite_prompt = json.loads(
                run_async(
                    session,
                    """
                    const done = arguments[arguments.length - 1];
                    const modal = document.getElementById('ratingModal');
                    done(JSON.stringify({
                      visible: !modal.classList.contains('hidden'),
                      title: document.getElementById('ratingTitle').textContent,
                      subtitle: document.getElementById('ratingSubtitle').textContent
                    }));
                    """,
                )
            )
            if favorite_prompt["visible"] and "favorite" in favorite_prompt["title"].lower():
                break
            time.sleep(0.4)
        log("favorites prompt:", favorite_prompt)
        assert favorite_prompt and favorite_prompt["visible"], "favorites prompt did not open"
        assert "favorite" in favorite_prompt["title"].lower(), favorite_prompt

        favorites_result = run_async(
            session,
            """
            const done = arguments[arguments.length - 1];
            document.querySelector('#ratingStars .star-btn[data-rating="4"]').click();
            document.getElementById('btnRatingApply').click();
            const deadline = Date.now() + 30000;
            const timer = setInterval(() => {
              const result = document.getElementById('ratingResult');
              if (!result.classList.contains('hidden') && result.textContent) {
                clearInterval(timer);
                done(result.textContent);
              } else if (Date.now() > deadline) {
                clearInterval(timer);
                done('TIMEOUT');
              }
            }, 200);
            """,
        )
        log("favorites rating:", favorites_result.splitlines()[0] if favorites_result else favorites_result)
        assert "5 file" in favorites_result.lower() and "rated" in favorites_result.lower(), favorites_result
        deadline = time.time() + 10
        while time.time() < deadline:
            modal_state = run_async(
                session,
                "const done = arguments[arguments.length - 1]; done(JSON.stringify({ hidden: document.getElementById('ratingModal').classList.contains('hidden') }));",
            )
            if json.loads(modal_state)["hidden"]:
                break
            time.sleep(0.2)
        assert json.loads(modal_state)["hidden"], "favorites modal should close automatically"

        with open(os.path.join(user_data, f"{gallery}.json"), encoding="utf-8") as handle:
            snapshot = json.load(handle)
        rated_favorites = [
            file for file in snapshot["files"] if file.get("favoriteRatedFor")
        ]
        assert len(rated_favorites) >= 5, rated_favorites
        rating_log = js_invoke(session, "get_sync_log", {"galleryId": gallery})
        assert any(
            entry.get("kind") == "rating"
            and entry.get("rating") == "4"
            and entry.get("source") == "favorites"
            and entry.get("rated", 0) >= 5
            for entry in rating_log
        ), rating_log
        after_rating = js_invoke(session, "sync_gallery", {"galleryId": gallery})
        log("after favorites:", after_rating["counts"], "favorites:", len(after_rating["favorites"]))
        assert not after_rating["favorites"], after_rating["favorites"]

        cleared = js_invoke(session, "clear_sync_log", {"galleryId": gallery})
        assert cleared.get("ok"), cleared
        assert not os.path.exists(
            os.path.join(user_data, "logs", f"{gallery}.log")
        ), "per-gallery log was not removed"
        assert not js_invoke(session, "get_sync_log", {"galleryId": gallery})

        # A gallery deleted on the web must not delete local files. The app asks
        # whether to remove it locally; Cancel keeps it, Accept unlinks it.
        remove_gallery_request = urllib.request.Request(
            f"{api_base}/api/v1/dev/remove-gallery",
            data=json.dumps({"gallery_id": "G456"}).encode(),
            headers={
                "Content-Type": "application/json",
                "Authorization": "Bearer demo-token",
            },
            method="POST",
        )
        urllib.request.urlopen(remove_gallery_request, timeout=30).read()
        run_async(
            session,
            """
            const done = arguments[arguments.length - 1];
            const row = Array.from(document.querySelectorAll('.gallery-row'))
              .find((item) => item.dataset.galleryId === 'G456');
            row.click();
            document.getElementById('btnSync').click();
            done('clicked');
            """,
        )
        deadline = time.time() + 30
        missing_prompt = None
        while time.time() < deadline:
            missing_prompt = json.loads(
                run_async(
                    session,
                    """
                    const done = arguments[arguments.length - 1];
                    done(JSON.stringify({
                      visible: !document.getElementById('confirmModal').classList.contains('hidden'),
                      title: document.getElementById('confirmTitle').textContent,
                      message: document.getElementById('confirmMessage').textContent
                    }));
                    """,
                )
            )
            if missing_prompt["visible"]:
                break
            time.sleep(0.2)
        log("missing gallery prompt:", missing_prompt)
        assert missing_prompt and missing_prompt["visible"], "missing gallery prompt did not open"
        assert "not found" in missing_prompt["title"].lower(), missing_prompt
        assert "nothing will be deleted" in missing_prompt["message"].lower(), missing_prompt

        run_async(
            session,
            "const done = arguments[arguments.length - 1]; document.getElementById('btnConfirmCancel').click(); done('cancelled');",
        )
        time.sleep(0.5)
        state_after_cancel = js_invoke(session, "get_state", {})
        assert any(
            item["galleryId"] == "G456" for item in state_after_cancel["galleries"]
        ), state_after_cancel["galleries"]

        run_async(
            session,
            "const done = arguments[arguments.length - 1]; document.getElementById('btnSync').click(); done('clicked');",
        )
        deadline = time.time() + 30
        while time.time() < deadline:
            missing_prompt = json.loads(
                run_async(
                    session,
                    "const done = arguments[arguments.length - 1]; done(JSON.stringify({ visible: !document.getElementById('confirmModal').classList.contains('hidden') }));",
                )
            )
            if missing_prompt["visible"]:
                break
            time.sleep(0.2)
        assert missing_prompt and missing_prompt["visible"], "missing gallery prompt did not reopen"
        run_async(
            session,
            "const done = arguments[arguments.length - 1]; document.getElementById('btnConfirmOk').click(); done('confirmed');",
        )
        deadline = time.time() + 30
        while time.time() < deadline:
            state_after_remove = js_invoke(session, "get_state", {})
            if not any(
                item["galleryId"] == "G456"
                for item in state_after_remove["galleries"]
            ):
                break
            time.sleep(0.2)
        assert not any(
            item["galleryId"] == "G456" for item in state_after_remove["galleries"]
        ), state_after_remove["galleries"]
        assert os.path.isdir(rating_dir), "removing a gallery must not touch local files"

        logout = js_invoke(session, "logout", {})
        assert logout.get("ok"), logout
        state = js_invoke(session, "get_state", {})
        assert not state["auth"]["connected"], "logout did not disconnect"

        # Switch the test target from the Info modal (no network call needed).
        run_async(
            session,
            """
            const done = arguments[arguments.length - 1];
            document.getElementById('btnInfo').click();
            document.getElementById('btnApiProduction').click();
            done('clicked');
            """,
        )
        deadline = time.time() + 20
        while time.time() < deadline:
            state = js_invoke(session, "get_state", {})
            if state["config"]["apiBase"] == "https://app.picstome.com":
                break
            time.sleep(0.2)
        log(
            "switched target:",
            {
                "apiBase": state["config"]["apiBase"],
                "apiKind": state["config"]["apiKind"],
                "connected": state["auth"]["connected"],
            },
        )
        assert state["config"]["apiBase"] == "https://app.picstome.com", state["config"]
        assert state["config"]["apiKind"] == "production", state["config"]
        assert not state["auth"]["connected"], "switching targets must sign out"
        log("E2E PASSED")
    finally:
        if session:
            try:
                request("DELETE", f"/session/{session}")
            except Exception:
                pass
        driver.terminate()
        try:
            driver.wait(timeout=10)
        except Exception:
            driver.kill()
        driver_log.close()
        if mock_process:
            mock_process.terminate()
            try:
                mock_process.wait(timeout=10)
            except Exception:
                mock_process.kill()


if __name__ == "__main__":
    try:
        main()
    except Exception as err:
        log("E2E FAILED:", err)
        try:
            code = DRIVER_PROCESS.poll() if DRIVER_PROCESS else None
            log("driver exit:", code)
            path = DRIVER_LOG_PATH
            if path:
                with open(path, "r", encoding="utf-8", errors="replace") as handle:
                    tail = handle.read().splitlines()[-30:]
                if tail:
                    log("driver log:\n" + "\n".join(tail))
        except Exception:
            pass
        sys.exit(1)
