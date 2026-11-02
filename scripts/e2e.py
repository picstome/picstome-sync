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


def run_async(session, script):
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
    with open(os.path.join(user_data, "index.json"), "w", encoding="utf-8") as handle:
        json.dump(
            [
                {
                    "galleryId": gallery,
                    "name": "E2E Gallery",
                    "localPath": photos,
                }
            ],
            handle,
        )
    with open(os.path.join(user_data, "config.json"), "w", encoding="utf-8") as handle:
        json.dump({"photosDir": rating_dir}, handle)


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

    driver_log_path = os.path.join(root, "driver.log")
    driver_log = open(driver_log_path, "w", encoding="utf-8")
    driver = subprocess.Popen(
        [DRIVER, "--port", str(DEBUG_PORT), "--native-driver", NATIVE_DRIVER],
        env=env,
        stdout=driver_log,
        stderr=subprocess.STDOUT,
    )
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
                  emptyHidden: document.getElementById('syncEmpty').classList.contains('hidden'),
                  demoBadge: !document.getElementById('mockBadge').classList.contains('hidden'),
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
        assert dom["demoBadge"] is False, "demo badge should not be shown"
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

            os.remove(os.path.join(photos, upload_name))
            proposal = js_invoke(session, "sync_gallery", {"galleryId": gallery})
            log("proposal:", proposal["counts"], "errors:", proposal["errors"])
            assert not proposal["errors"], proposal["errors"]
            ids = [item["id"] for item in proposal["proposals"]["deleteRemote"]]
            assert len(ids) == 1, proposal["proposals"]

            deleted = js_invoke(
                session, "apply_deletions", {"galleryId": gallery, "photoIds": ids}
            )
            log("deleted:", deleted["deleted"], "notFound:", deleted["not_found"])
            assert len(deleted["deleted"]) == 1, deleted

            final = js_invoke(session, "sync_gallery", {"galleryId": gallery})
            log("final:", final["counts"], "skipped:", final["skipped"])
            assert final["skipped"], final["counts"]

        # Rate tab: deep link + real exiftool write on a separate folder.
        rating_name = "rating-photo.jpg"
        fixture = os.path.join(ROOT, "src-tauri", "tests", "fixtures", "pixel.jpg")
        with open(fixture, "rb") as src, open(
            os.path.join(rating_dir, rating_name), "wb"
        ) as out:
            out.write(src.read())
        js_invoke(
            session,
            "dev_handle_deep_link",
            {"url": "picstome://rate?files=rating-photo&rating=4"},
        )
        rate_result = run_async(
            session,
            """
            const done = arguments[arguments.length - 1];
            document.querySelector('.tab[data-tab="rate"]').click();
            const btn = document.getElementById('btnRate');
            if (btn.disabled) { done('DISABLED'); }
            else {
              btn.click();
              const deadline = Date.now() + 30000;
              const timer = setInterval(() => {
                const summary = document.getElementById('resultSummary');
                const visible = !document.getElementById('results').classList.contains('hidden');
                if (visible && summary.textContent) {
                  clearInterval(timer);
                  done(summary.textContent + ' | ' + document.getElementById('detailList').textContent);
                } else if (Date.now() > deadline) {
                  clearInterval(timer);
                  done('TIMEOUT ' + summary.textContent);
                }
              }, 200);
            }
            """,
        )
        log("rate:", rate_result)
        assert not rate_result.startswith("DISABLED"), rate_result
        assert "1 file" in rate_result.lower() and "rated" in rate_result.lower(), rate_result

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
            with open(driver_log_path, "r", encoding="utf-8", errors="replace") as handle:
                tail = handle.read().splitlines()[-20:]
            if tail:
                log("driver log:\n" + "\n".join(tail))
        except Exception:
            pass
        sys.exit(1)
