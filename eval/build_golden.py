#!/usr/bin/env python3
"""Build a bootstrap golden set from the live index.

# What this produces, and what it does not

A machine-judged set of 200 queries. Every judgement in it is generated, not human, and the file
records that per query so a report cannot quote it as ground truth by accident.

**This detects regressions. It does not measure quality.** The judgements are derived by asking
the index which documents contain a query's terms, so a set built this way partly agrees with the
retrieval engine by construction. What it can still do is notice when a change makes the engine
disagree with its own past self — which is the thing that breaks silently and the reason
`M1-T15.4` exists.

Two kinds of query mitigate the circularity, and they are the ones worth watching:

- **Cross-script queries.** An Arabizi query judged against Arabic documents is not circular at
  all, because the judgement comes from the Arabic form and the retrieval has to bridge scripts to
  find it. These are the real test of transliteration and normalisation.
- **Orthographic variants.** `بجاية` versus `بجايه`, `أوت` versus `اوت`. The judgement is made on
  the canonical form; retrieval has to fold the variant onto it.

Replace judgements with human ones as they arrive: flip `judged_by` to `human` on the queries a
native speaker has reviewed. The report counts both, so the set can improve query by query.

    ./eval/build_golden.py --out eval/golden/v1.jsonl
"""

import argparse
import json
import re
import sys
import unicodedata
import urllib.parse
import urllib.request
from collections import Counter

# Language mix from Milestone 1 T15.5.
MIX = {"ar": 0.40, "ary": 0.25, "fr": 0.20, "en": 0.10, "mixed": 0.05}

# Arabic → Arabizi, the common substitutions Algerians actually type. Digits stand in for letters
# with no Latin equivalent: 7 for ح, 3 for ع, 9 for ق.
ARABIZI = {
    "ح": "7", "ع": "3", "ق": "9", "خ": "kh", "ش": "ch", "ث": "th", "ذ": "dh",
    "غ": "gh", "ص": "s", "ض": "d", "ط": "t", "ظ": "dh", "ا": "a", "أ": "a",
    "إ": "i", "آ": "a", "ب": "b", "ت": "t", "ج": "j", "د": "d", "ر": "r",
    "ز": "z", "س": "s", "ف": "f", "ك": "k", "ل": "l", "م": "m", "ن": "n",
    "ه": "h", "و": "ou", "ي": "i", "ى": "a", "ة": "a", "ء": "", "ئ": "i", "ؤ": "ou",
}

# Orthographic variants that normalisation must fold together. Each pair is a real difference
# between how Algerian publications and Algerian typists write the same word.
VARIANTS = [("ة", "ه"), ("أ", "ا"), ("إ", "ا"), ("آ", "ا"), ("ى", "ي"), ("ئ", "ي")]

STOP = {
    "ar": {"في", "من", "على", "إلى", "عن", "مع", "هذا", "هذه", "التي", "الذي", "بين",
           "أن", "إن", "كان", "قد", "ما", "لا", "و", "أو", "كل", "بعد", "قبل", "عند"},
    "fr": {"le", "la", "les", "de", "des", "du", "un", "une", "et", "en", "pour", "dans",
           "sur", "avec", "par", "au", "aux", "ce", "cette", "que", "qui", "est", "sont",
           "plus", "son", "ses", "sa", "il", "elle", "a", "à", "ne", "pas"},
    "en": {"the", "a", "an", "of", "and", "in", "on", "for", "to", "with", "at", "by",
           "is", "are", "was", "were", "this", "that", "from", "as", "it", "its"},
}
STOP["ary"] = STOP["ar"]
STOP["mixed"] = STOP["ar"] | STOP["fr"]


