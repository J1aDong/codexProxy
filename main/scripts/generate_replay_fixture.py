#!/usr/bin/env python3
"""
Generate a replay fixture from codexProxy runtime logs.

The fixture is intended for fast reproduction of stream parsing issues:
- Extracts upstream-like SSE frames (`event:` + `data:` pairs)
- Normalizes timestamp prefixes from logger output
- Emits a compact JSON fixture that can be consumed by tests/tools
"""

from __future__ import annotations

import argparse
import json
import re
from dataclasses import dataclass, asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import List, Optional


TIMESTAMP_PREFIX_RE = re.compile(r"^\[[^\]]+\]\s*")


@dataclass
class ReplayFrame:
    event: str
    data: str
    raw: str


def strip_log_prefix(line: str) -> str:
    return TIMESTAMP_PREFIX_RE.sub("", line.rstrip("\n"))


def parse_frames(lines: List[str]) -> List[ReplayFrame]:
    frames: List[ReplayFrame] = []
    current_event: Optional[str] = None
    current_data_lines: List[str] = []

    def flush_current() -> None:
        nonlocal current_event, current_data_lines
        if not current_event:
            current_data_lines = []
            return
        data = "\n".join(current_data_lines)
        raw = f"event: {current_event}\ndata: {data}\n\n"
        frames.append(ReplayFrame(event=current_event, data=data, raw=raw))
        current_event = None
        current_data_lines = []

    for raw_line in lines:
        line = strip_log_prefix(raw_line).strip()
        if not line:
            continue

        if line.startswith(":"):
            flush_current()
            frames.append(
                ReplayFrame(event="keepalive", data=line, raw=f"{line}\n\n")
            )
            continue

        if line.startswith("event: "):
            flush_current()
            current_event = line[len("event: ") :].strip()
            continue

        if line.startswith("data: "):
            payload = line[len("data: ") :]
            if current_event is None:
                # Some logs only preserve data lines. Keep them as anonymous data events
                current_event = "data_only"
            current_data_lines.append(payload)
            continue

    flush_current()
    return frames


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate replay fixture JSON from codexProxy log files"
    )
    parser.add_argument("log", type=Path, help="Path to proxy_*.log")
    parser.add_argument(
        "-o", "--out", type=Path, required=True, help="Output fixture path (.json)"
    )
    parser.add_argument(
        "--max-frames",
        type=int,
        default=400,
        help="Cap number of frames written to fixture (default: 400)",
    )
    args = parser.parse_args()

    log_path: Path = args.log.expanduser().resolve()
    out_path: Path = args.out.expanduser().resolve()

    if not log_path.exists():
        raise SystemExit(f"log file not found: {log_path}")

    source_lines = log_path.read_text(encoding="utf-8", errors="replace").splitlines()
    frames = parse_frames(source_lines)
    if args.max_frames > 0:
        frames = frames[: args.max_frames]

    fixture = {
        "type": "codex_proxy_replay_fixture",
        "source_log": str(log_path),
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "frame_count": len(frames),
        "frames": [asdict(frame) for frame in frames],
    }

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(
        json.dumps(fixture, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    print(f"wrote fixture: {out_path} (frames={len(frames)})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
