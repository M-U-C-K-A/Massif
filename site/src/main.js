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

  // Every version, when the site was built without GitHub's answer.
  const versions = document.querySelector('.versions[data-empty] .versions-list');
  if (versions) {
    const repo = document.querySelector('.versions-head a')?.href.replace(/\/releases$/, '').replace('https://github.com/', '');
    fetch(`https://api.github.com/repos/${repo}/releases?per_page=50`).then((r) => r.json()).then((list) => {
      versions.innerHTML = list.filter((r) => !r.draft && !r.prerelease).map((r) => `<details class="version"><summary><span class="v-name">${r.tag_name.replace(/^v/, '')}</span><span class="v-date">${new Date(r.published_at).toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric' })}</span></summary><div class="v-body"><div class="v-files">${r.assets.map((a) => `<a class="file-pill" href="${a.browser_download_url}">${a.name}<span>${Math.round(a.size / 1e6)} MB</span></a>`).join('')}</div><a class="v-link" href="${r.html_url}" rel="noopener">On GitHub</a></div></details>`).join('');
    }).catch(() => {});
  }

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
  // The guide's search: titles first, then headings, then the text.
  const boxes = document.querySelectorAll('[data-search]');
  if (boxes.length) {
    let index = null;
    const load = () => index || (index = fetch(`${base}/assets/search.json`).then((r) => r.json()).catch(() => []));
    const escH = (s) => s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
    const mark = (s, q) => escH(s).replace(new RegExp(`(${q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'ig'), '<mark>$1</mark>');
    const find = (pages, q) => {
      const lq = q.toLowerCase(), out = [];
      for (const p of pages) {
        if (p.t.toLowerCase().includes(lq)) out.push({ score: 100 - p.t.length / 10, u: p.u, g: p.g, t: p.t, s: p.s });
        for (const [h, id] of p.h) if (h.toLowerCase().includes(lq)) out.push({ score: 50, u: `${p.u}#${id}`, g: `${p.g} · ${p.t}`, t: h, s: '' });
        const i = p.x.toLowerCase().indexOf(lq);
        if (i >= 0 && !p.t.toLowerCase().includes(lq)) out.push({ score: 10, u: p.u, g: p.g, t: p.t, s: (i > 40 ? '…' : '') + p.x.slice(Math.max(0, i - 40), i + 80) + '…' });
      }
      const seen = new Set();
      return out.sort((a, b) => b.score - a.score).filter((r) => !seen.has(r.u) && seen.add(r.u)).slice(0, 8);
    };
    boxes.forEach((input) => {
      const list = input.parentElement.querySelector('.results');
      let on = -1;
      const show = async () => {
        const q = input.value.trim();
        if (q.length < 2) { list.hidden = true; return; }
        const hits = find(await load(), q);
        on = -1;
        list.innerHTML = hits.length ? hits.map((r) => `<a href="${r.u}"><small>${escH(r.g)}</small>${mark(r.t, q)}${r.s ? `<span>${mark(r.s, q)}</span>` : ''}</a>`).join('') : '<p>Nothing found.</p>';
        list.hidden = false;
      };
      input.addEventListener('input', show);
      input.addEventListener('focus', () => { load(); if (input.value.trim().length > 1) show(); });
      input.addEventListener('blur', () => setTimeout(() => { list.hidden = true; }, 150));
      input.addEventListener('keydown', (e) => {
        const links = [...list.querySelectorAll('a')];
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault();
          on = (on + (e.key === 'ArrowDown' ? 1 : -1) + links.length) % links.length;
          links.forEach((a, k) => a.classList.toggle('on', k === on));
        } else if (e.key === 'Enter' && links.length) {
          location.href = (links[on] || links[0]).href;
        } else if (e.key === 'Escape') { input.value = ''; list.hidden = true; input.blur(); }
      });
    });
    document.addEventListener('keydown', (e) => {
      if (e.key !== '/' || /input|textarea/i.test(document.activeElement?.tagName)) return;
      const visible = [...boxes].find((b) => b.offsetParent);
      if (visible) { e.preventDefault(); visible.focus(); }
    });
  }
})();
