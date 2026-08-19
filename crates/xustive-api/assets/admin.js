// Device settings form.
//
// An external file rather than an inline <script>: `script-src 'self'` in the CSP drops inline
// script, and the page must keep working under the same policy as the rest of the site.
//
// Progressive enhancement is not attempted here. Unlike /search, which is server-rendered so it
// works without JavaScript, this form is an operator tool used from a normal browser.
document.addEventListener('DOMContentLoaded', () => {
  const form = document.getElementById('device-form');
  if (!form) return;
  const out = document.getElementById('result');

  form.addEventListener('submit', async (event) => {
    event.preventDefault();
    const data = new FormData(form);
    const layers = parseInt(data.get('gpu_layers'), 10);
    out.textContent = 'saving…';

    try {
      const res = await fetch('/admin/device', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          preference: data.get('preference'),
          // A negative value means "decide from free memory", which the API spells as null.
          gpu_layers: Number.isNaN(layers) || layers < 0 ? null : layers,
        }),
      });
      const body = await res.json();
      if (res.ok) {
        out.textContent = 'saved';
        // Reload so the page shows what the setting actually resolved to, which is not always
        // what was asked for — a GPU request on a machine without one still lands on CPU.
        setTimeout(() => location.reload(), 500);
      } else {
        out.textContent = body.error ? body.error.message : 'could not save';
      }
    } catch (err) {
      out.textContent = 'could not reach the server';
    }
  });
});

// The politeness bypass.
//
// Absent on deployments where it is refused, so everything here is guarded rather than assumed —
// a null dereference at load would take the device controls down with it.
(function () {
  const form = document.getElementById('politeness-form');
  if (!form) return;
  const box = document.getElementById('ignore-politeness');

  form.addEventListener('submit', async function (e) {
    e.preventDefault();
    const res = await fetch('/admin/politeness', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ignore_politeness: box.checked }),
    });
    // Reloaded rather than patched in place, so the banner state always matches the server's.
    // A checkbox that says one thing while the crawler does another is the failure this whole
    // control has to avoid.
    if (res.ok) {
      location.reload();
    } else {
      const body = await res.json().catch(function () { return null; });
      alert((body && body.error && body.error.message) || 'Could not change the setting.');
    }
  });
})();

// --- the console ---------------------------------------------------------------------------
//
// One SSE connection for every live number on the page. Several would mean several reconnect
// storms whenever the API restarts, and they would drift apart between frames.
(function () {
  var live = document.getElementById('crawl-tiles');
  var bar = document.getElementById('statusbar');
  if (!bar) return;

  function text(id, v) { var el = document.getElementById(id); if (el) el.textContent = v; }
  function esc(s) { return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
    return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#x27;' }[c];
  }); }

  var lastIndexed = null, lastAt = null;

  function render(s) {
    var dot = document.getElementById('sb-dot');
    // "Unavailable" is not "idle". A zero and an unreachable Redis look identical, and the second
    // is the one that needs attention — so it is said, not implied.
    if (s.unavailable) {
      text('sb-state', 'state unknown');
      if (dot) dot.className = 'dot bad';
      var warn = document.getElementById('crawl-unavailable');
      if (warn) warn.hidden = false;
      return;
    }
    var warnOk = document.getElementById('crawl-unavailable');
    if (warnOk) warnOk.hidden = true;
    text('sb-state', s.state);
    if (dot) dot.className = 'dot' + (s.state === 'running' ? ' running' : '');

    // Rate is derived here from absolute counters, so a dropped frame costs nothing.
    var now = Date.now();
    if (lastIndexed !== null && now > lastAt) {
      var perMin = ((s.indexed - lastIndexed) / ((now - lastAt) / 60000));
      if (isFinite(perMin) && perMin >= 0) text('sb-rate', Math.round(perMin) + '/min');
    }
    lastIndexed = s.indexed; lastAt = now;

    if (!live) return;
    text('c-indexed', s.indexed);
    text('c-fetched', s.fetched);
    text('c-revisited', s.revisited);
    text('c-discovered', s.discovered);
    text('c-waiting', s.waiting);
    text('c-deferred', s.deferred);
    text('c-failed', s.failed);

    var recent = document.getElementById('crawl-recent');
    if (recent && s.recent) {
      recent.innerHTML = s.recent.map(function (r) {
        return '<tr><td><span class="outcome ' + esc(r.outcome) + '">' + esc(r.outcome) + '</span></td>' +
          '<td>' + (r.words || '') + '</td><td>' + esc(r.host) + '</td>' +
          '<td title="' + esc(r.url) + '">' + esc(r.url) + '</td></tr>';
      }).join('');
    }

    var skips = document.getElementById('crawl-skips');
    if (skips && s.skipped) {
      var rows = Object.keys(s.skipped).sort(function (a, b) { return s.skipped[b] - s.skipped[a]; });
      skips.innerHTML = rows.map(function (k) {
        return '<tr><th>' + esc(k) + '</th><td>' + s.skipped[k] + '</td></tr>';
      }).join('');
    }

    var hosts = document.getElementById('crawl-hosts');
    if (hosts && s.hosts) {
      var hs = Object.keys(s.hosts).sort(function (a, b) { return s.hosts[b] - s.hosts[a]; }).slice(0, 20);
      hosts.innerHTML = hs.map(function (h) {
        var ago = Math.max(0, Math.round(Date.now() / 1000 - s.hosts[h]));
        return '<tr><th>' + esc(h) + '</th><td>' + ago + 's ago</td></tr>';
      }).join('');
    }
  }

  var es = new EventSource('/admin/crawler/events');
  es.onmessage = function (e) { try { render(JSON.parse(e.data)); } catch (_) {} };
  es.onerror = function () {
    var dot = document.getElementById('sb-dot');
    if (dot) dot.className = 'dot bad';
    text('sb-state', 'disconnected');
  };
})();

