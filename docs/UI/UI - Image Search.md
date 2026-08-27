---
tags:
  - ui
type: ui
status: built
updated: 2026-08-27
---

# UI - Image Search

> The camera/upload flow. Backend: [[Image Pipeline]] · API: [[API Contract]] §6
> Parent: [[UI Specification]] · Code: `web/app/[lang]/tools/ocr/page.tsx` (the page),
> `web/components/tools/ImageOcr.tsx` (the island), `ocrImage()` / `imageSearch()` in
> `web/lib/api.ts`, the Camera link in `web/components/search/SearchBox.tsx`

---

## 0. Current behaviour (2026-08-27)

Built as **a page, not an overlay**. The camera icon in every search box is a real link to
`/[lang]/tools/ocr` (`aria-label`/`title` = `t.ocrByImage`), which opens in a new tab if wanted
and exists without JavaScript. On that page:

- A dashed drop zone (`ImageOcr`) with two buttons — `t.ocrChoose` (plain
  `<input type="file" accept="image/*">`) and `t.ocrCamera` (`capture="environment"`) — both
  inputs visually hidden (`sr-only`) and driven by the buttons. Drag & drop onto the zone and
  **paste anywhere on the page** also work.
- Before upload the image is downscaled to ≤ 2048 px on the long edge and re-encoded as JPEG
  q0.92 through a canvas, which drops every EXIF field including GPS.
- `POST /api/v1/ocr` with the raw bytes as the body. The text lands in an editable `<textarea>`
  (`dir="auto"`) with **Search this** (`t.ocrSearchThis` → `/[lang]/search?q=…`) and **Copy**.
  Nothing is searched automatically.
- **Find similar** (`t.ocrFindSimilar`, quiet button) is offered whether or not OCR found text:
  `POST /api/v1/search/image` with the same prepared blob. Matches render as a list (title →
  page, display URL, qualitative label: `similarityVery` ≥ 0.92, `similaritySimilar` ≥ 0.82,
  else `similarityRelated`). A 503 shows `t.ocrSimilarUnavailable`.
- Privacy line under the zone (`t.ocrPrivacy`): the image is read on this server and never
  stored.

The rest of this note is the original specification, annotated where it differs.

---

## 1. Why This Matters Here

The dominant Algerian use case is **screenshots**. Job postings, official announcements, price lists,
and event details circulate through Facebook groups as images, not as text. A user who receives such
a screenshot and wants to know more currently has to retype it. Image search removes that step.

This shapes the whole design: OCR is the primary path, and visual similarity is the fallback — the
opposite of how most reverse-image search products are built.

---

## 2. Flow

Original:

```
[📷 tap]
   → source choice: Take photo · Choose image · Paste (Ctrl/⌘+V) · Drag & drop (lg)
   → local preview + crop (optional)
   → upload with progress
   → server decides mode:
        OCR usable      → text appears IN THE SEARCH BOX, editable, not submitted
        OCR unusable    → visual similarity results render directly
```

As built (2026-08-27): the tap navigates to the tool page; there is no crop step and no progress
bar; the server does **not** choose a mode — OCR always runs first, and similarity is a separate
button the reader presses. The text lands in the tool's own textarea rather than the search box,
and "Search this" carries it to the results page.

As with voice, **OCR text is never auto-submitted**. It waits for the user to check and correct
it — Arabic OCR on a photographed sign is unreliable enough that reviewing it is faster than
re-searching ([[Image Pipeline]] §4.2). (Voice later moved to submit-on-stop because its words are
visible while spoken; OCR has no equivalent, so the rule stands here.)

---

## 3. Input Methods

| Method | Availability | Notes |
|:---|:---|:---|
| Camera | any device with a camera | `<input type="file" accept="image/*" capture="environment">` — uses the native camera app, no in-page camera |
| File picker | everywhere | `accept="image/*"` (the original listed jpeg/png/webp explicitly; anything `image/*` is accepted and non-images are ignored) |
| Paste | everywhere | `paste` listener on `window` reads the first `image/*` clipboard item — this is *the* screenshot path on desktop |
| Drag & drop | everywhere | the drop zone itself (`dragover` highlights with `--accent` / `--accent-wash`); not full-page |

Using the native camera rather than an in-page `getUserMedia` viewfinder is deliberate: it is faster,
handles focus/flash/orientation properly, and avoids holding a camera stream open in a page whose
core promise is privacy.

---

## 4. Client-Side Preprocessing

Before upload, in a `canvas` (`downscale()` in `ImageOcr.tsx`):

| Step | Rule |
|:---|:---|
| Downscale | longest edge → 2048 px (`MAX_DIM`), never enlarged |
| Re-encode | JPEG q0.92 always (the original kept PNG for PNG screenshots — not built) |
| Orientation | `createImageBitmap(file, { imageOrientation: 'from-image' })` bakes EXIF rotation in; the canvas export carries no EXIF |
| Size check | none client-side (original: reject > 8 MB) — the server's 422 is the limit |
| Fallback | if the canvas cannot encode, the original bytes are sent |

A 12 MP phone photo goes from ~4 MB to a few hundred KB. On 3G that is the difference between a
usable feature and an abandoned one.

**EXIF is stripped client-side as well as server-side.** Belt and braces: GPS coordinates should not
even leave the device ([[Image Pipeline]] §4.1).

---

## 5. Crop Step (optional) — not built

The original planned a skippable crop rectangle after preview. It was not built; the preview is
shown (`<img alt="">`, `max-h-64`) and OCR starts immediately on selection. See §12.

---

## 6. Results

