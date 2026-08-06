---
tags:
  - ui
type: ui
status: specified
updated: 2026-08-06
---

# UI - Image Search

> The camera/upload flow. Backend: [[Image Pipeline]] · API: [[API Contract]] §6
> Parent: [[UI Specification]]

---

## 1. Why This Matters Here

The dominant Algerian use case is **screenshots**. Job postings, official announcements, price lists,
and event details circulate through Facebook groups as images, not as text. A user who receives such
a screenshot and wants to know more currently has to retype it. Image search removes that step.

This shapes the whole design: OCR is the primary path, and visual similarity is the fallback — the
opposite of how most reverse-image search products are built.

---

## 2. Flow

```
[📷 tap]
   → source choice: Take photo · Choose image · Paste (Ctrl/⌘+V) · Drag & drop (lg)
   → local preview + crop (optional)
   → upload with progress
   → server decides mode:
        OCR usable      → text appears IN THE SEARCH BOX, editable, not submitted
        OCR unusable    → visual similarity results render directly
```

As with voice, **OCR text is never auto-submitted**. It lands in the search box for the user to check
and correct — Arabic OCR on a photographed sign is unreliable enough that reviewing it is faster than
re-searching ([[Image Pipeline]] §4.2).

---

## 3. Input Methods

| Method | Availability | Notes |
|:---|:---|:---|
| Camera | `sm` with camera | `<input type="file" accept="image/*" capture="environment">` — uses the native camera app, no in-page camera |
| File picker | everywhere | `accept="image/jpeg,image/png,image/webp"` |
| Paste | `lg` | `paste` event on the document reads `clipboardData.files` — this is *the* screenshot path on desktop |
| Drag & drop | `lg` | full-page drop zone with an overlay on `dragenter` |

Using the native camera rather than an in-page `getUserMedia` viewfinder is deliberate: it is faster,
handles focus/flash/orientation properly, and avoids holding a camera stream open in a page whose
core promise is privacy.

---

## 4. Client-Side Preprocessing

Before upload, in a `canvas`:

| Step | Rule |
|:---|:---|
| Downscale | longest edge → 2048 px (keeps OCR quality, cuts upload size dramatically) |
| Re-encode | JPEG q0.85, or keep PNG when the source is a PNG screenshot (text edges matter) |
| Orientation | apply EXIF orientation, then discard all EXIF |
| Size check | reject > 8 MB before upload with a clear message |

A 12 MP phone photo goes from ~4 MB to ~350 KB. On 3G that is the difference between a usable feature
and an abandoned one.

**EXIF is stripped client-side as well as server-side.** Belt and braces: GPS coordinates should not
even leave the device ([[Image Pipeline]] §4.1).

---

## 5. Crop Step (optional)

Shown after preview, skippable with one tap. A draggable rectangle with a "Crop to text" hint —
cropping to the relevant region measurably improves OCR when a screenshot contains chrome, other
posts, or a keyboard.

Default: no crop, full image. The crop UI must never be a required step.

---

## 6. Results

### OCR path

```
┌────────────────────────────────────────┐
│ [thumbnail 64×64]  Text found in image │
│                                        │
│  "إعلان توظيف: مطلوب مهندس معلوماتية…"  │  ← editable, in the search box
│                                        │
│  [ Search this text ]  [ Find similar images ] │
└────────────────────────────────────────┘
```

Both actions are always offered — OCR may have succeeded while the user actually wanted the image.

### Similarity path

A responsive grid (2 cols `sm`, 4 `lg`) of matches. Each tile: thumbnail, source badge, date,
similarity as a **qualitative** label ("very similar" ≥ 0.9, "similar" ≥ 0.8, "possibly related"
≥ 0.75) rather than a raw cosine score — a number invites false precision. Tapping a tile opens the
source document card.

Empty: "No similar images found" + "Try searching by text instead", with the OCR text if any was
extracted at all.

---

## 7. States

| State | UI |
|:---|:---|
| `idle` | camera icon in the search box |
| `choosing` | source sheet (`sm`) or file dialog (`lg`) |
| `preview` | image + Crop / Search / Cancel |
| `uploading` | progress bar with real byte progress, cancellable |
| `analyzing` | "Reading the image…" |
| `ocr-result` | §6 OCR panel |
| `similar-result` | §6 grid |
| `error` | inline, per §8 |

---

## 8. Error Handling

| Error | Message | Recovery |
|:---|:---|:---|
| File too large (client) | "Image is too large (max 8 MB)" | choose another |
| Unsupported type | "Use a JPEG, PNG, or WebP image" | choose another |
| 415 / 422 `image_unreadable` | "We couldn't read this image" | Retry / choose another |
| Decode failure client-side | "This image seems corrupted" | choose another |
| 503 | "Image search is busy — try again" | Retry |
| Network failure mid-upload | "Upload failed" | Retry, blob kept in memory |
| No OCR text and no similar images | empty state with a text-search suggestion | — |

---

## 9. Privacy Surface

Shown at the preview step:

> Your image is analysed on our servers in Algeria and is never stored. Location data is removed
> before upload.

Both halves are enforced, not just stated: EXIF stripping happens client-side (§4) and server-side
([[Image Pipeline]] §4.1), and no uploaded image is written to disk
([[Security and Privacy]] P4).

Also worth stating explicitly, because users reasonably worry about it: **Xustive does not do face
recognition.** Reverse image search here matches whole images, not people
([[Image Pipeline]] §10). If we ever surface a "who is this" capability, that promise breaks — so we
do not build one.

---

## 10. Accessibility

- The camera button has `aria-label="Search by image"`.
- The preview `<img>` gets `alt="Image you selected"`; extracted OCR text is exposed as real text,
  not baked into an image — which incidentally makes the OCR path the most accessible part of the
  product.
- Similarity tiles have accessible names: "Result 3 of 12, very similar, from Instagram, 4 August
  2026".
- The drag-and-drop zone has a keyboard-reachable equivalent (the file picker button) — drag and drop
  is never the only route.
- Upload progress announces at 0 %, 50 %, 100 % via a polite live region, not continuously.

---

## 11. Performance

| Metric | Budget |
|:---|:---|
| Client downscale of a 12 MP image | ≤ 400 ms |
| Upload (350 KB on 3G) | ~3 s, with real progress shown |
| Server analysis | ≤ 500 ms ([[Performance Budgets]]) |
| Grid render (20 tiles) | ≤ 16 ms |

Thumbnails in the grid are lazy-loaded with fixed boxes, so a slow CDN never shifts the layout.

---

## 12. Open Questions

- [ ] Should "Find similar" also be offered on every result card that has an image?
- [ ] Is the crop step worth its complexity, or does client-side downscaling plus server `--psm 11`
      handle cluttered screenshots well enough?
- [ ] Do we show *why* an image matched (the shared region)? Genuinely useful, but CLIP embeddings
      cannot explain themselves.
- [ ] Multi-image upload — realistic use case, or scope creep?

## Related

[[Image Pipeline]] · [[Vector Index]] · [[API Contract]] · [[UI - States and Errors]] ·
[[UI - Accessibility]] · [[Security and Privacy]]