// --- documents -----------------------------------------------------------------------------
(function () {
  var form = document.getElementById('doc-filters');
  if (!form) return;
  var rows = document.getElementById('doc-rows');
  var count = document.getElementById('doc-count');
  var pager = document.getElementById('doc-pager');
  var page = 1;
  var totalPages = 1;

  function esc(s) { return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
    return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#x27;' }[c];
  }); }

  // Prev / page-of / Next. Built once; the handlers read the live `page`/`totalPages`.
  function renderPager() {
    if (!pager) return;
    if (totalPages <= 1) { pager.innerHTML = ''; return; }
    pager.className = 'pager';
    pager.innerHTML =
      '<button type="button" id="doc-prev"' + (page <= 1 ? ' disabled' : '') + '>← Prev</button>' +
      '<span class="page-info">Page ' + page + ' of ' + totalPages + '</span>' +
      '<button type="button" id="doc-next"' + (page >= totalPages ? ' disabled' : '') + '>Next →</button>';
    var prev = document.getElementById('doc-prev');
    var next = document.getElementById('doc-next');
    if (prev) prev.onclick = function () { if (page > 1) { page--; load(); window.scrollTo(0, 0); } };
    if (next) next.onclick = function () { if (page < totalPages) { page++; load(); window.scrollTo(0, 0); } };
  }

  function load() {
    var p = new URLSearchParams({
      q: document.getElementById('doc-q').value,
      host: document.getElementById('doc-host').value,
      lang: document.getElementById('doc-lang').value,
      page: page,
    });
    fetch('/admin/crawler/documents?' + p).then(function (r) { return r.json(); }).then(function (d) {
      if (d.error) { count.textContent = d.error.message; return; }
      var total = d.estimated_total || 0;
      var perPage = d.per_page || 20;
      totalPages = Math.max(1, Math.min(100, Math.ceil(total / perPage)));
      count.textContent = total + ' document' + (total === 1 ? '' : 's') +
        (total > perPage ? ' — showing ' + ((page - 1) * perPage + 1) + '–' + Math.min(page * perPage, total) : '');
      rows.innerHTML = (d.hits || []).map(function (h) {
        var when = h.published_at ? new Date(h.published_at * 1000).toISOString().slice(0, 16).replace('T', ' ') : '';
        // Titles come from crawled pages. Escaped, never rendered — an admin console that renders
        // untrusted markup is stored XSS aimed at the most privileged account in the system.
        // `body_len` is the extracted body's real length, which is what distinguishes an article
        // from a navigation page. This column previously measured the *excerpt*, which is capped —
        // so it was reporting the truncation, not the document.
        var words = h.body_len != null ? h.body_len : (h.excerpt ? h.excerpt.length : '');
        return '<tr><td title="' + esc(h.title) + '"><a href="' + esc(h.url) + '" rel="noopener nofollow">' + esc(h.title || h.url) + '</a></td>' +
          '<td>' + esc(h.domain || '') + '</td><td>' + esc(h.language || '') + '</td>' +
          '<td>' + words + '</td><td>' + when + '</td></tr>';
      }).join('');
      renderPager();
    }).catch(function () { count.textContent = 'could not reach the index'; });
  }

  form.addEventListener('submit', function (e) { e.preventDefault(); page = 1; load(); });
  // `/` focuses search. This is a tool used repetitively by one person, which is exactly when
  // shortcuts pay for themselves.
  document.addEventListener('keydown', function (e) {
    if (e.key === '/' && document.activeElement.tagName !== 'INPUT') {
      e.preventDefault(); document.getElementById('doc-q').focus();
    }
  });
  load();
})();

