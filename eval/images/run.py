#!/usr/bin/env python3
"""The reverse-image golden set (M10-T05.1) and its budget (T05.2).

Pictures the index holds must find themselves. The set is sampled once from the vector store —
thirty images with their pages — and written to `set.jsonl` (URLs and ids only; no image is kept
here), so later runs measure the same thing. For each picture: the original must find its own
page at rank 1 and itself in `same`; a 20 % centre crop and a q50 re-encode must find it in the
top 3; a 90° rotation is allowed to miss and is only reported.

    services/clip-embed/.venv/bin/python eval/images/run.py [--resample] [--n 30]

Needs the API on :8080 with [vector] on, Qdrant on :6333, and Pillow (the clip venv has it).
"""
from __future__ import annotations

import argparse
import io
import json
import os
import statistics
import sys
import time
import urllib.request

ROTATE = "--rotate" in sys.argv
HERE = os.path.dirname(os.path.abspath(__file__))
SET = os.path.join(HERE, "set.jsonl")
API = os.environ.get("API", "http://127.0.0.1:8080")
QDRANT = os.environ.get("QDRANT", "http://127.0.0.1:6333")
UA = "XustiveEval/0.1 (+https://xustive.dz)"


def http(url: str, data: bytes | None = None, ctype: str = "application/json", timeout: int = 60):
    req = urllib.request.Request(url, data=data, headers={"Content-Type": ctype, "User-Agent": UA})
    return urllib.request.urlopen(req, timeout=timeout)


def sample(n: int) -> list[dict]:
    body = json.dumps({"limit": 400, "with_payload": True, "with_vector": False}).encode()
    pts = json.load(http(f"{QDRANT}/collections/image_clip/points/scroll", body))["result"]["points"]
    # Prefer pictures with a style: photographs and illustrations, not tracking pixels.
    pts = [p for p in pts if p["payload"].get("style")] or pts
    seen, out = set(), []
    for p in pts:
        d = p["payload"]["document_id"]
        if d in seen:
            continue
        seen.add(d)
        out.append({"url": p["payload"]["media_url"], "document_id": d})
        if len(out) >= n:
            break
    return out


def fetch(url: str) -> bytes | None:
    try:
        with http(url, timeout=20) as r:
            return r.read() if r.headers.get_content_type().startswith("image/") else None
    except Exception:  # noqa: BLE001
        return None


def variants(data: bytes) -> dict[str, bytes]:
    from PIL import Image

    im = Image.open(io.BytesIO(data)).convert("RGB")
    out = {"original": data}
    w, h = im.size
    box = (int(w * 0.1), int(h * 0.1), int(w * 0.9), int(h * 0.9))
    for name, img, q in [("crop20", im.crop(box), 92), ("q50", im, 50), ("rot90", im.rotate(90, expand=True), 92)]:
        if name == "rot90" and not ROTATE:
            continue
        buf = io.BytesIO()
        img.save(buf, format="JPEG", quality=q)
        out[name] = buf.getvalue()
    return out


def rank_of(images: list[dict], url: str, doc: str) -> tuple[int | None, str | None]:
    for i, im in enumerate(images):
        if im.get("group") == "web":
            continue
        if im.get("url") == url or im.get("page", {}).get("id") == doc:
            return i + 1, im.get("group")
    return None, None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--resample", action="store_true")
    ap.add_argument("--n", type=int, default=30)
    ap.add_argument("--rotate", action="store_true", help="also try a 90° rotation (reported, never gated)")
    # The endpoint allows ten searches a minute per network; the eval is a client like any other.
    ap.add_argument("--pace", type=float, default=6.5, help="seconds between searches")
    args = ap.parse_args()

    if args.resample or not os.path.exists(SET):
        items = sample(args.n)
        with open(SET, "w", encoding="utf-8") as fh:
            for it in items:
                fh.write(json.dumps(it) + "\n")
        print(f"sampled {len(items)} pictures into {SET}")
    items = [json.loads(l) for l in open(SET, encoding="utf-8") if l.strip()]

    hits = {"original": [0, 0], "crop20": [0, 0], "q50": [0, 0], "rot90": [0, 0]}  # [top1, top3]
    print(f"{len(items)} pictures, {args.pace}s between searches — this takes a while, by design")
    same_ok, n_ok, lat = 0, 0, []
    for it in items:
        data = fetch(it["url"])
        if not data:
            print(f"  skip (unfetchable): {it['url'][:70]}")
            continue
        try:
            vs = variants(data)
        except Exception as e:  # noqa: BLE001
            print(f"  skip (undecodable {type(e).__name__}): {it['url'][:70]}")
            continue
        n_ok += 1
        for name, blob in vs.items():
            t0 = time.monotonic()
            try:
                reply = json.load(http(f"{API}/api/v1/search/image", blob, "image/jpeg"))
            except urllib.error.HTTPError as e:
                print(f"  error HTTP {e.code} on {name}: {it['url'][:60]}")
                time.sleep(args.pace)
                continue
            except Exception as e:  # noqa: BLE001
                print(f"  error {type(e).__name__} on {name}: {it['url'][:60]}")
                continue
            lat.append(time.monotonic() - t0)
            time.sleep(args.pace)
            r, g = rank_of(reply.get("images", []), it["url"], it["document_id"])
            if r == 1:
                hits[name][0] += 1
            if r is not None and r <= 3:
                hits[name][1] += 1
            if name == "original" and g == "same":
                same_ok += 1
    if n_ok == 0:
        print("no usable pictures")
        return 2
    print(f"\n{n_ok} pictures × 4 variants")
    for name, (t1, t3) in hits.items():
        if name == "rot90" and not ROTATE:
            continue
        print(f"  {name:9} top-1 {t1}/{n_ok}  top-3 {t3}/{n_ok}")
    print(f"  original in `same`: {same_ok}/{n_ok}")
    lat.sort()
    p95 = lat[int(len(lat) * 0.95) - 1] if lat else 0
    print(f"  latency: median {statistics.median(lat)*1000:.0f} ms, p95 {p95*1000:.0f} ms over {len(lat)} searches")
    gate = hits["original"][0] == n_ok and same_ok == n_ok and hits["crop20"][1] == n_ok and hits["q50"][1] == n_ok
    print("\nGATE:", "pass" if gate else "FAIL", "(original top-1 & same, crop20 and q50 top-3, for every picture)")
    return 0 if gate else 1


if __name__ == "__main__":
    sys.exit(main())
