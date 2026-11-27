#!/usr/bin/env python3
"""End-to-end test for the real Tauri app through WebDriver.

Drives the review -> apply flow the way a photographer does: press "Review
changes", choose per row, press Apply, and check the disk and the Picstome
side after every step.

Mock mode (default):
    E2E_MOCK=1 xvfb-run -a python3 scripts/e2e.py
  Starts `target/debug/picstome-sync-mock` and covers: a review moves
  nothing; partial apply (the rest comes back next review); upload; a local
  deletion is proposed (never re-downloaded) and deleting in Picstome asks
  first; a photo deleted in Picstome goes to the trash with its RAW; stars
  photo by photo; a file edited after the review is skipped; the mass-delete
  guard; a picstome://sync link opens the review without applying.

Real API mode (read-only unless E2E_WRITE=1):
    E2E_GALLERY=<ulid> PICSTOME_API_BASE=https://devel.picstome.com \
    PICSTOME_TOKEN=... E2E_MOCK=0 python3 scripts/e2e.py

Requirements: `cargo build -p picstome-sync` and the mock binary,
tauri-driver, WebKitWebDriver, and a display (xvfb-run on headless Linux).
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


# --- UI helpers ------------------------------------------------------------
#
# `ui` runs a snippet in the app window; the snippet calls `done(value)` and
# gets the value back as JSON. The renderer's top-level `state` is reachable
# from here, which is how the tests wait for a review or a report.


def ui(session, body):
    script = """
    const callback = arguments[arguments.length - 1];
    const done = (value) => callback(JSON.stringify(value === undefined ? null : value));
    try {
    %s
    } catch (error) {
      done({ __error: String(error && (error.stack || error)) });
    }
    """ % body
    value = json.loads(run_async(session, script))
    if isinstance(value, dict) and "__error" in value:
        raise RuntimeError(value["__error"])
    return value


def wait_for(session, body, timeout=90):
    """Poll a snippet until it returns something truthy."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = ui(session, body)
        if value:
            return value
        time.sleep(0.25)
    raise RuntimeError(f"timed out waiting for: {body.strip()[:120]}")


def click(session, selector):
    ok = ui(
        session,
        "const node = document.querySelector(%s); if (node) node.click(); done(Boolean(node));"
        % json.dumps(selector),
    )
    assert ok, f"nothing to click at {selector}"


def _choose(session, selector, value):
    ok = ui(
        session,
        """
        const select = document.querySelector(%s);
        if (!select) { done(false); } else {
          select.value = %s;
          select.dispatchEvent(new Event('change', { bubbles: true }));
          done(select.value === %s);
        }
        """
        % (json.dumps(selector), json.dumps(value), json.dumps(value)),
    )
    assert ok, f"could not choose {value} in {selector}"


def set_row(session, group, row_id, value):
    _choose(session, f'select[data-group="{group}"][data-id="{row_id}"]', value)


def set_group(session, group, value):
    _choose(session, f'select.review-all[data-group="{group}"]', value)


def has_group(session, group):
    return ui(
        session,
        "done(Boolean(document.querySelector('select.review-all[data-group=\"%s\"]')));" % group,
    )


def open_gallery(session, gallery_id):
    click(session, f'.gallery-row[data-gallery-id="{gallery_id}"]')
    wait_for(session, "done(state.currentGalleryId === %s);" % json.dumps(gallery_id))


def review_ui(session):
    """Press "Review changes" and return the plan, or None when in sync."""
    click(session, "#btnSync")
    result = wait_for(
        session,
        """
        if (state.syncBusy || !(state.review || state.report)) { done(null); }
        else done({ plan: state.review ? state.review.plan : null, report: state.report });
        """,
    )
    errors = (result.get("report") or {}).get("errors") or []
    assert not errors, f"review failed: {errors}"
    return result["plan"]


def wait_report(session):
    report = wait_for(
        session,
        "done(!state.syncBusy && !state.review && state.report ? state.report : null);",
        timeout=180,
    )
    assert not report.get("errors"), f"apply failed: {report.get('errors')}"
    return report


def apply_ui(session):
    """Press Apply (confirming a Picstome deletion if asked) and return the report."""
    click(session, "#btnReviewApply")
    time.sleep(0.3)
    if ui(session, "done(!document.getElementById('confirmModal').classList.contains('hidden'));"):
        click(session, "#btnConfirmOk")
    return wait_report(session)


