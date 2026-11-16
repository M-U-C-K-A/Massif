// The few things the site does in the browser: the theme, the menu, the
// download button for the visitor's system, the gallery's lightbox and the
// guide's "On this page".
(() => {
  const root = document.documentElement;
  const base = (document.querySelector('link[rel=stylesheet]')?.getAttribute('href') || '/assets/style.css').replace(/\/assets\/style\.css$/, '');

  // Theme: the system's until the visitor picks one.
  const dark = () => (root.dataset.theme ? root.dataset.theme === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches);
  document.querySelector('.theme-btn')?.addEventListener('click', () => {
    const t = dark() ? 'light' : 'dark';
    root.dataset.theme = t;
    try { localStorage.setItem('massif-theme', t); } catch (e) {}
  });

  // Menu on narrow screens.
  const btn = document.querySelector('.menu-btn'), sheet = document.getElementById('sheet');
  btn?.addEventListener('click', () => {
    const open = sheet.hidden;
    sheet.hidden = !open;
    btn.setAttribute('aria-expanded', String(open));
  });

  // The visitor's system, when it is one Massif runs on.
  const ua = navigator.userAgent;
  const platform = (navigator.userAgentData?.platform || navigator.platform || '').toLowerCase();
  const mobile = /android|iphone|ipad|ipod/i.test(ua) || (navigator.maxTouchPoints > 1 && /mac/.test(platform));
  const os = mobile ? '' : /win/.test(platform) || /windows/i.test(ua) ? 'windows' : /mac/.test(platform) || /mac os/i.test(ua) ? 'macos' : /linux|x11/.test(platform) || /linux/i.test(ua) ? 'linux' : '';

  const apply = (dl) => {
    if (!dl || !os || !dl[os]) return;
    document.querySelectorAll('.dl-auto').forEach((a) => {
      a.href = dl[os].url;
      a.querySelector('.dl-label').textContent = `Download for ${dl[os].label}`;
    });
    document.querySelectorAll('.dl-meta').forEach((m) => {
      m.textContent = `${dl.version ? `Version ${dl.version} · ` : ''}${dl[os].label}${dl[os].size ? `, ${dl[os].size}` : ''} · `;
      const other = document.createElement('a');
      other.href = `${base}/download/`;
      other.textContent = 'Other systems';
      other.style.textDecoration = 'underline';
      other.style.textUnderlineOffset = '3px';
      m.append(other);
    });
  };
  if (document.querySelector('.dl-auto') && os) {
    fetch(`${base}/assets/release.json`).then((r) => r.json()).then(apply).catch(() => {});
  }
  const mine = os && document.querySelector(`.os-card[data-os="${os}"]`);
  if (mine) { mine.classList.add('mine'); mine.parentElement.prepend(mine); }

  // Gallery.
  const box = document.querySelector('.lightbox');
  if (box) {
    document.querySelectorAll('.shot-open').forEach((b) => b.addEventListener('click', () => {
      const img = box.querySelector('img');
      img.src = b.dataset.full;
      img.alt = b.dataset.alt;
      box.showModal();
    }));
    box.addEventListener('click', (e) => { if (e.target === box) box.close(); });
  }

  // The guide's "On this page" follows the reading.
  const toc = document.querySelectorAll('.toc a');
  if (toc.length && 'IntersectionObserver' in window) {
    const heads = [...toc].map((a) => document.getElementById(decodeURIComponent(a.hash.slice(1)))).filter(Boolean);
    const seen = new Set();
    const io = new IntersectionObserver((es) => {
      es.forEach((e) => (e.isIntersecting ? seen.add(e.target) : seen.delete(e.target)));
      const first = heads.find((h) => seen.has(h));
      if (first) toc.forEach((a) => a.classList.toggle('on', a.hash === '#' + first.id));
    }, { rootMargin: '-70px 0px -60% 0px' });
    heads.forEach((h) => io.observe(h));
  }
})();
