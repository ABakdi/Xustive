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
