#!/usr/bin/env python3
import json
import os
import sys
import time
import socket
import http.client
import urllib.parse
from pathlib import Path

ROOT_DIR = Path(__file__).resolve().parents[1]
CONFIG_PATH = ROOT_DIR / "docs" / "key.json"

BLACK_PIXEL_PNG_BASE64 = (
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
)
DATA_URL = f"data:image/png;base64,{BLACK_PIXEL_PNG_BASE64}"


def load_config():
    if not CONFIG_PATH.exists():
        raise RuntimeError(f"Missing config: {CONFIG_PATH}")
    with open(CONFIG_PATH, "r", encoding="utf-8") as f:
        data = json.load(f)
    api_key = data.get("api_key")
    base_url = data.get("base_url", "http://localhost:8889")
    endpoint = data.get("messages_endpoint", "/v1/messages")
    log_dirs = data.get("log_dirs", [])
    if not api_key:
        raise RuntimeError("api_key is required in docs/key.json")
    return api_key, base_url, endpoint, log_dirs


def build_url(base_url, endpoint):
    base = base_url.rstrip("/") + "/"
    path = endpoint.lstrip("/")
    return urllib.parse.urljoin(base, path)


def http_request(method, url, headers, body=None, timeout=30):
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme == "https":
        conn = http.client.HTTPSConnection(parsed.hostname, parsed.port or 443, timeout=timeout)
    else:
        conn = http.client.HTTPConnection(parsed.hostname, parsed.port or 80, timeout=timeout)
    path = parsed.path or "/"
    if parsed.query:
        path += "?" + parsed.query
    data = None
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers = dict(headers)
        headers["Content-Type"] = "application/json"
    conn.request(method, path, body=data, headers=headers)
    resp = conn.getresponse()
    text = resp.read().decode("utf-8", errors="ignore")
    conn.close()
    return resp.status, text


def is_timeout_error(exc):
    if isinstance(exc, (socket.timeout, TimeoutError)):
        return True
    return "timed out" in str(exc).lower()


def parse_sse_events(text):
    events = []
    for line in text.splitlines():
        if line.startswith("data: "):
            payload = line[6:].strip()
            if payload == "[DONE]":
                continue
            try:
                events.append(json.loads(payload))
            except json.JSONDecodeError:
                continue
    return events


def create_temp_image_file():
    tmp_dir = Path("/tmp/codex_proxy_test")
    tmp_dir.mkdir(parents=True, exist_ok=True)
    image_path = tmp_dir / "test_image.png"
    if not image_path.exists():
        image_path.write_bytes(_decode_base64_png())
    return image_path


def _decode_base64_png():
    import base64

    return base64.b64decode(BLACK_PIXEL_PNG_BASE64)


def find_request_in_logs(log_dirs, marker):
    for log_dir in log_dirs:
        path = (ROOT_DIR / log_dir) if not os.path.isabs(log_dir) else Path(log_dir)
        if not path.exists():
            continue
        log_files = sorted(path.glob("proxy_*.log"), key=lambda p: p.stat().st_mtime, reverse=True)
        for log_file in log_files:
            text = log_file.read_text(encoding="utf-8", errors="ignore")
            for request_body in extract_request_bodies(text):
                request_dump = json.dumps(request_body, ensure_ascii=False)
                if marker in request_dump:
                    return request_body, log_file
    return None, None


def extract_request_bodies(log_text):
    lines = log_text.splitlines()
    idx = 0
    while idx < len(lines):
        if "Request Body:" in lines[idx]:
            idx += 1
            while idx < len(lines) and lines[idx].strip() == "":
                idx += 1
            json_lines = []
            while idx < len(lines):
                line = lines[idx]
                if line.strip().startswith("════════"):
                    idx += 1
                    break
                json_lines.append(line)
                idx += 1
            if json_lines:
                try:
                    yield json.loads("\n".join(json_lines))
                except json.JSONDecodeError:
                    pass
        else:
            idx += 1


def extract_message_with_marker(request_body, marker):
    for item in request_body.get("input", []):
        if item.get("type") != "message":
            continue
        for block in item.get("content", []):
            if isinstance(block, dict) and marker in json.dumps(block, ensure_ascii=False):
                return item
    return None


