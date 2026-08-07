# Xustive web

Next.js 16 frontend. See `docs/UI/UI - Frontend Architecture.md` for the design, and
`docs/Decisions/ADR-0010 - Next.js for the Frontend.md` for why it exists.

```bash
npm install
npm run dev     # against a local xustive-api on :8080
npm run build && npm start
```

`XUSTIVE_API_URL` points at the Rust API (default `http://127.0.0.1:8080`). Server Components
call it directly; the browser reaches it through a rewrite so everything stays same-origin.

## Status

Built: home, results, filters, suggestions, summary, theming, i18n scaffolding.
Not yet: tool cards, settings page, font self-hosting, bundle budgets in CI, the no-JS CI check.

`web-legacy/` holds the Rust-served assets and is deleted once the port is complete
(M1B-T03.7) — two renderers is the problem being solved.
