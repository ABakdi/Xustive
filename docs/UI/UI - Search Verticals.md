---
tags:
  - ui
---

# UI — Search Verticals

> The tab row above results: All, News, Images, Videos, Short videos, Files, Social.
>
> Interface here; the content each depends on is in [[Image Pipeline]], [[Web Fetcher]] and the
> social connectors.

---

## 1. The problem with building all of them now

A tab is a promise that there is something behind it. Today most of these have nothing:

| Tab | Content exists? | Blocked on |
|:---|:---|:---|
| **All** | yes | — |
| **News** | yes | — a filter over what is already indexed |
| **Files** | **no** | the fetcher accepts seven text MIME types and refuses PDFs outright ([[Web Fetcher]] `INDEXABLE`) |
| **Images** | **no** | [[Image Pipeline]] — M3 |
| **Videos** | **no** | video metadata; no source produces it |
| **Short videos** | **no** | TikTok / Reels — the social connectors, M2-T08–T10 |
| **Social** | **no** | same |

Shipping seven tabs where five return "no results" is worse than shipping two. An empty tab is
indistinguishable from a broken one, and the reader's conclusion is that the engine does not work
rather than that the feature is unfinished.

**So tabs appear as their content does.** The row renders the verticals that can return something,
and each new one lights up when its pipeline lands.

## 2. What each tab actually is

Not separate indexes. One corpus, filtered — which is why News and Files are cheap and Images is
not.

| Tab | Definition |
|:---|:---|
| All | no filter |
| News | `source_type = web` and the document has a publication date and an article-shaped body |
| Files | `content_type` in a document set — PDF, DOCX, XLSX, PPTX |
| Images | documents from [[Image Pipeline]], ranked by CLIP embedding and OCR text |
| Videos | documents with a video `media[]` entry over a duration threshold |
| Short videos | video documents under that threshold, plus platform origin |
| Social | `source_type != web` |

The News definition is worth arguing about: "has a date" is doing most of the work. A section index
and an article are both HTML on a news site, and the date is the cheapest signal that separates
them. It will be wrong sometimes — an undated article is excluded — which is the right direction
to be wrong in for a tab whose whole promise is recency.

## 3. Behaviour

- **The tab is in the URL** (`?v=news`), so a vertical is shareable and the back button works. A
  tab that only exists in memory is a tab that vanishes when someone sends a link.
- **The query survives switching.** Changing tab re-runs the same query, never clears it.
- **Counts are not shown per tab.** Getting them means running every vertical on every search —
  seven queries to render one row. Google shows no counts either, and for the same reason.
- **An empty vertical says which vertical is empty** — "no images for `سونلغاز`" — with a link back
  to All. A generic "no results" leaves the reader unsure whether the engine has nothing or the
  tab is broken.
- **Keyboard**: the row is a `role="tablist"`, arrow keys move between tabs, and the current one
  carries `aria-current`. It is a navigation control before it is a visual one.

## 4. Files needs a decision first

Indexing PDFs is not a UI change. It means:

- Accepting `application/pdf` in the fetcher, which currently refuses it.
- Extracting text, which needs a PDF library and a hard page cap — a PDF bomb is a real thing and
  the adversarial suite already covers the HTML equivalent.
- A size limit well below the HTML one. Government sites publish 40 MB scanned decrees, and a
  scanned PDF has no text at all — OCR is [[Image Pipeline]], not this.

That last point matters for Algeria specifically: a large share of `.gov.dz` PDFs are scans of
printed documents. Text extraction returns nothing for them, so a Files tab built without OCR
would cover the born-digital minority and silently miss the rest.

## 5. Order of work

1. **News** — a filter over existing content. Real on day one.
2. **Files** — fetcher change, PDF extraction, caps. Useful for `.gov.dz` immediately.
3. **Social** — arrives with the connectors, which are already tracked.
4. **Images / Videos / Short videos** — M3.

## 6. Related

[[UI - Results Page]] · [[UI - Filters and Facets]] · [[Image Pipeline]] · [[Web Fetcher]] ·
[[Milestone 3 - Multimodal Input]]