def count_input_images(message_item):
    count = 0
    for block in message_item.get("content", []):
        if isinstance(block, dict) and block.get("type") == "input_image":
            count += 1
    return count


def get_input_image_urls(message_item):
    urls = []
    for block in message_item.get("content", []):
        if isinstance(block, dict) and block.get("type") == "input_image":
            url = block.get("image_url", "")
            if isinstance(url, str):
                urls.append(url)
    return urls


def make_base_headers(api_key):
    return {
        "x-api-key": api_key,
        "x-anthropic-version": "2023-06-01",
        "User-Agent": "codex-proxy-test",
    }


def test_core_001(base_url, headers):
    status, _ = http_request("GET", base_url, headers, body=None, timeout=10)
    return status > 0, f"status={status}"


def test_core_002(url, headers):
    body = {
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 128,
        "messages": [{"role": "user", "content": "Say hello"}],
        "stream": True,
    }
    status, text = http_request("POST", url, headers, body=body, timeout=30)
    if status != 200:
        return False, f"status={status}"
    events = parse_sse_events(text)
    has_message_start = any(e.get("type") == "message_start" for e in events)
    has_content_block = any(e.get("type") == "content_block_start" for e in events)
    has_text_delta = any(e.get("type") == "content_block_delta" for e in events)
    has_message_stop = any(e.get("type") == "message_stop" for e in events)
    ok = all([has_message_start, has_content_block, has_text_delta, has_message_stop])
    return ok, f"events={len(events)}"


def test_core_003(url, headers):
    body = {
        "model": "claude-sonnet-4-5-20250929",
        "messages": [{"role": "user", "content": "Use shell_command to run echo ok"}],
        "tools": [
            {
                "name": "shell_command",
                "description": "Execute shell commands",
                "input_schema": {
                    "type": "object",
                    "properties": {"command": {"type": "string"}},
                    "required": ["command"],
                },
            }
        ],
        "stream": True,
        "max_tokens": 128,
    }
    status, text = http_request("POST", url, headers, body=body, timeout=60)
    if status != 200:
        return False, f"status={status}"
    events = parse_sse_events(text)
    has_tool_use = any(
        e.get("type") == "content_block_start"
        and e.get("content_block", {}).get("type") == "tool_use"
        for e in events
    )
    has_tool_stop = any(
        e.get("type") == "message_stop" and e.get("stop_reason") == "tool_use"
        for e in events
    )
    ok = has_tool_use or has_tool_stop
    return ok, f"tool_use={has_tool_use}, tool_stop={has_tool_stop}"


def test_img_001_007(url, headers, log_dirs):
    marker = f"TDD-IMG-001-{int(time.time())}"
    body = {
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 128,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": f"Describe this image: {marker}"},
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": BLACK_PIXEL_PNG_BASE64,
                        },
                    },
                ],
            }
        ],
        "stream": True,
    }
    status, text = http_request("POST", url, headers, body=body, timeout=60)
    if status != 200:
        return False, f"status={status}"
    if "本地路径" in text or "local path" in text.lower():
        return False, "response requests local path"
    request_body, log_file = find_request_in_logs(log_dirs, marker)
    if request_body is None:
        return False, "no log entry found for marker"
    message_item = extract_message_with_marker(request_body, marker)
    if message_item is None:
        return False, f"marker not found in request (log={log_file})"
    urls = get_input_image_urls(message_item)
    if not urls:
        return False, "no input_image in request"
    ok = any(u.startswith("data:image/") for u in urls)
    return ok, f"log={log_file}"


