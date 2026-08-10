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
    text('c-discovered', s.discovered);
    text('c-waiting', s.waiting);
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
  var page = 1;

  function esc(s) { return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
    return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#x27;' }[c];
  }); }

  function load() {
    var p = new URLSearchParams({
      q: document.getElementById('doc-q').value,
      host: document.getElementById('doc-host').value,
      lang: document.getElementById('doc-lang').value,
      page: page,
    });
    fetch('/admin/crawler/documents?' + p).then(function (r) { return r.json(); }).then(function (d) {
      if (d.error) { count.textContent = d.error.message; return; }
      count.textContent = (d.estimated_total || 0) + ' documents';
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
