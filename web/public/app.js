/* Xustive — progressive enhancement.
 *
 * The server already renders /search without any JavaScript. This script upgrades it to a
 * client-side fetch so navigation does not reload the page, and keeps the URL as the single
 * source of truth so back/forward and link sharing behave correctly.
 *
 * Nothing here is required for search to work. If this file fails to load, the forms still
 * submit and the server renders results.
 *
 * Deliberately absent: analytics, cookies, localStorage of anything query-shaped, and any
 * third-party request. The CSP (`default-src 'self'`) blocks those regardless.
 */

(() => {
  'use strict';

  const MAX_QUERY = 512;

  // --- escaping -------------------------------------------------------------------
  // Result text comes from crawled pages. Everything is escaped; only the <em> markers the
  // search engine inserted are re-admitted, and only after escaping. Never innerHTML on raw
  // server text.

  function escapeHtml(s) {
    return String(s ?? '')
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#x27;');
  }

  function escapePreservingEm(s) {
    return escapeHtml(s)
      .replace(/&lt;em&gt;/g, '<em>')
      .replace(/&lt;\/em&gt;/g, '</em>');
  }

  // --- formatting -----------------------------------------------------------------

  const nf = new Intl.NumberFormat();

  // Day-month-year, matching the server renderer exactly.
  //
  // Enhancement must not change content. Using the browser locale here would render
  // "August 5, 2026" client-side against the server's "5 August 2026", so a card would visibly
  // reflow the moment JavaScript took over.
  //
  // The locale is pinned to `en-GB` for day-month-year ordering rather than left to the browser.
  // Localised month names — including the Algerian forms (أوت, not أغسطس) — land with the i18n
  // work in M1; doing it half-way now would mean two wrong answers instead of one.
  const DATE_FMT = new Intl.DateTimeFormat('en-GB', {
    year: 'numeric', month: 'long', day: 'numeric', timeZone: 'UTC',
  });
  const MONTH_FMT = new Intl.DateTimeFormat('en-GB', {
    year: 'numeric', month: 'long', timeZone: 'UTC',
  });

  function fmtDate(ts, precision) {
    // We never render a date we guessed as though it were fact.
    if (!ts || precision === 'unknown') return '<span class="muted">date unknown</span>';
    const d = new Date(ts * 1000);
    const label = (precision === 'month' ? MONTH_FMT : DATE_FMT).format(d);
    return `<time datetime="${escapeHtml(d.toISOString())}">${escapeHtml(label)}</time>`;
  }

  const PLATFORM = { web: 'Web', facebook: 'Facebook', instagram: 'Instagram', tiktok: 'TikTok' };
  const GLYPH = { positive: '▲', negative: '▼', neutral: '●' };

  // --- rendering ------------------------------------------------------------------

  function renderCard(c) {
    // Sentiment is omitted entirely below the confidence floor — the API already returns null
    // in that case. Colour is never the only carrier: glyph + text label too.
    const sentiment = c.sentiment
      ? `<span class="badge sentiment ${escapeHtml(c.sentiment.label)}">` +
        `${GLYPH[c.sentiment.label] || '●'} ${escapeHtml(c.sentiment.label)}</span>`
      : '';

    return `<li class="result-card" dir="auto" id="result-${escapeHtml(c.id)}">
      <div class="card-meta">
        <span class="badge platform ${escapeHtml(c.source_type)}">${escapeHtml(PLATFORM[c.source_type] || c.source_type)}</span>
        <span class="display-url">${escapeHtml(c.display_url)}</span>
        ${fmtDate(c.published_at, c.published_at_precision)}
        ${sentiment}
      </div>
      <h3><a href="${escapeHtml(c.url)}" rel="noopener nofollow">${escapePreservingEm(c.title)}</a></h3>
      <p class="excerpt">${escapePreservingEm(c.excerpt)}</p>
    </li>`;
  }

  function renderEmpty(q) {
    const tips = [];
    if (/[35792]/.test(q) && /^[\x00-\x7F]*$/.test(q)) {
      tips.push('Try writing the query in Arabic script');
    }
    tips.push('Check the spelling', 'Use fewer or more general words', 'Remove any filters');
    return `<div class="empty-state">
      <p class="empty-title">No results for “${escapeHtml(q)}”</p>
      <ul class="empty-tips">${tips.map((t) => `<li>${escapeHtml(t)}</li>`).join('')}</ul>
    </div>`;
  }

  function renderSkeleton() {
    // Dimensions match a real card so nothing shifts when results arrive.
    let out = '<p class="result-count">&nbsp;</p><ol class="result-list">';
    for (let i = 0; i < 5; i++) {
      out += `<li class="result-card" aria-hidden="true">
        <div class="skeleton" style="inline-size:40%"></div>
        <div class="skeleton" style="inline-size:80%;block-size:1.3em"></div>
        <div class="skeleton" style="inline-size:100%"></div>
        <div class="skeleton" style="inline-size:65%"></div>
      </li>`;
    }
    return out + '</ol>';
  }

  function renderResults(q, data) {
    const p = data.pagination;
    const count = `<p class="result-count">${p.estimated ? 'about ' : ''}` +
      `${nf.format(p.total_hits)} results (${data.took_ms} ms)</p>`;

    if (!data.results.length) return count + renderEmpty(q);

    // An empty container, filled later if a summary arrives. Reserving no height is deliberate:
    // a placeholder that collapses when nothing comes back moves the results under the reader's
    // cursor, and most summaries do not arrive.
    const slot = data.summary_token ? '<div id="summary" hidden></div>' : '';
    const filters = renderFilters(data.facets);
    const list = `<ol class="result-list">${data.results.map(renderCard).join('')}</ol>`;
    return count + filters + slot + list + renderPagination(q, p);
  }

  // --- summary ----------------------------------------------------------------------

  // Fetched after the results paint, never with them. Generation takes seconds on CPU, and no
  // part of the page waits for it.
  async function fetchSummary(token, signal) {
    const slot = document.getElementById('summary');
    if (!slot || !token) return;

    let data;
    try {
      const res = await fetch('/api/v1/summary', {
        method: 'POST',
        signal,
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ token }),
      });
      data = await res.json();
    } catch (err) {
      return; // Including aborts. No summary is a normal outcome, not an error to show.
    }

    if (!data || !data.summary) return;

    // The model's text is untrusted output derived from untrusted crawled pages. It is escaped
    // and inserted as text; only the citation markers we generate ourselves become markup.
    const cites = new Map((data.citations || []).map((c) => [c.n, c.result_id]));
    const html = escapeHtml(data.summary).replace(/\[(\d+)\]/g, (m, n) => {
      const id = cites.get(Number(n));
      return id
        ? `<a class="citation" href="#result-${escapeHtml(id)}" aria-label="Source ${n}">[${n}]</a>`
        : '';
    });

    slot.innerHTML = `<div class="summary-body" dir="auto">${html}</div>` +
      '<p class="summary-note" dir="auto">Generated from the results below. Check the sources.</p>';
    slot.hidden = false;
    announce('Summary available');
  }

  function renderPagination(q, p) {
    const total = Math.max(p.total_pages, 1);
    if (total <= 1) return '';
    const href = (n) => `/search?q=${encodeURIComponent(q)}&page=${n}`;
    let out = '<nav class="pagination" aria-label="Pagination">';
    if (p.page > 1) out += `<a class="page" href="${href(p.page - 1)}">‹ Previous</a>`;
    const start = Math.max(1, p.page - 2);
    for (let n = start; n <= Math.min(start + 4, total); n++) {
      out += n === p.page
        ? `<span class="page current" aria-current="page">${n}</span>`
        : `<a class="page" href="${href(n)}">${n}</a>`;
    }
    if (p.page < total) out += `<a class="page" href="${href(p.page + 1)}">Next ›</a>`;
    return out + '</nav>';
  }

  // --- live region -----------------------------------------------------------------

  let liveRegion;
  function announce(msg) {
    if (!liveRegion) {
      liveRegion = document.createElement('div');
      liveRegion.setAttribute('aria-live', 'polite');
      liveRegion.setAttribute('role', 'status');
      liveRegion.style.cssText =
        'position:absolute;width:1px;height:1px;overflow:hidden;clip:rect(0 0 0 0);';
      document.body.appendChild(liveRegion);
    }
    liveRegion.textContent = msg;
  }

  // --- search ----------------------------------------------------------------------

  let inFlight = null;

  async function runSearch(q, page, push) {
    const main = document.getElementById('results');
    if (!main) return false;

    // A slower earlier response must not overwrite a newer one.
    if (inFlight) inFlight.abort();
    const controller = new AbortController();
    inFlight = controller;

    main.innerHTML = renderSkeleton();

    // Active filters ride along. Without this, typing a new query silently drops the narrowing
    // the user just applied — and the chips would still render as active, which is worse than
    // dropping them visibly.
    const params = new URLSearchParams({ q, page, hits_per_page: 20 });
    for (const g of FACET_GROUPS) {
      const v = currentParams().get(g.param);
      if (v) params.set(g.param, v);
    }
    const url = '/api/v1/search?' + params.toString();
    try {
      const res = await fetch(url, { signal: controller.signal, headers: { Accept: 'application/json' } });
      const data = await res.json();

      if (!res.ok) {
        const msg = (data && data.error && data.error.message) || 'Something went wrong.';
        main.innerHTML = `<div class="empty-state"><p class="empty-title">${escapeHtml(msg)}</p></div>`;
        announce(msg);
        return true;
      }

      main.innerHTML = renderResults(q, data);
      announce(`${nf.format(data.pagination.total_hits)} results`);
      // Not awaited: the search is complete without it.
      fetchSummary(data.summary_token, controller.signal);
      if (push) {
        const target = `/search?q=${encodeURIComponent(q)}` + (page > 1 ? `&page=${page}` : '');
        history.pushState({ q, page }, '', target);
      }
      document.title = `${q} — Xustive`;
      return true;
    } catch (err) {
      if (err.name === 'AbortError') return true;
      // Let the browser fall back to a full page load rather than showing a broken shell.
      return false;
    } finally {
      if (inFlight === controller) inFlight = null;
    }
  }

  // --- wiring -----------------------------------------------------------------------

  // The results page is server-rendered, so on a fresh load there is a summary slot in the DOM
  // that no client-side search created. Pick up its token and fetch the summary the same way.
  function claimRenderedSummary() {
    const slot = document.getElementById('summary');
    const token = slot && slot.dataset.token;
    if (!token) return;
    delete slot.dataset.token;
    fetchSummary(token);
  }

  // --- strings ----------------------------------------------------------------------
  //
  // The UI language follows the page, not the query: a Darija speaker searching in French still
  // wants the chrome in the language they chose. `document.documentElement.lang` is the single
  // source of truth, set server-side.
  //
  // Kept in one object rather than fetched, because a filter label arriving after the filters
  // is worse than one that was never translated.
  const STRINGS = {
    ar: {
      filters: 'تصفية', all: 'الكل', language: 'اللغة', source: 'المصدر',
      sentiment: 'الانطباع', clear: 'مسح التصفية', results: 'نتيجة',
      positive: 'إيجابي', neutral: 'محايد', negative: 'سلبي',
      web: 'موقع', facebook: 'فيسبوك', instagram: 'إنستغرام', tiktok: 'تيك توك',
      lang_ar: 'العربية', lang_ary: 'الدارجة', lang_fr: 'الفرنسية',
      lang_en: 'الإنجليزية', lang_mixed: 'مختلط',
    },
    fr: {
      filters: 'Filtrer', all: 'Tout', language: 'Langue', source: 'Source',
      sentiment: 'Ton', clear: 'Effacer les filtres', results: 'résultats',
      positive: 'Positif', neutral: 'Neutre', negative: 'Négatif',
      web: 'Web', facebook: 'Facebook', instagram: 'Instagram', tiktok: 'TikTok',
      lang_ar: 'Arabe', lang_ary: 'Darija', lang_fr: 'Français',
      lang_en: 'Anglais', lang_mixed: 'Mixte',
    },
    en: {
      filters: 'Filter', all: 'All', language: 'Language', source: 'Source',
      sentiment: 'Tone', clear: 'Clear filters', results: 'results',
      positive: 'Positive', neutral: 'Neutral', negative: 'Negative',
      web: 'Web', facebook: 'Facebook', instagram: 'Instagram', tiktok: 'TikTok',
      lang_ar: 'Arabic', lang_ary: 'Darija', lang_fr: 'French',
      lang_en: 'English', lang_mixed: 'Mixed',
    },
  };
  // Darija falls back to Arabic rather than to English. Someone who set the interface to Darija
  // reads Arabic; sending them to English would be a strictly worse guess.
  const UI_LANG = (() => {
    const declared = (document.documentElement.lang || 'ar').slice(0, 3);
    if (STRINGS[declared]) return declared;
    if (declared.startsWith('ar')) return 'ar';
    return STRINGS[declared.slice(0, 2)] ? declared.slice(0, 2) : 'ar';
  })();
  const t = (key) => (STRINGS[UI_LANG] && STRINGS[UI_LANG][key]) || STRINGS.en[key] || key;

  // --- filters ------------------------------------------------------------------------
  //
  // Facets have been in the search response since the beginning and nothing consumed them.
  // Rendered as links rather than as script-driven controls so the whole thing works without
  // JavaScript on the server-rendered page — a filter that needs script is a filter that
  // disappears on the connection where narrowing results matters most.

  const FACET_GROUPS = [
    { key: 'language', param: 'lang', label: 'language', prefix: 'lang_' },
    { key: 'source_type', param: 'source', label: 'source', prefix: '' },
    { key: 'sentiment.label', param: 'sentiment', label: 'sentiment', prefix: '' },
  ];

  function currentParams() {
    return new URLSearchParams(location.search);
  }

  function facetHref(param, value) {
    const params = currentParams();
    // Toggling: clicking an active facet clears it. A filter you cannot undo by clicking the
    // thing you clicked is a filter people abandon the page to escape.
    if (params.get(param) === value) params.delete(param);
    else params.set(param, value);
    params.delete('page'); // Narrowing invalidates the page number.
    return '/search?' + params.toString();
  }

  function renderFilters(facets) {
    if (!facets) return '';
    const params = currentParams();
    let groups = '';

    for (const group of FACET_GROUPS) {
      const counts = facets[group.key];
      if (!counts) continue;
      const values = Object.entries(counts)
        .filter(([, n]) => n > 0)
        .sort((a, b) => b[1] - a[1]);
      const active = params.get(group.param);
      // A facet with one value narrows nothing — unless it is the active filter, in which case
      // hiding it leaves no way to clear what was applied.
      if (values.length < 2 && !active) continue;
      const chips = values
        .map(([value, count]) => {
          const on = active === value;
          const label = t(group.prefix + value) || value;
          return `<a class="chip${on ? ' active' : ''}" href="${escapeHtml(facetHref(group.param, value))}"` +
            `${on ? ' aria-current="true"' : ''}>` +
            `${escapeHtml(label)} <span class="chip-count">${nf.format(count)}</span></a>`;
        })
        .join('');

      groups += `<div class="facet-group" role="group" aria-label="${escapeHtml(t(group.label))}">` +
        `<span class="facet-label">${escapeHtml(t(group.label))}</span>${chips}</div>`;
    }

    if (!groups) return '';
    const hasActive = FACET_GROUPS.some((g) => params.get(g.param));
    const clear = hasActive
      ? `<a class="chip clear" href="/search?q=${encodeURIComponent(params.get('q') || '')}">` +
        `${escapeHtml(t('clear'))}</a>`
      : '';
    return `<div class="filters">${groups}${clear}</div>`;
  }

  // --- suggestions ------------------------------------------------------------------
  //
  // An ARIA combobox, because a list of options that appears under an input and responds to
  // arrow keys is a combobox whether or not it is labelled as one — and a screen reader that is
  // not told about it reads the input as empty while sighted users see eight suggestions.
  //
  // Nothing about a prefix is stored client-side either. There is no local history, because the
  // most private thing a search engine holds is what you started to type and then deleted.

  const SUGGEST_DEBOUNCE_MS = 90;
  const MIN_SUGGEST_CHARS = 2;

  function attachSuggestions(form) {
    const input = form.querySelector('input[name="q"]');
    if (!input) return;

    const list = document.createElement('ul');
    list.className = 'suggestions';
    list.id = 'suggestions-' + Math.random().toString(36).slice(2, 9);
    list.setAttribute('role', 'listbox');
    list.hidden = true;
    form.appendChild(list);

    input.setAttribute('role', 'combobox');
    input.setAttribute('aria-expanded', 'false');
    input.setAttribute('aria-controls', list.id);
    input.setAttribute('aria-autocomplete', 'list');

    let items = [];
    let active = -1;
    let timer = null;
    let inFlightSuggest = null;
    // What the user actually typed, kept so arrowing through options and back returns it.
    let typed = '';

    function close() {
      list.hidden = true;
      list.innerHTML = '';
      items = [];
      active = -1;
      input.setAttribute('aria-expanded', 'false');
      input.removeAttribute('aria-activedescendant');
    }

    function highlight(index) {
      active = index;
      [...list.children].forEach((li, i) => {
        const on = i === index;
        li.setAttribute('aria-selected', on ? 'true' : 'false');
        li.classList.toggle('active', on);
      });
      if (index >= 0) {
        input.value = items[index].text;
        input.setAttribute('aria-activedescendant', list.children[index].id);
      } else {
        input.value = typed;
        input.removeAttribute('aria-activedescendant');
      }
    }

    function render(suggestions) {
      items = suggestions;
      if (!items.length) return close();

      list.innerHTML = items
        .map((s, i) =>
          `<li id="${list.id}-${i}" role="option" aria-selected="false" dir="auto" ` +
          `data-index="${i}">${escapeHtml(s.text)}</li>`)
        .join('');
      list.hidden = false;
      active = -1;
      input.setAttribute('aria-expanded', 'true');
    }

    async function fetchSuggestions(prefix) {
      if (inFlightSuggest) inFlightSuggest.abort();
      const controller = new AbortController();
      inFlightSuggest = controller;
      try {
        const res = await fetch(
          '/api/v1/suggest?q=' + encodeURIComponent(prefix) + '&limit=8',
          { signal: controller.signal, headers: { Accept: 'application/json' } },
        );
        if (!res.ok) return close();
        const data = await res.json();
        // The prefix may have moved on while this was in flight.
        if (input.value.trim() !== prefix) return;
        render(data.suggestions || []);
      } catch (err) {
        // Including aborts. A suggestion box that shows an error is worse than one that shows
        // nothing: the user is mid-keystroke and did not ask a question.
        close();
      } finally {
        if (inFlightSuggest === controller) inFlightSuggest = null;
      }
    }

    input.addEventListener('input', () => {
      typed = input.value;
      const prefix = typed.trim();
      clearTimeout(timer);
      if (prefix.length < MIN_SUGGEST_CHARS) return close();
      // Debounced so a fast typist sends one request per pause rather than one per key. Short
      // enough that the list still feels attached to the keyboard.
      timer = setTimeout(() => fetchSuggestions(prefix), SUGGEST_DEBOUNCE_MS);
    });

    input.addEventListener('keydown', (e) => {
      if (list.hidden) return;
      // Arrow semantics do not flip in RTL: down is still "further down the list". Only the
      // horizontal axis mirrors, and this list has no horizontal axis.
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        highlight(active + 1 >= items.length ? -1 : active + 1);
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        highlight(active - 1 < -1 ? items.length - 1 : active - 1);
      } else if (e.key === 'Enter') {
        // Let the form submit; just commit the highlighted option first.
        if (active >= 0) input.value = items[active].text;
        close();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        input.value = typed;
        close();
      } else if (e.key === 'Tab') {
        close();
      }
    });

    list.addEventListener('mousedown', (e) => {
      // mousedown, not click: blur fires first on click and would close the list before the
      // selection registered.
      const li = e.target.closest('li[data-index]');
      if (!li) return;
      e.preventDefault();
      input.value = items[Number(li.dataset.index)].text;
      close();
      form.requestSubmit ? form.requestSubmit() : form.submit();
    });

    input.addEventListener('blur', () => setTimeout(close, 120));
    form.addEventListener('submit', close);
  }

  function enhanceForms() {
    document.querySelectorAll('form[role="search"]').forEach((form) => {
      form.addEventListener('submit', (e) => {
        const input = form.querySelector('input[name="q"]');
        const q = (input?.value || '').trim();
        if (!q) {
          e.preventDefault();
          return;
        }
        if (q.length > MAX_QUERY) return; // let the server return the contract error
        if (!document.getElementById('results')) return; // home page: normal navigation

        e.preventDefault();
        runSearch(q, 1, true).then((handled) => {
          if (!handled) form.submit();
        });
      });
      attachSuggestions(form);
    });
  }

  function enhancePagination() {
    // Delegated, so it survives re-rendering the results list.
    document.addEventListener('click', (e) => {
      const link = e.target.closest?.('a.page');
      if (!link || !document.getElementById('results')) return;
      const url = new URL(link.href, location.origin);
      if (url.pathname !== '/search') return;

      e.preventDefault();
      const q = url.searchParams.get('q') || '';
      const page = parseInt(url.searchParams.get('page') || '1', 10);
      runSearch(q, page, true).then((handled) => {
        if (handled) window.scrollTo({ top: 0, behavior: 'smooth' });
        else location.assign(link.href);
      });
    });
  }

  window.addEventListener('popstate', (e) => {
    if (!document.getElementById('results')) return;
    const params = new URLSearchParams(location.search);
    const q = e.state?.q ?? params.get('q');
    const page = e.state?.page ?? parseInt(params.get('page') || '1', 10);
    if (q) runSearch(q, page, false);
  });

  enhanceForms();
  enhancePagination();
  claimRenderedSummary();
})();