def discard_ui(session):
    ui(
        session,
        "const button = document.getElementById('btnReviewDiscard');"
        " if (!document.getElementById('reviewBar').classList.contains('hidden')) button.click();"
        " done(true);",
    )
    wait_for(session, "done(state.review === null);")


def summarize(plan):
    if not plan:
        return None
    return {
        key: len(plan.get(key) or [])
        for key in ("downloads", "uploads", "remoteDeleted", "localDeleted", "conflicts")
    } | {"ratings": len(plan["ratings"]["items"]), "rawDecision": plan.get("rawDecisionNeeded")}


def report_summary(report):
    return {
        "downloaded": [item["filename"] for item in report.get("downloaded") or []],
        "uploaded": [item["filename"] for item in report.get("uploaded") or []],
        "deletedRemote": report.get("deletedRemote"),
        "skippedChanged": report.get("skippedChanged"),
        "errors": report.get("errors"),
    }


def finish_without_writes(session):
    log("E2E PASSED (read-only)")


# --- Disk and fake Picstome helpers ------------------------------------------


def photo_files(folder):
    return {
        name
        for name in os.listdir(folder)
        if not name.startswith(".") and os.path.isfile(os.path.join(folder, name))
    }


def write_jpeg(path, size, seed):
    body = bytes((seed * 31 + index) % 251 for index in range(max(0, size - 4)))
    with open(path, "wb") as handle:
        handle.write(b"\xff\xd8\xff\xe0" + body)


def _mock_request(url, body=None):
    request_ = urllib.request.Request(
        url,
        data=json.dumps(body).encode() if body is not None else None,
        headers={
            "Accept": "application/json",
            "Content-Type": "application/json",
            "Authorization": "Bearer demo-token",
        },
        method="POST" if body is not None else "GET",
    )
    with urllib.request.urlopen(request_, timeout=30) as response:
        return json.loads(response.read() or b"null")


def mock_manifest(api_base):
    return _mock_request(f"{api_base}/api/v1/manifest?gallery=G123")


def mock_remove(api_base, photo_id):
    return _mock_request(f"{api_base}/api/v1/dev/remove", {"gallery_id": "G123", "photo_id": photo_id})


