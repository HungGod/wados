// The cactus:// pages' script, served at cactus://res/pages.js. What a page
// asks of the browser goes to its "cactus" script message handler as JSON,
// `{"do": "<action>", ...}` (wadbrowser_core::internal::PageAction reads
// it). Toggles flip and removed rows go at once; the browser re-renders the
// page when it next loads.
(() => {
  'use strict';

  /** Tells the browser; false if this page isn't in one that listens. */
  function post(msg) {
    const h = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.cactus;
    if (!h) return false;
    h.postMessage(JSON.stringify(msg));
    return true;
  }

  function args(el) {
    try {
      return el.dataset.args ? JSON.parse(el.dataset.args) : {};
    } catch {
      return {};
    }
  }

  // ------------------------------------------------------------ toggles, buttons, choices

  document.addEventListener('click', (e) => {
    const el = e.target.closest('[data-do], [data-back]');
    if (!el || el.disabled) return;
    if (el.hasAttribute('data-back')) {
      e.preventDefault();
      history.back();
      return;
    }
    if (el.tagName === 'SELECT') return;
    e.preventDefault();
    const msg = { do: el.dataset.do, ...args(el) };
    if (el.classList.contains('toggle')) {
      const on = !el.classList.contains('on');
      el.classList.toggle('on', on);
      el.setAttribute('aria-checked', String(on));
      msg.on = on;
    }
    post(msg);
    const row = el.hasAttribute('data-gone') && el.closest('[data-row]');
    if (row) gone(row);
  });

  /** A row that went; its list says why it's empty when it's the last (`data-empty`). */
  function gone(row) {
    const list = row.parentElement;
    row.remove();
    if (!list || list.dataset.empty === undefined || list.querySelector('[data-row]')) return;
    if (list.tagName === 'UL') {
      const p = document.createElement('p');
      p.className = 'empty';
      p.textContent = list.dataset.empty;
      list.replaceWith(p);
    } else {
      const r = document.createElement('div');
      r.className = 'row';
      const s = document.createElement('small');
      s.textContent = list.dataset.empty;
      r.append(s);
      list.append(r);
    }
  }

  document.addEventListener('change', (e) => {
    const el = e.target.closest('select[data-do]');
    if (el) post({ do: el.dataset.do, ...args(el), id: el.value });
  });

  // ------------------------------------------------------------ the new tab's search box

  const URLISH = /^[a-z][a-z0-9+.-]*:\/\/\S*$/i;
  const HOSTISH = /^(localhost|[a-z0-9-]+(\.[a-z0-9-]+)*\.[a-z]{2,}|\d{1,3}(\.\d{1,3}){3})(:\d+)?([/?#]\S*)?$/i;

  /** What the box's text goes to without the browser: an address, or a search (the template's %s). */
  function target(text, template) {
    if (URLISH.test(text)) return text;
    if (!/\s/.test(text) && HOSTISH.test(text)) return `${/^(localhost|\d)/i.test(text) ? 'http' : 'https'}://${text}`;
    return template.replace('%s', encodeURIComponent(text));
  }

  for (const form of document.querySelectorAll('form.search')) {
    form.addEventListener('submit', (e) => {
      e.preventDefault();
      const text = form.querySelector('input').value.trim();
      if (!text) return;
      if (!post({ do: 'go', to: text })) location.href = target(text, form.dataset.template || '');
    });
  }

  // ------------------------------------------------------------ a tile of your own: its menu

  let menu = null;
  function closeMenu() {
    if (!menu) return;
    menu.tile.classList.remove('menu-open');
    menu.el.remove();
    menu = null;
  }

  function item(label, icon, run, danger) {
    const b = document.createElement('button');
    b.className = danger ? 'item danger' : 'item';
    b.setAttribute('role', 'menuitem');
    b.innerHTML = icon;
    b.append(label);
    b.addEventListener('click', () => {
      closeMenu();
      run();
    });
    return b;
  }

  document.addEventListener('contextmenu', (e) => {
    const tile = e.target.closest('.tile.mine');
    closeMenu();
    if (!tile) return;
    e.preventDefault();
    const icons = JSON.parse(document.getElementById('menu-icons')?.textContent || '{}');
    const el = document.createElement('div');
    el.className = 'menu';
    el.setAttribute('role', 'menu');
    const heading = document.createElement('div');
    heading.className = 'heading';
    heading.textContent = tile.dataset.name;
    const sep = document.createElement('div');
    sep.className = 'sep';
    el.append(
      heading,
      item('Open', icons.globe || '', () => (location.href = tile.href)),
      item('Open in its own window', icons['app-window'] || '', () => post({ do: 'open-app', id: tile.dataset.id })),
      sep,
      item('Remove from apps', icons.trash || '', () => post({ do: 'remove-app', id: tile.dataset.id }) && tile.remove(), true),
    );
    document.body.append(el);
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    el.style.left = `${Math.max(6, Math.min(e.clientX, innerWidth - w - 6))}px`;
    el.style.top = `${Math.max(6, Math.min(e.clientY, innerHeight - h - 6))}px`;
    tile.classList.add('menu-open');
    menu = { el, tile };
    el.querySelector('.item').focus();
  });

  document.addEventListener('pointerdown', (e) => {
    if (menu && !menu.el.contains(e.target)) closeMenu();
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') closeMenu();
  });
  addEventListener('blur', closeMenu);
})();