### OCR path (as built)

```
┌────────────────────────────────────────┐
│ [preview]                              │
│ النص المستخرج                            │  ← label (t.ocrResult)
│ ┌────────────────────────────────────┐ │
│ │ إعلان توظيف: مطلوب مهندس…           │ │  ← <textarea dir="auto">, editable
│ └────────────────────────────────────┘ │
│  الثقة منخفضة — راجع النص              │  ← t.ocrLowConfidence when !usable or confidence < 60
│  قراءة محسّنة (نموذج بصري)              │  ← t.ocrEnhanced when backend === 'unlimited'
│  [ ابحث عن هذا ]  [ نسخ ]               │
│  [ صور مشابهة ]                         │  ← always offered
└────────────────────────────────────────┘
```

No text at all → `t.ocrEmpty`, and "Find similar" is still offered — OCR may have found nothing
while the user actually wanted the image.

### Similarity path (as built)

A vertical **list**, not a grid (the original planned 2/4 columns of thumbnails — the API returns
documents, not images, so there is nothing to tile). Each row: title (link to the page),
`display_url` (`dir="ltr"` inside `<bdi>`), and a **qualitative** label rather than a raw cosine
score — a number invites false precision. Thresholds moved from the original 0.9/0.8/0.75 to
0.92/0.82/else.

Empty: `t.ocrNoSimilar`. Feature off or vector services down (503): `t.ocrSimilarUnavailable` —
a distinct, honest state, not a failure the user caused.

---

## 7. States

| State | UI |
|:---|:---|
| `idle` | drop zone with `t.ocrDrop` and the two buttons |
| `reading` | spinner + `t.ocrReading`, `aria-live="polite"` |
| `done` | §6 OCR panel (or `t.ocrEmpty`) |
| `failed` | `t.ocrFailed` in `--negative` |
| similarity `searching` / `done` / `unavailable` / `failed` | spinner / list / `t.ocrSimilarUnavailable` / `t.ocrFailed` |

There is no `choosing`, `preview`, `uploading` (with progress) or `analyzing` state as separate
UI; upload and analysis are one `reading` state.

---

## 8. Error Handling

| Error | Today |
|:---|:---|
| Non-image file | silently ignored (`file.type.startsWith('image/')`) |
| 415 / 422 `image_unreadable`, any other OCR failure | `t.ocrFailed` |
| Decode failure client-side (`createImageBitmap` throws) | `t.ocrFailed` |
| Similarity 503 | `t.ocrSimilarUnavailable` |
| Similarity other failure | `t.ocrFailed` |
| Network failure | `t.ocrFailed`; the prepared blob is kept in memory, so "Find similar" does not re-prepare, but a retry of OCR means choosing the file again |

The original's per-cause messages ("too large", "use a JPEG/PNG/WebP", "corrupted", "busy") are not
built; one message covers them.

---

## 9. Privacy Surface

Shown under the drop zone at all times (`t.ocrPrivacy`):

> The image is read on this server and is never stored.

The original line also promised "Location data is removed before upload"; that is true (§4) but
not stated on the page. Both halves are enforced, not just stated: EXIF stripping happens
client-side (§4) and server-side ([[Image Pipeline]] §4.1), and no uploaded image is written to
disk ([[Security and Privacy]] P4). Nothing is kept client-side either — the preview object URL
is revoked on replacement and unmount.

Also worth stating explicitly, because users reasonably worry about it: **Xustive does not do face
recognition.** Reverse image search here matches whole images, not people
([[Image Pipeline]] §10). If we ever surface a "who is this" capability, that promise breaks — so we
do not build one.

---

## 10. Accessibility

- The camera link in the search box has `aria-label` = `t.ocrByImage`.
- The tool is a `<section aria-label={t.ocrTitle}>` under an `<h1>`; the preview `<img>` has
  `alt=""` (decorative — the extracted text is the content); the textarea has a real `<label>`.
- Extracted OCR text is exposed as real text, not baked into an image — which incidentally makes
  the OCR path the most accessible part of the product.
- Similarity rows are plain links with a text label; no "Result 3 of 12" naming — not built.
- Drag & drop and paste are never the only route: the two buttons are keyboard-reachable.
- Reading / failed / similarity states are `aria-live="polite"`. No upload-progress announcements
  (there is no progress bar).

---

## 11. Performance

| Metric | Budget |
|:---|:---|
| Client downscale of a 12 MP image | ≤ 400 ms |
| Upload (a few hundred KB on 3G) | ~3 s (no progress shown today) |
| Server analysis | ≤ 500 ms ([[Performance Budgets]]) |

Not measured on 2026-08-27; targets only.

---

## 12. Open Questions

- [ ] Should "Find similar" also be offered on every result card that has an image?
- [ ] Is the crop step worth its complexity, or does client-side downscaling plus server `--psm 11`
      handle cluttered screenshots well enough? (Shipped without it; nobody has asked yet.)
- [ ] Do we show *why* an image matched (the shared region)? Genuinely useful, but CLIP embeddings
      cannot explain themselves.
- [ ] Multi-image upload — realistic use case, or scope creep?
- [ ] Should the result land back in the search box on the home page (original design) rather than
      on the tool page? The tool page is simpler and works without script; the round trip is one
      extra tap.

## Related

[[Image Pipeline]] · [[Vector Index]] · [[API Contract]] · [[UI - States and Errors]] ·
[[UI - Accessibility]] · [[Security and Privacy]] · [[UI - Search Verticals]] (the Images tab is a
different thing: it filters the index, this searches by an uploaded picture)
