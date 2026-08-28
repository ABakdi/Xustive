"""Contract test for the reranker sidecar. Needs the service up (RERANK_URL, default :8096)."""

import json
import os
import urllib.request

URL = os.environ.get("RERANK_URL", "http://127.0.0.1:8096")


def post(path, body):
    req = urllib.request.Request(URL + path, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r)


def test_health():
    with urllib.request.urlopen(URL + "/health", timeout=10) as r:
        assert r.status == 200


def test_the_relevant_document_scores_higher():
    out = post("/rerank", {"query": "couscous recipe", "documents": [
        "How to cook couscous: steam the semolina twice, then serve with the vegetable broth.",
        "The 2024 municipal election results by wilaya.",
    ]})
    scores = out["scores"]
    assert len(scores) == 2
    assert 0.0 <= min(scores) and max(scores) <= 1.0
    assert scores[0] > scores[1]


if __name__ == "__main__":
    test_health()
    test_the_relevant_document_scores_higher()
    print("ok")