def mock_add(api_base, filename):
    return _mock_request(f"{api_base}/api/v1/dev/add", {"gallery_id": "G123", "filename": filename})


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
    env.setdefault("PICSTOME_DISABLE_UPDATE_CHECK", "1")
    # Headless runners have no unlocked Secret Service. The app then uses its
    # 0600 auth.json fallback, which is exactly what these tests exercise.
    env.setdefault("PICSTOME_DISABLE_KEYRING", "1")
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

        # ------------------------------------------------------------------
        # Review → Apply. Every step drives the real UI (clicks, selects) and
        # checks the disk and the Picstome side, not only the app state.
        # ------------------------------------------------------------------

        home = ui(
            session,
            """
            done({
              title: document.getElementById('detailName').textContent,
              homeVisible: !document.getElementById('home').classList.contains('hidden'),
              rows: document.querySelectorAll('.gallery-row').length,
              accountMenuItems: document.querySelectorAll('#accountMenu [role=menuitem]').length,
              syncLabel: document.getElementById('btnSync').textContent.trim(),
              title_: document.title
            });
            """,
        )
        log("home:", home)
        assert home["homeVisible"], "the app must open on the home page"
        assert home["rows"] >= 1, "linked galleries must be listed"
        assert home["accountMenuItems"] >= 3, "account menu: profile, info, log out"
        assert home["title_"] == "Picstome Sync", home

        open_gallery(session, gallery)
        label = ui(session, "done(document.getElementById('btnSync').textContent.trim());")
        assert label in ("Review changes", "Revisar cambios"), label

        # 1. Reviewing moves nothing.
        before = photo_files(photos)
        plan = review_ui(session)
        log("first review:", summarize(plan))
        assert plan is not None, "first review must list changes"
        assert plan["firstSync"], plan["firstSync"]
        assert photo_files(photos) == before, "a review must not touch the disk"
        if not MOCK:
            # Real API: read-only from here on unless E2E_WRITE=1.
            discard_ui(session)
            if not WRITE:
                log("real API, read-only: review checked, nothing applied")
                return finish_without_writes(session)
        expected_downloads = len(plan["downloads"])
        assert expected_downloads >= 1, plan

        # 2. Apply only part of the review: 5 downloads, everything else later.
        set_group(session, "downloads", "later")
        if plan["ratings"]["items"] or plan["ratings"]["needsRating"]:
            set_group(session, "ratings", "later")
        first_ids = [item["id"] for item in plan["downloads"][:5]]
        for photo_id in first_ids:
            set_row(session, "downloads", photo_id, "apply")
        report = apply_ui(session)
        log("partial apply:", report_summary(report))
        assert len(report["downloaded"]) == 5, report_summary(report)
        assert len(photo_files(photos)) == len(before) + 5, sorted(photo_files(photos))

        # What was left for later comes back in the next review.
        plan = review_ui(session)
        assert len(plan["downloads"]) == expected_downloads - 5, summarize(plan)
        set_group(session, "ratings", "later") if has_group(session, "ratings") else None
        report = apply_ui(session)
        assert len(report["downloaded"]) == expected_downloads - 5, report_summary(report)
        assert len(photo_files(photos)) == len(before) + expected_downloads

        if not MOCK:
            return finish_without_writes(session)

        # 3. Upload a new local photo.
        local_new = os.path.join(photos, "LOCAL_NEW.jpg")
        write_jpeg(local_new, 4096, 7)
        plan = review_ui(session)
        names = [item["filename"] for item in plan["uploads"]]
        assert names == ["LOCAL_NEW.jpg"], summarize(plan)
        set_group(session, "ratings", "later") if has_group(session, "ratings") else None
        report = apply_ui(session)
        assert [item["filename"] for item in report["uploaded"]] == ["LOCAL_NEW.jpg"], report_summary(report)
        assert any(p["filename"] == "LOCAL_NEW.jpg" for p in mock_manifest(api_base)["photos"])

        # 4. Deleted on this computer: proposed, never re-downloaded behind our
        #    back; deleting in Picstome asks for confirmation.
        os.remove(local_new)
        plan = review_ui(session)
        assert [item["filename"] for item in plan["localDeleted"]] == ["LOCAL_NEW.jpg"], summarize(plan)
        assert not plan["downloads"], "a photo deleted here must not be re-downloaded"
        deleted_id = plan["localDeleted"][0]["id"]
        set_row(session, "localDeleted", deleted_id, "delete")
        set_group(session, "ratings", "later") if has_group(session, "ratings") else None
        click(session, "#btnReviewApply")
        confirm = wait_for(
            session,
            "done(!document.getElementById('confirmModal').classList.contains('hidden')"
            " ? document.getElementById('confirmMessage').textContent : null);",
        )
        log("delete confirmation:", confirm)
        click(session, "#btnConfirmOk")
        report = wait_report(session)
        assert report["deletedRemote"] == ["LOCAL_NEW.jpg"], report_summary(report)
        assert not any(p["filename"] == "LOCAL_NEW.jpg" for p in mock_manifest(api_base)["photos"])

        # 5. Deleted in Picstome, still here with its RAW: trash both.
        manifest = mock_manifest(api_base)
        victim = manifest["photos"][0]
        stem = os.path.splitext(victim["filename"])[0]
        victim_path = os.path.join(photos, victim["filename"])
        raw_path = os.path.join(photos, stem + ".CR3")
        with open(raw_path, "wb") as handle:
            handle.write(b"II*\0" + os.urandom(2044))
        mock_remove(api_base, victim["id"])
        plan = review_ui(session)
        rows = plan["remoteDeleted"]
        assert [row["filename"] for row in rows] == [victim["filename"]], summarize(plan)
        assert rows[0]["companions"] == [stem + ".CR3"], rows
        # The RAW is a companion: the pair is not proposed for upload.
        set_row(session, "remoteDeleted", rows[0]["id"], "trash_raw")
        set_group(session, "ratings", "later") if has_group(session, "ratings") else None
        report = apply_ui(session)
        assert not os.path.exists(victim_path), "the photo should be in the trash"
        assert not os.path.exists(raw_path), "its RAW should be in the trash too"
        assert report["remoteDeletedResolved"] == 1, report_summary(report)

        # 6. Picstome stars, photo by photo: one row left for later.
        plan = review_ui(session)
        ratings = plan["ratings"]["items"]
        log("ratings:", [(item["filename"], item["rating"]) for item in ratings])
        assert ratings, "the demo gallery has rated photos"
        later_id = ratings[0]["id"]
        set_row(session, "ratings", later_id, "later")
        report = apply_ui(session)
        rated = (report.get("ratingsResult") or {}).get("rated", 0)
        assert rated == len(ratings) - 1, (rated, len(ratings))
        plan = review_ui(session)
        left = [item["id"] for item in plan["ratings"]["items"]]
        assert left == [later_id], left
        discard_ui(session)

        # 7. A file edited after the review is left alone.
        edited = os.path.join(photos, "EDITED_AFTER_REVIEW.jpg")
        write_jpeg(edited, 3000, 3)
        plan = review_ui(session)
        assert [item["filename"] for item in plan["uploads"]] == ["EDITED_AFTER_REVIEW.jpg"], summarize(plan)
        time.sleep(0.05)
        write_jpeg(edited, 5000, 9)
        set_group(session, "ratings", "later") if has_group(session, "ratings") else None
        report = apply_ui(session)
        assert report["skippedChanged"] == ["EDITED_AFTER_REVIEW.jpg"], report_summary(report)
        assert not report["uploaded"], report_summary(report)
        os.remove(edited)

        # 8. Mass delete guard: Apply stays disabled until the count is typed.
        doomed = sorted(photo_files(photos))[:12]
        for name in doomed:
            os.remove(os.path.join(photos, name))
        plan = review_ui(session)
        assert plan["massDelete"], summarize(plan)
        set_group(session, "localDeleted", "delete")
        guard = ui(
            session,
            "done({input: Boolean(document.getElementById('massDeleteConfirm')),"
            " disabled: document.getElementById('btnReviewApply').disabled});",
        )
        assert guard == {"input": True, "disabled": True}, guard
        ui(
            session,
            """
            const input = document.getElementById('massDeleteConfirm');
            input.value = String(%d);
            input.dispatchEvent(new Event('input', { bubbles: true }));
            done(document.getElementById('btnReviewApply').disabled);
            """ % len(plan["localDeleted"]),
        )
        enabled = not ui(session, "done(document.getElementById('btnReviewApply').disabled);")
        assert enabled, "typing the count must enable Apply"
        # Do not delete: bring the photos back instead.
        set_group(session, "localDeleted", "restore")
        set_group(session, "ratings", "later") if has_group(session, "ratings") else None
        report = apply_ui(session)
        assert len(report["downloaded"]) == len(doomed), report_summary(report)
        assert all(os.path.exists(os.path.join(photos, name)) for name in doomed)
        assert len(mock_manifest(api_base)["photos"]) == EXPECTED - 1

        # 9. A picstome://sync link opens the review and applies nothing.
        mock_add(api_base, "FROM_THE_WEB.jpg")
        files_before = photo_files(photos)
        js_invoke(session, "dev_handle_deep_link", {"url": f"picstome://sync?gallery={gallery}&name=E2E"})
        plan = wait_for(
            session,
            "done(!state.syncBusy && state.review ? state.review.plan : null);",
            timeout=60,
        )
        assert any(item["filename"] == "FROM_THE_WEB.jpg" for item in plan["downloads"]), summarize(plan)
        assert photo_files(photos) == files_before, "a deep link must not apply anything"
        discard_ui(session)

        # 10. In sync: a review with nothing to do says so.
        plan = review_ui(session)
        if plan:
            set_group(session, "ratings", "later") if has_group(session, "ratings") else None
            apply_ui(session)
            plan = review_ui(session)
        assert plan is None or not (plan["downloads"] or plan["uploads"]), summarize(plan)

        # Unlinking asks first and never touches local files.
        click(session, "#btnUnlink")
        wait_for(session, "done(!document.getElementById('confirmModal').classList.contains('hidden'));")
        click(session, "#btnConfirmCancel")
        assert any(item["galleryId"] == gallery for item in js_invoke(session, "get_state", {})["galleries"])

        logout = js_invoke(session, "logout", {})
        assert logout.get("ok"), logout
        state = js_invoke(session, "get_state", {})
        assert not state["auth"]["connected"], "logout did not disconnect"

        state = js_invoke(session, "set_test_mode", {"enabled": True})
        assert state["config"]["apiKind"] == "development", state["config"]
        state = js_invoke(session, "set_test_mode", {"enabled": False})
        assert state["config"]["apiKind"] == "production", state["config"]

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