def test_img_002(url, headers, log_dirs):
    marker = f"TDD-IMG-002-{int(time.time())}"
    body = {
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 128,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": f"Describe this image: {marker}"},
                    {
                        "type": "image",
                        "source": {"type": "base64", "data": DATA_URL},
                    },
                ],
            }
        ],
        "stream": True,
    }
    status, text = http_request("POST", url, headers, body=body, timeout=60)
    if status != 200:
        return False, f"status={status}"
    if "本地路径" in text or "local path" in text.lower():
        return False, "response requests local path"
    request_body, _ = find_request_in_logs(log_dirs, marker)
    if request_body is None:
        return False, "no log entry found for marker"
    message_item = extract_message_with_marker(request_body, marker)
    if message_item is None:
        return False, "marker not found in request"
    urls = get_input_image_urls(message_item)
    if not urls:
        return False, "no input_image in request"
    ok = any(u == DATA_URL for u in urls)
    return ok, "data_url passthrough"


def test_img_003(url, headers, log_dirs):
    marker = f"TDD-IMG-003-{int(time.time())}"
    body = {
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 128,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": f"Describe this image: {marker}"},
                    {
                        "type": "image_url",
                        "image_url": {"url": DATA_URL},
                    },
                ],
            }
        ],
        "stream": True,
    }
    status, text = http_request("POST", url, headers, body=body, timeout=60)
    if status != 200:
        return False, f"status={status}"
    if "本地路径" in text or "local path" in text.lower():
        return False, "response requests local path"
    request_body, _ = find_request_in_logs(log_dirs, marker)
    if request_body is None:
        return False, "no log entry found for marker"
    message_item = extract_message_with_marker(request_body, marker)
    if message_item is None:
        return False, "marker not found in request"
    urls = get_input_image_urls(message_item)
    if not urls:
        return False, "no input_image in request"
    ok = any(u == DATA_URL for u in urls)
    return ok, "image_url object resolved"


def test_img_004(url, headers, log_dirs):
    marker = f"TDD-IMG-004-{int(time.time())}"
    body = {
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 128,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": f"before {marker}"},
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": BLACK_PIXEL_PNG_BASE64,
                        },
                    },
                    {"type": "text", "text": f"after {marker}"},
                ],
            }
        ],
        "stream": True,
    }
    status, text = http_request("POST", url, headers, body=body, timeout=60)
    if status != 200:
        return False, f"status={status}"
    if "本地路径" in text or "local path" in text.lower():
        return False, "response requests local path"
    request_body, _ = find_request_in_logs(log_dirs, marker)
    if request_body is None:
        return False, "no log entry found for marker"
    message_item = extract_message_with_marker(request_body, marker)
    if message_item is None:
        return False, "marker not found in request"
    types = [block.get("type") for block in message_item.get("content", []) if isinstance(block, dict)]
    ok = "input_image" in types
    if not ok:
        return False, "input_image missing"
    idx_img = types.index("input_image")
    ok = idx_img > 0 and idx_img < len(types) - 1
    return ok, f"order={types}"


def test_img_005(url, headers, log_dirs):
    marker = f"TDD-IMG-005-{int(time.time())}"
    body = {
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 128,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": f"Compare these images: {marker}"},
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": BLACK_PIXEL_PNG_BASE64,
                        },
                    },
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": BLACK_PIXEL_PNG_BASE64,
                        },
                    },
                ],
            }
        ],
        "stream": True,
    }
    status, text = http_request("POST", url, headers, body=body, timeout=60)
    if status != 200:
        return False, f"status={status}"
    if "本地路径" in text or "local path" in text.lower():
        return False, "response requests local path"
    request_body, _ = find_request_in_logs(log_dirs, marker)
    if request_body is None:
        return False, "no log entry found for marker"
    message_item = extract_message_with_marker(request_body, marker)
    if message_item is None:
        return False, "marker not found in request"
    count = count_input_images(message_item)
    return count >= 2, f"input_image_count={count}"