def fetch_documents(meili, index, key):
    """Pull the whole corpus. Small enough to hold in memory and simpler than paging blindly."""
    docs, offset = [], 0
    while True:
        url = (
            f"{meili}/indexes/{index}/documents?limit=1000&offset={offset}"
            "&fields=id,title,body,excerpt,language,domain,published_at"
        )
        req = urllib.request.Request(url)
        if key:
            req.add_header("Authorization", f"Bearer {key}")
        with urllib.request.urlopen(req, timeout=30) as r:
            page = json.load(r)["results"]
        if not page:
            break
        docs.extend(page)
        offset += len(page)
        if len(page) < 1000:
            break
    return docs


def fold(text):
    """Strip combining marks, for *comparison only*.

    Never used to build query text. NFD decomposition splits Arabic hamza carriers — ئ becomes
    ي plus a combining hamza — so stripping marks turns رئيس into رييس. That is a plausible
    misspelling but it is not what the document says, and emitting it as a "topical query from a
    document title" would mislabel an orthographic test as a topical one.
    """
    return "".join(c for c in unicodedata.normalize("NFD", text)
                   if unicodedata.category(c) != "Mn")


def tokens(text, lang):
    """Content words in their original spelling.

    Long words carry the topic; short ones carry grammar. Folding is applied when *matching*
    these against documents, not when producing them.
    """
    words = re.findall(r"[\w؀-ۿ]+", text or "")
    stop = STOP.get(lang, STOP["ar"])
    return [w for w in words
            if len(w) >= 4 and fold(w.lower()) not in {fold(x) for x in stop}
            and not w.isdigit()]


def to_arabizi(text):
    return "".join(ARABIZI.get(c, c) for c in text)


def apply_variant(text):
    """Rewrite one orthographic form into another that must fold onto it."""
    for src, dst in VARIANTS:
        if src in text:
            return text.replace(src, dst)
    return None


def judge(docs, terms, lang):
    """Grade documents by how much of the query they contain.

    Coverage, not a score: a document containing every query term is ideal, most of them is
    relevant, one is marginal. Crude, and deliberately so — a cleverer heuristic would agree with
    the ranker even more closely and measure even less.
    """
    graded = {}
    wanted = [fold(t.lower()) for t in terms]
    for d in docs:
        # Pre-folded once in main() (`_hay`); judging every query against every document used to
        # re-fold the full body per query — O(queries × corpus × body), minutes at corpus scale.
        haystack = d["_hay"]
        hits = sum(1 for t in wanted if t in haystack)
        if hits == 0:
            continue
        share = hits / len(wanted)
        if share >= 0.99:
            graded[d["id"]] = 3
        elif share >= 0.6:
            graded[d["id"]] = 2
        else:
            graded[d["id"]] = 1
    return graded


