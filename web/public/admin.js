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