// --- sources ---------------------------------------------------------------------------------
(function () {
  var form = document.getElementById('seed-form');
  if (!form) return;
  var rows = document.getElementById('seed-rows');
  var msg = document.getElementById('seed-msg');

  function esc(s) { return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
    return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#x27;' }[c];
  }); }

  function load() {
    fetch('/admin/crawler/sources').then(function (r) { return r.json(); }).then(function (d) {
      // Seed URLs come from a file an operator edits, so they are escaped like anything else —
      // this console is the highest-privilege page in the system.
      rows.innerHTML = (d.seeds || []).map(function (s) {
        return '<tr><td>' + esc(s.source_id) + '</td><td>' + esc(s.trust) + '</td>' +
          '<td title="' + esc(s.url) + '"><a href="' + esc(s.url) + '" rel="noopener nofollow">' + esc(s.url) + '</a></td>' +
          '<td>' + esc(s.note) + '</td>' +
          '<td><button class="rm" data-url="' + esc(s.url) + '">remove</button></td></tr>';
      }).join('');
    });
  }

  form.addEventListener('submit', function (e) {
    e.preventDefault();
    msg.textContent = 'adding…';
    fetch('/admin/crawler/sources', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        url: document.getElementById('seed-url').value,
        trust: document.getElementById('seed-trust').value,
      }),
    }).then(function (r) { return r.json(); }).then(function (d) {
      if (d.error) { msg.textContent = d.error.message; return; }
      msg.textContent = d.already_listed
        ? 'already listed — queued to crawl next'
        : 'added as ' + d.source_id + ' and queued to crawl next';
      document.getElementById('seed-url').value = '';
      load();
    }).catch(function () { msg.textContent = 'could not add it'; });
  });

  rows.addEventListener('click', function (e) {
    var btn = e.target.closest('.rm');
    if (!btn) return;
    // Confirmed with the URL, so it is obvious which row is going. A bare "are you sure" on a
    // table of fifty rows is a rubber stamp.
    if (!confirm('Stop crawling ' + btn.dataset.url + '?\nDocuments already collected stay in the index.')) return;
    fetch('/admin/crawler/sources/remove', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: btn.dataset.url }),
    }).then(function (r) { return r.json(); }).then(function (d) {
      msg.textContent = d.error ? d.error.message : 'removed — already-crawled documents remain';
      load();
    });
  });

  load();
})();

// --- shared helper for the fetch-and-render pages below --------------------------------------
function xEsc(s) {
  return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
    return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#x27;' }[c];
  });
}
function xPct(v) { return v == null ? '<span class="muted">—</span>' : (v * 100).toFixed(0) + '%'; }

