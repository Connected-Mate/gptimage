#!/usr/bin/env python3
"""Turn a raw ChatGPT-login response (saved with HOI4_IMAGEGEN_RECORD_RESPONSES) into a recorded-real replay fixture.

Every event and field the backend sent is kept, except:
* identifiers that could link the fixture to an account (safety_identifier, prompt_cache_key, user, response and item
  ids) are replaced with fixed placeholders;
* the image result is replaced by the same image downscaled to fit 64 px, so the fixture stays small. The image is
  still the real output, only smaller.

Usage: make_replay_fixture.py <raw.sse> <out.sse>
"""

from __future__ import annotations

import base64
import io
import json
import re
import sys

from PIL import Image

REDACT_KEYS = {"safety_identifier": "user-REDACTED", "prompt_cache_key": "REDACTED", "user": None}


def scrub(value):
    if isinstance(value, dict):
        out = {}
        for k, v in value.items():
            if k in REDACT_KEYS and v is not None:
                out[k] = REDACT_KEYS[k]
            elif k == "result" and isinstance(v, str) and len(v) > 1000:
                out[k] = shrink(v)
            else:
                out[k] = scrub(v)
        return out
    if isinstance(value, list):
        return [scrub(v) for v in value]
    if isinstance(value, str):
        value = re.sub(r"\bresp_[0-9a-f]{20,}\b", "resp_recorded", value)
        value = re.sub(r"\big_[0-9a-f]{20,}\b", "ig_recorded", value)
        value = re.sub(r"\bmsg_[0-9a-f]{20,}\b", "msg_recorded", value)
        value = re.sub(r"\brs_[0-9a-f]{20,}\b", "rs_recorded", value)
    return value


def shrink(b64: str) -> str:
    image = Image.open(io.BytesIO(base64.b64decode(b64))).convert("RGBA")
    image.thumbnail((64, 64), Image.Resampling.LANCZOS)
    buf = io.BytesIO()
    image.save(buf, format="PNG", optimize=True)
    return base64.b64encode(buf.getvalue()).decode("ascii")


def main() -> None:
    raw, out = sys.argv[1], sys.argv[2]
    text = open(raw, encoding="utf-8").read().replace("\r\n", "\n")
    events = []
    for block in text.split("\n\n"):
        if not block.strip():
            continue
        lines = []
        for line in block.split("\n"):
            if line.startswith("data:"):
                payload = line[5:].strip()
                if payload and payload != "[DONE]":
                    try:
                        payload = json.dumps(scrub(json.loads(payload)), separators=(",", ":"))
                    except json.JSONDecodeError:
                        pass
                lines.append("data: " + payload)
            else:
                lines.append(line)
        events.append("\n".join(lines))
    body = "\n\n".join(events) + "\n\n"
    if re.search(r"eyJ[A-Za-z0-9_-]{100,}\.", body):
        raise SystemExit("refusing to write: something that looks like a JWT is still present")
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write(body)
    print(f"wrote {out} ({len(body)} bytes)")


if __name__ == "__main__":
    main()
