# Reverse-image golden set

`run.py` samples thirty pictures the index holds (URLs and page ids only — `set.jsonl`; no image
is stored here) and asks the reverse image search for each one four ways: the original, a 20 %
centre crop, a q50 re-encode, and a 90° rotation. The gate (M10-T05.1): the original finds its
page at rank 1 and itself in `same`; the crop and the re-encode find it in the top 3; the rotation
is reported only. Latency is the local leg's, median and p95 (M10-T05.2).

```
services/clip-embed/.venv/bin/python eval/images/run.py            # reuse set.jsonl
services/clip-embed/.venv/bin/python eval/images/run.py --resample # a fresh thirty
```