def test_img_006(url, headers, log_dirs):
    marker = f"TDD-IMG-006-{int(time.time())}"
    image_path = create_temp_image_file()
    body = {
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 128,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": f"Describe this image: {marker}"},
                    {
                        "type": "image",
                        "source": {
                            "type": "url",
                            "url": f"file://{image_path}",
                        },
                    },
                ],
            }
        ],
        "stream": True,
    }
    status = None
    text = ""
    timed_out = False
    try:
        status, text = http_request("POST", url, headers, body=body, timeout=60)
    except Exception as e:
        timed_out = is_timeout_error(e)
        if not timed_out:
            return False, f"exception={e}"

    if status is not None and status != 200:
        lower = text.lower()
        ok = ("file://" in lower) or ("unsupported" in lower) or ("not supported" in lower)
        return ok, f"status={status}"
    if text and ("本地路径" in text or "local path" in text.lower()):
        return False, "response requests local path"
    request_body, _ = find_request_in_logs(log_dirs, marker)
    if request_body is None:
        return False, "no log entry found for marker"
    message_item = extract_message_with_marker(request_body, marker)
    if message_item is None:
        return False, "marker not found in request"
    urls = get_input_image_urls(message_item)
    if not urls:
        return False, "no input_image in request"
    ok = any(u.startswith("data:image/") for u in urls)
    if timed_out:
        return ok, "timeout but file:// converted to data URL"
    return ok, "file:// converted to data URL"


def test_img_008(url, headers, log_dirs):
    marker = f"TDD-IMG-008-{int(time.time())}"
    image_path = create_temp_image_file()
    body = {
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 128,
        "messages": [
            {
                "role": "user",
                "content": [
                    {"type": "text", "text": f"Describe this image: {marker}"},
                    {
                        "type": "image",
                        "source": {
                            "type": "file",
                            "path": str(image_path),
                        },
                    },
                ],
            }
        ],
        "stream": True,
    }
    status = None
    text = ""
    timed_out = False
    try:
        status, text = http_request("POST", url, headers, body=body, timeout=60)
    except Exception as e:
        timed_out = is_timeout_error(e)
        if not timed_out:
            return False, f"exception={e}"

    if status is not None and status != 200:
        lower = text.lower()
        ok = ("file://" in lower) or ("unsupported" in lower) or ("not supported" in lower)
        return ok, f"status={status}"
    if text and ("本地路径" in text or "local path" in text.lower()):
        return False, "response requests local path"
    request_body, _ = find_request_in_logs(log_dirs, marker)
    if request_body is None:
        return False, "no log entry found for marker"
    message_item = extract_message_with_marker(request_body, marker)
    if message_item is None:
        return False, "marker not found in request"
    urls = get_input_image_urls(message_item)
    if not urls:
        return False, "no input_image in request"
    ok = any(u.startswith("data:image/") for u in urls)
    if timed_out:
        return ok, "timeout but path converted to data URL"
    return ok, "path converted to data URL"


def run():
    api_key, base_url, endpoint, log_dirs = load_config()
    url = build_url(base_url, endpoint)
    headers = make_base_headers(api_key)

    tests = [
        ("CORE-001 Server Health", lambda: test_core_001(base_url, headers)),
        ("CORE-002 Text Streaming SSE", lambda: test_core_002(url, headers)),
        ("CORE-003 Tool Call Mapping", lambda: test_core_003(url, headers)),
        ("IMG-001/007 Base64 Image + No Local Path", lambda: test_img_001_007(url, headers, log_dirs)),
        ("IMG-002 Data URL Passthrough", lambda: test_img_002(url, headers, log_dirs)),
        ("IMG-003 image_url Object Accepted", lambda: test_img_003(url, headers, log_dirs)),
        ("IMG-004 Mixed Content Order", lambda: test_img_004(url, headers, log_dirs)),
        ("IMG-005 Multiple Images", lambda: test_img_005(url, headers, log_dirs)),
        ("IMG-006 file:// Path Handling", lambda: test_img_006(url, headers, log_dirs)),
        ("IMG-008 path Field Handling", lambda: test_img_008(url, headers, log_dirs)),
    ]

    failures = 0
    print("Codex Proxy Tests")
    print("=================")
    for name, fn in tests:
        try:
            ok, msg = fn()
        except Exception as e:
            ok, msg = False, f"exception={e}"
        status = "PASS" if ok else "FAIL"
        print(f"[{status}] {name} - {msg}")
        if not ok:
            failures += 1

    print("=================")
    print(f"Total: {len(tests)}, Failed: {failures}")
    return failures == 0


if __name__ == "__main__":
    success = run()
    sys.exit(0 if success else 1)