def build(docs, target):
    by_lang = {}
    for d in docs:
        by_lang.setdefault(d.get("language", "ar"), []).append(d)

    queries, seen = [], set()
    counts = Counter()

    def add(qid, text, lang, note, judgement_terms, judge_lang=None):
        text = " ".join(text.split())
        if not text or text.lower() in seen or len(judgement_terms) < 2:
            return False
        grades = judge(docs, judgement_terms, judge_lang or lang)
        # A query nothing answers teaches the harness nothing: it is excluded from nDCG anyway,
        # and it would inflate the zero-result rate with a question about a corpus we do not have.
        if not any(g >= 2 for g in grades.values()):
            return False
        seen.add(text.lower())
        counts[lang] += 1
        queries.append({
            "id": qid,
            "query": text,
            "lang": lang,
            "note": note,
            "judged_by": "machine",
            "judgements": grades,
        })
        return True

    quota = {lang: max(1, round(target * share)) for lang, share in MIX.items()}

    # --- pass 1: topical queries straight from document titles ------------------------------
    #
    # A title is a real information need someone already wrote down, so these read like queries
    # rather than like generated strings.
    for lang, want in quota.items():
        pool = by_lang.get(lang) or by_lang.get("ar", [])
        for d in pool:
            if counts[lang] >= want * 0.6:
                break
            terms = tokens(d.get("title", ""), lang)[:4]
            if len(terms) < 2:
                continue
            add(f"{lang}-topic-{counts[lang]:03d}", " ".join(terms[:3]), lang,
                "topical query from a document title", terms[:3])

    # --- pass 2: cross-script and orthographic variants --------------------------------------
    #
    # The queries that are not circular. Judgements come from the canonical Arabic form; the
    # engine has to bridge script or spelling to reach it.
    for lang in ("ar", "ary", "mixed"):
        want = quota[lang]
        pool = by_lang.get("ar", []) + by_lang.get("ary", [])
        for d in pool:
            if counts[lang] >= want:
                break
            terms = tokens(d.get("title", ""), "ar")[:3]
            if len(terms) < 2:
                continue

            if lang == "ary":
                text = to_arabizi(" ".join(terms[:2]))
                note = "arabizi query judged against arabic documents — tests transliteration"
            elif lang == "mixed":
                text = f"{terms[0]} {to_arabizi(terms[1])}"
                note = "script-mixed query — tests per-token handling"
            else:
                variant = apply_variant(" ".join(terms[:3]))
                if not variant:
                    continue
                text = variant
                note = "orthographic variant — tests normalisation folding"

            add(f"{lang}-cross-{counts[lang]:03d}", text, lang, note, terms[:2], "ar")

    # --- pass 3: single-entity queries to fill the remainder ---------------------------------
    for lang, want in quota.items():
        pool = by_lang.get(lang) or by_lang.get("ar", [])
        for d in pool:
            if counts[lang] >= want:
                break
            terms = tokens(f"{d.get('title', '')} {d.get('excerpt', '')}", lang)
            freq = Counter(terms)
            top = [w for w, _ in freq.most_common(6)]
            if len(top) < 2:
                continue
            add(f"{lang}-entity-{counts[lang]:03d}", " ".join(top[:2]), lang,
                "two-term entity query", top[:2])

    return queries, counts


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--meili", default="http://localhost:7700")
    ap.add_argument("--index", default="documents")
    ap.add_argument("--key", default="")
    ap.add_argument("--target", type=int, default=200)
    ap.add_argument("--out", default="eval/golden/v1.jsonl")
    args = ap.parse_args()

    docs = fetch_documents(args.meili, args.index, args.key)
    if not docs:
        print("no documents in the index; crawl or seed first", file=sys.stderr)
        return 1
    print(f"corpus: {len(docs)} documents", file=sys.stderr)

    # Fold each document's haystack ONCE — title, excerpt, and the topical head of the body. Judging
    # is term-containment, and topical terms sit near the top of a page, so the head is enough; a cap
    # also bounds memory on a large corpus. This turns judging from minutes into seconds.
    for d in docs:
        d["_hay"] = fold(
            f"{d.get('title', '')} {d.get('excerpt', '')} {d.get('body', '')[:2000]}"
        ).lower()

    queries, counts = build(docs, args.target)
    if not queries:
        print("no queries could be judged against this corpus", file=sys.stderr)
        return 1

    import os
    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as f:
        # A header line recording the corpus these judgements were made against. Judgements are
        # frozen; the index is not. Every document added afterwards is relevant to some query and
        # judged for none of them, so it is counted as irrelevant and drags nDCG down — a
        # regression that is really just a bigger corpus. The runner compares this number and
        # says so rather than letting the gate fire on growth.
        f.write(json.dumps({"_meta": {
            "corpus_size": len(docs),
            "queries": len(queries),
            "judged_by": "machine",
        }}, ensure_ascii=False) + "\n")
        for q in queries:
            f.write(json.dumps(q, ensure_ascii=False) + "\n")

    total = len(queries)
    print(f"wrote {total} queries to {args.out}", file=sys.stderr)
    for lang in MIX:
        got = counts[lang]
        print(f"  {lang:6} {got:4}  {got / total:5.1%}  (target {MIX[lang]:.0%})",
              file=sys.stderr)
    if total < args.target:
        print(
            f"\n  Short of {args.target}. Every query needs at least one document graded 2 or "
            f"better,\n  and a {len(docs)}-document corpus cannot answer more than this. Crawl "
            "more and re-run.",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
