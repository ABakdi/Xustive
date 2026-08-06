#!/usr/bin/env python3
"""Generate the M0 sample corpus.

Deliberately *not* clean data. The corpus mixes MSA, Darija in Arabic script, Arabizi, French
and code-switched text, and includes tatweel, harakat, Arabic-Indic digits and unknown dates —
because a corpus that only contains tidy text hides exactly the bugs this project needs to find.

Output is JSON Lines matching the canonical Document schema.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
import time
import unicodedata
from pathlib import Path

# --- vocabulary ---------------------------------------------------------------------

WILAYAS = [
    ("الجزائر", "Alger", "16"), ("وهران", "Oran", "31"), ("قسنطينة", "Constantine", "25"),
    ("عنابة", "Annaba", "23"), ("سطيف", "Setif", "19"), ("باتنة", "Batna", "05"),
    ("بجاية", "Bejaia", "06"), ("تلمسان", "Tlemcen", "13"), ("ورقلة", "Ouargla", "30"),
    ("تيزي وزو", "Tizi Ouzou", "15"), ("البليدة", "Blida", "09"), ("مستغانم", "Mostaganem", "27"),
]

ORGS = [
    ("سونلغاز", "Sonelgaz"), ("سيال", "Seaal"), ("اتصالات الجزائر", "Algerie Telecom"),
    ("الضمان الاجتماعي", "CNAS"), ("الوكالة الوطنية للتشغيل", "ANEM"),
    ("الخطوط الجوية الجزائرية", "Air Algerie"), ("موبيليس", "Mobilis"), ("جيزي", "Djezzy"),
    ("نفطال", "Naftal"), ("بريد الجزائر", "Algerie Poste"),
]

TOPICS_AR = [
    ("فاتورة الكهرباء", "كيفية دفع فاتورة الكهرباء والغاز عبر الإنترنت"),
    ("عروض التشغيل", "إعلان توظيف جديد في عدة تخصصات"),
    ("سعر الصرف", "تطورات سعر الصرف في السوق الموازية"),
    ("النقل الحضري", "تدعيم شبكة النقل الحضري بحافلات جديدة"),
    ("التعليم العالي", "فتح باب التسجيلات الجامعية للموسم الجديد"),
    ("الصحة", "حملة تلقيح واسعة عبر المؤسسات الصحية"),
    ("السكن", "توزيع حصص جديدة من السكنات الاجتماعية"),
    ("الرياضة", "المنتخب الوطني يستعد للمباراة المقبلة"),
    ("الفلاحة", "موسم الحصاد يعرف نتائج مشجعة هذا العام"),
    ("الإنترنت", "تحسين تدفق الإنترنت عبر عدة ولايات"),
]

TOPICS_FR = [
    ("facture electricite", "Comment payer sa facture d'electricite en ligne"),
    ("offre emploi", "Recrutement dans plusieurs specialites"),
    ("transport urbain", "Renforcement du reseau de transport urbain"),
    ("inscription universitaire", "Ouverture des inscriptions universitaires"),
    ("logement social", "Distribution de nouveaux quotas de logements"),
    ("internet debit", "Amelioration du debit internet dans plusieurs wilayas"),
]

DARIJA_AR = [
    "واش راكم خاوتي، شكون يعرف كيفاش ندير هاد الإجراء؟",
    "بزاف الناس راهم يسقسيو على هاد الموضوع، نحاول نشرح بالتفصيل",
    "درك جاوبوني، شحال يدوم الملف باش يخرج؟",
    "خويا حاب نعرف وين نلقى المكتب نتاع هاد الخدمة",
    "راني قلت ليكم من قبل، الملف لازم يكون كامل قبل ما تروح",
    "كاين واحد الطريقة سهلة بزاف، غير اتبع الخطوات",
]

DARIJA_LATIN = [
    "wach rakom khawti, chkoun ya3ref kifach ndir had lijra2?",
    "bezaf nas rahom yesqsiw 3la had lmawdou3, n7awel nchre7 bettafsil",
    "drok jawbouni, ch7al ydoum lmalaf bach yekhroj?",
    "khoya 7ab n3ref win nelqa lmaktab nta3 had lkhedma",
    "rani goltlkom men qbel, lmalaf lazem ykoun kamel qbel ma trou7",
]

BODY_AR = [
    "أعلنت المصالح المعنية عن إجراءات جديدة تهدف إلى تبسيط العملية على المواطنين، "
    "حيث سيتم اعتماد المنصة الرقمية بشكل كامل بداية من الشهر المقبل.",
    "أوضح المسؤول في تصريح للصحافة أن العملية ستمس عددا كبيرا من المستفيدين عبر "
    "مختلف ولايات الوطن، مؤكدا أن الآجال ستحترم.",
    "تشهد الولاية حركية كبيرة هذه الأيام مع انطلاق المشاريع الجديدة التي من شأنها "
    "تحسين الإطار المعيشي للسكان في مختلف البلديات.",
    "دعت الجهات المعنية المواطنين إلى ضرورة استكمال الملفات في الآجال المحددة، "
    "مشيرة إلى أن كل ملف ناقص سيتم رفضه تلقائيا.",
]

BODY_FR = [
    "Les services concernes ont annonce de nouvelles mesures visant a simplifier la "
    "procedure pour les citoyens, avec une numerisation complete des le mois prochain.",
    "Le responsable a precise que l'operation touchera un grand nombre de beneficiaires "
    "a travers plusieurs wilayas du pays, assurant que les delais seront respectes.",
    "La wilaya connait une dynamique importante ces derniers jours avec le lancement "
    "de nouveaux projets destines a ameliorer le cadre de vie des habitants.",
]

DOMAINS = [
    ("elkhabar.dz", "elkhabar-dz", "A"), ("echoroukonline.dz", "echorouk-dz", "A"),
    ("aps.dz", "aps-dz", "A"), ("liberte.dz", "liberte-dz", "B"),
    ("annahar.dz", "annahar-dz", "B"), ("emploi.dz", "emploi-dz", "C"),
    ("forum-dz.dz", "forum-dz", "C"), ("interieur.gov.dz", "interieur-gov-dz", "A"),
]

GROUPS = [
    ("Groupe Emploi Alger", "fb-emploi-alger"),
    ("سوق وهران للإعلانات", "fb-souk-oran"),
    ("Etudiants Algerie", "fb-etudiants-dz"),
    ("أخبار قسنطينة", "fb-akhbar-constantine"),
]

ARABIC_INDIC = "٠١٢٣٤٥٦٧٨٩"


def to_arabic_indic(n: int) -> str:
    return "".join(ARABIC_INDIC[int(d)] for d in str(n))


def add_tatweel(text: str, rng: random.Random) -> str:
    """Insert kashida the way real Arabic web copy does."""
    words = text.split(" ")
    for i, w in enumerate(words):
        if len(w) > 4 and rng.random() < 0.25:
            pos = rng.randint(1, len(w) - 2)
            words[i] = w[:pos] + "ـ" * rng.randint(1, 4) + w[pos:]
    return " ".join(words)


def add_harakat(text: str, rng: random.Random) -> str:
    marks = "َُِّْ"
    out = []
    for ch in text:
        out.append(ch)
        if "ء" <= ch <= "ي" and rng.random() < 0.15:
            out.append(rng.choice(marks))
    return "".join(out)


def normalize_for_hash(text: str) -> str:
    """Mirror of xustive-text::normalize, enough for a stable content_hash in fixtures."""
    text = unicodedata.normalize("NFKC", text)
    out = []
    for ch in text:
        cp = ord(ch)
        if cp == 0x0640:                       # tatweel
            continue
        if 0x064B <= cp <= 0x065F or cp == 0x0670:   # harakat
            continue
        if 0x0610 <= cp <= 0x061A:
            continue
        if 0x06D6 <= cp <= 0x06ED:
            continue
        if cp in (0x00AD, 0xFEFF) or 0x200B <= cp <= 0x200F or 0x202A <= cp <= 0x202E:
            continue
        if 0x0660 <= cp <= 0x0669:
            ch = chr(ord("0") + cp - 0x0660)
        elif 0x06F0 <= cp <= 0x06F9:
            ch = chr(ord("0") + cp - 0x06F0)
        out.append(ch)
    return " ".join("".join(out).lower().split())


def make_document(i: int, rng: random.Random, now: int) -> dict:
    kind = rng.choices(
        ["web_ar", "web_fr", "fb_darija_ar", "fb_darija_latin", "ig", "tiktok"],
        weights=[35, 20, 20, 10, 8, 7],
    )[0]

    age_days = rng.choices([0, 1, 3, 7, 30, 120, 400], weights=[8, 12, 15, 20, 25, 12, 8])[0]
    published = now - age_days * 86400 - rng.randint(0, 86399)
    crawled = min(now, published + rng.randint(3600, 172800))

    wilaya_ar, wilaya_fr, wilaya_code = rng.choice(WILAYAS)
    org_ar, org_fr = rng.choice(ORGS)

    media = []
    entities = []
    topics = []
    precision = "second"

    if kind == "web_ar":
        domain, source_id, tier = rng.choice(DOMAINS[:5] + [DOMAINS[7]])
        topic, headline = rng.choice(TOPICS_AR)
        title = f"{headline} بـ{wilaya_ar}"
        body = f"{rng.choice(BODY_AR)} {org_ar} أكدت أن العملية تخص {to_arabic_indic(rng.randint(100, 9999))} مستفيد بولاية {wilaya_ar}."
        title = add_tatweel(title, rng)
        if rng.random() < 0.3:
            body = add_harakat(body, rng)
        url = f"https://www.{domain}/article/{100000 + i}"
        source_type, language, script = "web", "ar", "arabic"
        entities = [wilaya_ar, org_ar]
        topics = [topic]
        if rng.random() < 0.15:
            precision = "unknown"

    elif kind == "web_fr":
        domain, source_id, tier = rng.choice(DOMAINS)
        topic, headline = rng.choice(TOPICS_FR)
        title = f"{headline} a {wilaya_fr}"
        body = f"{rng.choice(BODY_FR)} {org_fr} a confirme que l'operation concerne {rng.randint(100, 9999)} beneficiaires."
        url = f"https://www.{domain}/fr/{100000 + i}"
        source_type, language, script = "web", "fr", "latin"
        entities = [wilaya_fr, org_fr]
        topics = [topic]

    elif kind in ("fb_darija_ar", "fb_darija_latin"):
        group_name, source_id = rng.choice(GROUPS)
        if kind == "fb_darija_ar":
            body = f"{rng.choice(DARIJA_AR)} في {wilaya_ar} مع {org_ar}"
            language, script = "ary", "arabic"
        else:
            body = f"{rng.choice(DARIJA_LATIN)} f {wilaya_fr} m3a {org_fr}"
            language, script = "ary", "latin"
        title = body[:80]
        domain = "facebook.com"
        url = f"https://www.facebook.com/groups/{rng.randint(10**14, 10**15)}/posts/{10**14 + i}"
        source_type = "facebook"
        entities = [wilaya_ar if script == "arabic" else wilaya_fr]
        # Group posts frequently have no reliable timestamp on the cheap access paths.
        if rng.random() < 0.25:
            precision = "day"

    elif kind == "ig":
        source_id, domain = "ig-dz-news", "instagram.com"
        topic, headline = rng.choice(TOPICS_AR)
        body = "" if rng.random() < 0.4 else f"{headline} #{wilaya_fr.lower()} #dz"
        title = (body[:80] or headline)
        url = f"https://www.instagram.com/p/{''.join(rng.choices('ABCDEFGHJKLMNPQRSTUVWXYZ0123456789', k=11))}/"
        source_type, language, script = "instagram", "ar", "arabic"
        media = [{
            "type": "image",
            "url": f"https://cdn.example.dz/ig/{i}.jpg",
            "thumb_url": f"https://cdn.example.dz/ig/{i}_t.jpg",
            "width": 1080, "height": 1350,
            # The image-first case: the real text is inside the picture.
            "ocr_text": headline if not body else None,
            "ocr_lang": "ar",
        }]
        topics = [topic]

    else:  # tiktok
        source_id, domain = "tt-dz-creators", "tiktok.com"
        body = rng.choice(DARIJA_LATIN)
        title = body[:80]
        url = f"https://www.tiktok.com/@creator{rng.randint(1,200)}/video/{7 * 10**18 + i}"
        source_type, language, script = "tiktok", "ary", "latin"
        media = [{
            "type": "video",
            "url": url,
            "thumb_url": f"https://cdn.example.dz/tt/{i}_cover.jpg",
            "width": 720, "height": 1280,
        }]
        topics = ["dz", wilaya_fr.lower()]

    if source_type == "web":
        source_id_final, tier_final = source_id, tier
    else:
        source_id_final, tier_final = source_id, "C"

    # Sentiment: mostly neutral, with a realistic tail. Low confidence stays neutral so the UI
    # shows no badge, which is what the spec asks for.
    label = rng.choices(["neutral", "positive", "negative"], weights=[55, 22, 23])[0]
    confidence = round(rng.uniform(0.2, 0.95), 2)
    if confidence < 0.35:
        label = "neutral"
    score = {"neutral": rng.uniform(-0.14, 0.14),
             "positive": rng.uniform(0.16, 0.9),
             "negative": rng.uniform(-0.9, -0.16)}[label]

    engagement = {"likes": 0, "comments": 0, "shares": 0, "views": 0, "captured_at": crawled}
    if source_type != "web":
        engagement["likes"] = int(rng.paretovariate(1.3) * 12)
        engagement["comments"] = int(engagement["likes"] * rng.uniform(0.05, 0.4))
        engagement["shares"] = int(engagement["likes"] * rng.uniform(0.0, 0.15))
        if source_type == "tiktok":
            engagement["views"] = engagement["likes"] * rng.randint(15, 300)

    excerpt = (body or title)[:320]
    norm = normalize_for_hash(f"{title} {body}")

    return {
        "id": f"01J{i:019d}"[:26].upper(),
        "content_hash": "b3:" + hashlib.blake2b(norm.encode(), digest_size=32).hexdigest(),
        "url": url,
        "canonical_url": url,
        "domain": domain,
        "source_type": source_type,
        "source_id": source_id_final,
        "title": title,
        "excerpt": excerpt,
        "body": body or title,
        "body_len": len(body or title),
        "body_source": "ocr" if (source_type == "instagram" and not body) else "native",
        "language": language,
        "language_confidence": round(rng.uniform(0.55, 0.98), 2),
        "script": script,
        "author": {
            "name": f"محرر {rng.randint(1, 40)}" if script == "arabic" else f"Auteur {rng.randint(1, 40)}",
            "handle": f"user{rng.randint(1, 500)}",
            "verified": rng.random() < 0.1,
        },
        "published_at": published,
        "crawled_at": crawled,
        "indexed_at": crawled + rng.randint(1, 120),
        "published_at_precision": precision,
        "sentiment": {
            "label": label,
            "score": round(score, 3),
            "confidence": confidence,
            "model": "fixture@0",
        },
        "engagement": engagement,
        "comments_count": engagement["comments"],
        "media": media,
        "entities": entities,
        "topics": topics,
        "geo": {"wilaya": wilaya_code, "wilaya_name": wilaya_fr},
        "quality_score": round(
            {"A": rng.uniform(0.6, 0.95), "B": rng.uniform(0.4, 0.8), "C": rng.uniform(0.2, 0.6)}[tier_final], 3
        ),
        "spam_score": round(rng.choices([rng.uniform(0, 0.2), rng.uniform(0.8, 0.99)],
                                        weights=[95, 5])[0], 3),
        "is_nsfw": False,
        "robots_indexable": True,
        "http_status": 200,
        "fetch_method": "static" if source_type == "web" else "api",
        "schema_version": 1,
    }


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--count", type=int, default=10000)
    p.add_argument("--out", type=Path, default=Path("tests/fixtures/corpus/documents.jsonl"))
    p.add_argument("--seed", type=int, default=20260806, help="fixed for reproducible fixtures")
    args = p.parse_args()

    rng = random.Random(args.seed)
    now = int(time.time())

    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("w", encoding="utf-8") as fh:
        for i in range(args.count):
            doc = make_document(i, rng, now)
            # Duplicates are part of the data: the same item cross-posted, which is what
            # deduplication has to catch.
            fh.write(json.dumps(doc, ensure_ascii=False) + "\n")
            if rng.random() < 0.04:
                dup = dict(doc)
                dup["id"] = f"01JD{i:018d}"[:26].upper()
                dup["url"] = doc["url"].rstrip("/") + "?utm_source=share"
                dup["source_id"] = "forum-dz"
                dup["domain"] = "forum-dz.dz"
                fh.write(json.dumps(dup, ensure_ascii=False) + "\n")

    size = args.out.stat().st_size
    print(f"wrote {args.out} ({size / 1_048_576:.1f} MiB)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