// --- discovery yield (/admin/discovery) ------------------------------------------------------
// These live in admin.js, not an inline <script>, because the console's CSP is `script-src 'self'`
// — an inline script is blocked and the page would sit forever on "Loading…".
(function () {
  var rows = document.getElementById('ch-rows');
  if (!rows) return;
  var msg = document.getElementById('ch-msg');
  function load() {
    fetch('/admin/crawler/channels', { headers: { accept: 'application/json' } })
      .then(function (r) { if (!r.ok) throw new Error(r.status); return r.json(); })
      .then(function (d) {
        var cs = d.channels || [];
        rows.innerHTML = cs.map(function (c) {
          return '<tr><td>' + xEsc(c.channel) + '</td><td>' + c.discovered + '</td><td>' + c.fetched +
            '</td><td>' + c.indexed + '</td><td>' + c.duplicate + '</td><td>' + xPct(c.yield_rate) +
            '</td><td>' + xPct(c.unique_rate) + '</td></tr>';
        }).join('');
        msg.textContent = cs.length ? cs.length + ' channel(s). Refreshing every 10s.'
          : 'No discovery activity recorded yet.';
      })
      .catch(function () { msg.textContent = 'Could not load discovery yield.'; });
  }
  load();
  setInterval(load, 10000);
})();

// --- source health (/admin/sources/health) ---------------------------------------------------
(function () {
  var rows = document.getElementById('sh-rows');
  if (!rows) return;
  var msg = document.getElementById('sh-msg');
  // A cell is amber when its value is outside the §7 healthy band.
  function cell(v, ok) {
    if (v == null) return '<td><span class="muted">—</span></td>';
    var cls = ok(v) ? '' : ' class="warn"';
    return '<td' + cls + '>' + (v * 100).toFixed(0) + '%</td>';
  }
  function load() {
    fetch('/admin/crawler/sources/health', { headers: { accept: 'application/json' } })
      .then(function (r) { if (!r.ok) throw new Error(r.status); return r.json(); })
      .then(function (d) {
        var ss = d.sources || [];
        rows.innerHTML = ss.map(function (s) {
          var q = s.quality, c = s.counts;
          return '<tr><td>' + xEsc(s.display_name || s.id) + ' <span class="muted">' + xEsc(s.id) +
            '</span></td><td>' + xEsc(s.lifecycle || '—') + '</td><td>' + xEsc(s.trust_tier || '—') +
            '</td><td>' + c.fetched + '</td><td>' + c.indexed + '</td>' +
            cell(q.fetch_success_rate, function (v) { return v > 0.95; }) +
            cell(q.extraction_success_rate, function (v) { return v > 0.90; }) +
            cell(q.duplicate_ratio, function (v) { return v < 0.30; }) +
            cell(q.spam_mean, function (v) { return v < 0.20; }) +
            cell(q.date_unknown_ratio, function (v) { return v < 0.10; }) + '</tr>';
        }).join('');
        msg.textContent = ss.length + ' source(s). Refreshing every 10s.';
      })
      .catch(function () { msg.textContent = 'Could not load source health.'; });
  }
  load();
  setInterval(load, 10000);
})();

// --- weak coverage (/admin/weak-coverage) ----------------------------------------------------
(function () {
  var rows = document.getElementById('wc-rows');
  if (!rows) return;
  var msg = document.getElementById('wc-msg');
  var kEl = document.getElementById('wc-k');
  function load() {
    fetch('/admin/crawler/weak-coverage', { headers: { accept: 'application/json' } })
      .then(function (r) { if (!r.ok) throw new Error(r.status); return r.json(); })
      .then(function (d) {
        if (kEl) kEl.textContent = d.k_anonymity;
        if (!d.enabled) {
          rows.innerHTML = '';
          msg.textContent = 'Query-driven discovery is off. Set discovery.weak_coverage_enabled to collect gaps.';
          return;
        }
        var ts = d.terms || [];
        // The term is a user's search text — build the cell with textContent, never innerHTML.
        rows.innerHTML = '';
        ts.forEach(function (t) {
          var tr = document.createElement('tr');
          var term = document.createElement('td'); term.textContent = t.term;
          var count = document.createElement('td'); count.textContent = t.count;
          tr.appendChild(term); tr.appendChild(count); rows.appendChild(tr);
        });
        msg.textContent = ts.length
          ? ts.length + ' coverage gap(s), each searched ≥ ' + d.k_anonymity + ' times.'
          : 'No coverage gaps yet — search for something the corpus does not have.';
      })
      .catch(function () { msg.textContent = 'Could not load weak coverage.'; });
  }
  load();
  setInterval(load, 15000);
})();
