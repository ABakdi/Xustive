"""Contract tests against a running sidecar (M10-T01.5).

Run: `.venv/bin/python -m pytest test_contract.py -q` with the sidecar on :8092. Skips, not fails,
when it is down — this is a test of the wire contract, not of the machine.

Fixtures: a PNG generated here (contract shape), and, when present, real images in
`CLIP_FIXTURES` (default: none) named `screenshot.*`, `photo*.*`, `mosque.*`, `football.*` for the
label assertions the milestone's exit gate names.
"""
from __future__ import annotations

import glob
import io
import json
import math
import os
import urllib.request

import pytest

BASE = os.environ.get("CLIP_URL", "http://127.0.0.1:8092")
FIX = os.environ.get("CLIP_FIXTURES", "")


def _up() -> bool:
    try:
        return urllib.request.urlopen(f"{BASE}/health", timeout=3).status == 200
    except Exception:  # noqa: BLE001
        return False


pytestmark = pytest.mark.skipif(not _up(), reason="clip sidecar not running")


def post(path: str, data: bytes, ctype: str) -> dict:
    r = urllib.request.Request(f"{BASE}{path}", data=data, headers={"Content-Type": ctype})
    return json.load(urllib.request.urlopen(r, timeout=60))


def png() -> bytes:
    from PIL import Image

    im = Image.new("RGB", (64, 64), (200, 30, 30))
    buf = io.BytesIO()
    im.save(buf, format="PNG")
    return buf.getvalue()


def softmax_top(styles: dict) -> tuple[str, float]:
    z = {k: 100 * v for k, v in styles.items()}
    m = max(z.values())
    e = {k: math.exp(v - m) for k, v in z.items()}
    s = sum(e.values())
    k = max(e, key=e.get)
    return k, e[k] / s


def test_embed_shape():
    out = post("/embed", png(), "image/png")
    assert len(out["embedding"]) == 512
    assert abs(sum(x * x for x in out["embedding"]) - 1.0) < 1e-3  # unit vector
    assert "styles" not in out


def test_describe_shape():
    out = post("/embed?describe=1", png(), "image/png")
    assert set(out) == {"embedding", "styles", "subjects"}
    assert len(out["styles"]) >= 10 and all(isinstance(v, float) for v in out["styles"].values())
    assert 1 <= len(out["subjects"]) <= 5 and {"id", "score"} <= set(out["subjects"][0])


def test_text_and_classify_agree_with_describe():
    text = post("/embed/text", json.dumps({"texts": ["a photograph", "a screenshot of a screen"]}).encode(), "application/json")
    assert len(text["vectors"]) == 2 and len(text["vectors"][0]) == 512
    img = post("/embed?describe=1", png(), "image/png")
    cls = post("/classify", json.dumps({"vectors": [img["embedding"]]}).encode(), "application/json")
    # Classifying the stored vector must give the same words as describing the image.
    assert cls["items"][0]["styles"] == pytest.approx(img["styles"], abs=1e-3)


def test_bad_inputs():
    with pytest.raises(urllib.error.HTTPError) as e:
        post("/embed", b"", "image/png")
    assert e.value.code == 422
    with pytest.raises(urllib.error.HTTPError) as e:
        post("/classify", json.dumps({"vectors": [[0.1, 0.2]]}).encode(), "application/json")
    assert e.value.code == 422


@pytest.mark.skipif(not FIX, reason="no CLIP_FIXTURES dir")
@pytest.mark.parametrize("pattern,style", [("screenshot.*", "screenshot"), ("mosque.*", "photo"), ("football.*", "photo")])
def test_fixture_styles(pattern, style):
    files = glob.glob(os.path.join(FIX, pattern))
    if not files:
        pytest.skip(f"no {pattern} fixture")
    out = post("/embed?describe=1", open(files[0], "rb").read(), "image/jpeg")
    top, p = softmax_top(out["styles"])
    assert top == style and p >= 0.5


@pytest.mark.skipif(not FIX, reason="no CLIP_FIXTURES dir")
def test_text_tower_separates_subjects():
    m, f = glob.glob(os.path.join(FIX, "mosque.*")), glob.glob(os.path.join(FIX, "football.*"))
    if not (m and f):
        pytest.skip("need mosque and football fixtures")
    tv = post("/embed/text", json.dumps({"texts": ["a photo of a mosque"]}).encode(), "application/json")["vectors"][0]
    mv = post("/embed", open(m[0], "rb").read(), "image/jpeg")["embedding"]
    fv = post("/embed", open(f[0], "rb").read(), "image/jpeg")["embedding"]
    dot = lambda a, b: sum(x * y for x, y in zip(a, b))  # noqa: E731
    assert dot(tv, mv) > dot(tv, fv)
