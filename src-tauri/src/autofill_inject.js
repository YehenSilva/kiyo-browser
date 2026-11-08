(function () {
  if (window.__kiyo_autofill_injected) return;
  window.__kiyo_autofill_injected = true;

  let autofillData = { status: 'empty', host: '', entries: [] };
  let currentField = null;
  let hoverTimer = null;
  let closeTimer = null;
  let shadowRoot = null;
  let containerEl = null;
  let menuEl = null;
  let selectedIndex = 0;
  let isMouseOverMenu = false;

  const icons = {
    user: '<svg viewBox="0 0 24 24"><path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>',
    key: '<svg viewBox="0 0 24 24"><circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M16 7l3 3"/></svg>',
    lock: '<svg viewBox="0 0 24 24"><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></svg>',
    gear: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1"/></svg>'
  };

  function getInvoke() {
    return (
      (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) ||
      (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke)
    );
  }

  function isVisible(el) {
    if (!el) return false;
    const style = window.getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || style.opacity === '0') return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }

  function isAuthInput(el) {
    if (!el || el.tagName !== 'INPUT' || el.disabled || el.readOnly) return false;
    const type = (el.type || 'text').toLowerCase();
    if (type === 'password') return true;
    if (type === 'email') return true;
    const name = (el.name || '').toLowerCase();
    const id = (el.id || '').toLowerCase();
    const auto = (el.autocomplete || '').toLowerCase();
    if (auto.includes('username') || auto.includes('email') || auto.includes('current-password')) return true;
    if (name.includes('user') || name.includes('login') || name.includes('email') || id.includes('user') || id.includes('login') || id.includes('email')) {
      return !!document.querySelector('input[type="password"]');
    }
    return false;
  }

  function ensureShadow() {
    if (shadowRoot) return;
    const host = document.createElement('div');
    host.id = '__kiyo_autofill_host';
    host.style.cssText = 'all:initial!important;position:absolute!important;top:0!important;left:0!important;width:0!important;height:0!important;z-index:2147483647!important;';
    (document.body || document.documentElement).appendChild(host);

    shadowRoot = host.attachShadow({ mode: 'open' });
    const style = document.createElement('style');
    style.textContent = `
      * { box-sizing: border-box; margin: 0; padding: 0; }
      .kiyo-dropdown {
        position: absolute;
        z-index: 2147483647;
        background: #1c1b22;
        color: #efedf5;
        border: 1px solid #363442;
        border-radius: 9px;
        box-shadow: 0 10px 28px rgba(0,0,0,0.55), 0 2px 6px rgba(0,0,0,0.3);
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, system-ui, sans-serif;
        font-size: 13px;
        line-height: 1.35;
        user-select: none;
        overflow: hidden;
        min-width: 240px;
        max-width: 380px;
        animation: kiyoFadeIn 0.12s ease-out;
      }
      @keyframes kiyoFadeIn {
        from { opacity: 0; transform: translateY(-4px); }
        to { opacity: 1; transform: translateY(0); }
      }
      .item {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 8px 12px;
        cursor: pointer;
        transition: background 0.1s, color 0.1s;
        border-bottom: 1px solid rgba(255, 255, 255, 0.05);
      }
      .item:last-child {
        border-bottom: none;
      }
      .item:hover, .item.selected {
        background: #2d2a38;
        color: #ffffff;
      }
      .item svg {
        width: 15px;
        height: 15px;
        fill: none;
        stroke: #9b98aa;
        stroke-width: 1.8;
        stroke-linecap: round;
        stroke-linejoin: round;
        flex: none;
        transition: stroke 0.1s;
      }
      .item:hover svg, .item.selected svg {
        stroke: #f0a8a0;
      }
      .text-wrap {
        flex: 1;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 1px;
      }
      .primary {
        font-weight: 600;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .secondary {
        font-size: 11px;
        color: #8e8b9d;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        letter-spacing: 0.08em;
      }
      .manage-item {
        background: #16151a;
        color: #9b98aa;
        font-size: 11.5px;
        padding: 7px 12px;
        border-top: 1px solid #363442;
      }
      .manage-item svg {
        stroke: #6d6a7c;
      }
      .manage-item:hover, .manage-item.selected {
        background: #25232d;
        color: #efedf5;
      }
      .manage-item:hover svg, .manage-item.selected svg {
        stroke: #f0a8a0;
      }
    `;
    shadowRoot.appendChild(style);

    containerEl = document.createElement('div');
    containerEl.className = 'kiyo-dropdown';
    containerEl.style.display = 'none';

    containerEl.addEventListener('mouseenter', () => { isMouseOverMenu = true; clearTimeout(closeTimer); });
    containerEl.addEventListener('mouseleave', () => { isMouseOverMenu = false; scheduleClose(); });

    shadowRoot.appendChild(containerEl);
    menuEl = containerEl;
  }

  function setInputValue(input, val) {
    if (!input || val == null) return;
    try {
      const proto = window.HTMLInputElement.prototype;
      const descriptor = Object.getOwnPropertyDescriptor(proto, 'value');
      if (descriptor && descriptor.set) {
        descriptor.set.call(input, val);
      } else {
        input.value = val;
      }
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
    } catch (e) {
      input.value = val;
    }
  }

  function fillCredentials(entry) {
    if (!entry) return;
    hideDropdown();

    let pw = (currentField && currentField.type === 'password') ? currentField : null;
    const form = currentField ? currentField.closest('form') : null;
    const root = form || document;

    if (!pw) {
      pw = [...root.querySelectorAll('input[type="password"]')].find(isVisible);
    }

    const cand = [...root.querySelectorAll('input[type="text"], input[type="email"], input[type="tel"], input:not([type])')].filter(isVisible);

    let userField = null;
    if (currentField && currentField.type !== 'password' && isAuthInput(currentField)) {
      userField = currentField;
    } else if (pw) {
      userField = cand.filter(e => (e.compareDocumentPosition(pw) & 4) !== 0).pop();
    } else {
      userField = cand[0] || null;
    }

    if (userField && entry.user) {
      setInputValue(userField, entry.user);
    }
    if (pw && entry.pass) {
      setInputValue(pw, entry.pass);
      pw.focus();
    } else if (userField) {
      userField.focus();
    }
  }

  function showDropdownFor(field) {
    if (!field || !isVisible(field)) return;
    currentField = field;
    ensureShadow();

    // Notify backend
    const rect = field.getBoundingClientRect();
    const invoke = getInvoke();
    if (typeof invoke === 'function') {
      invoke('autofill_field', {
        bounds: {
          x: rect.left,
          y: rect.bottom,
          width: rect.width,
          height: rect.height
        }
      }).catch(() => {});
    }

    renderDropdown(field);
  }

  function renderDropdown(field) {
    if (!field || !menuEl) return;
    const rect = field.getBoundingClientRect();
    const items = [];

    if (autofillData.status === 'locked') {
      items.push({ action: 'unlock', primary: 'Unlock password vault', secondary: 'Master password required', icon: icons.lock });
      items.push({ action: 'manage', primary: 'Manage passwords…', icon: icons.gear });
    } else if (autofillData.status === 'ready' && autofillData.entries && autofillData.entries.length > 0) {
      autofillData.entries.forEach((e, idx) => {
        items.push({
          action: 'fill',
          entry: e,
          index: idx,
          primary: e.user || '(no username)',
          secondary: '••••••••',
          icon: icons.user
        });
      });
      items.push({ action: 'manage', primary: 'Manage passwords…', icon: icons.gear });
    } else {
      // If unlocked and no entries match, do not show an intrusive empty dropdown
      hideDropdown();
      return;
    }

    menuEl.innerHTML = '';
    items.forEach((it, idx) => {
      const row = document.createElement('div');
      const isManage = it.action === 'manage';
      row.className = 'item' + (isManage ? ' manage-item' : '') + (idx === selectedIndex ? ' selected' : '');
      row.innerHTML = it.icon +
        '<div class="text-wrap"><div class="primary">' + escapeHtml(it.primary) + '</div>' +
        (it.secondary ? '<div class="secondary">' + escapeHtml(it.secondary) + '</div>' : '') +
        '</div>';

      row.addEventListener('click', (ev) => {
        ev.preventDefault();
        ev.stopPropagation();
        handleItemClick(it);
      });
      row.addEventListener('mouseenter', () => {
        selectedIndex = idx;
        updateSelected();
      });
      menuEl.appendChild(row);
    });

    // Position menu below field (or flip above if near bottom edge)
    const scrollX = window.pageXOffset || document.documentElement.scrollLeft || 0;
    const scrollY = window.pageYOffset || document.documentElement.scrollTop || 0;
    const width = Math.max(rect.width, 240);
    let left = rect.left + scrollX;

    if (rect.left + width > window.innerWidth - 12) {
      left = Math.max(8, window.innerWidth - width - 12) + scrollX;
    }

    let top = rect.bottom + scrollY + 4;
    const estimatedHeight = items.length * 40 + 8;
    if (rect.bottom + estimatedHeight > window.innerHeight && rect.top - estimatedHeight > 0) {
      top = rect.top + scrollY - estimatedHeight - 4;
    }

    menuEl.style.left = left + 'px';
    menuEl.style.top = top + 'px';
    menuEl.style.width = width + 'px';
    menuEl.style.display = 'block';
  }

  function updateSelected() {
    if (!menuEl) return;
    const rows = menuEl.querySelectorAll('.item');
    rows.forEach((r, i) => r.classList.toggle('selected', i === selectedIndex));
  }

  function handleItemClick(it) {
    if (it.action === 'fill') {
      fillCredentials(it.entry);
    } else if (it.action === 'unlock') {
      hideDropdown();
      const invoke = getInvoke();
      if (typeof invoke === 'function') {
        invoke('autofill_select', { action: 'unlock' }).catch(() => {});
      }
    } else if (it.action === 'manage') {
      hideDropdown();
      const invoke = getInvoke();
      if (typeof invoke === 'function') {
        invoke('autofill_select', { action: 'manage' }).catch(() => {});
      }
    }
  }

  function hideDropdown() {
    clearTimeout(hoverTimer);
    clearTimeout(closeTimer);
    if (menuEl) {
      menuEl.style.display = 'none';
    }
    const invoke = getInvoke();
    if (typeof invoke === 'function') {
      invoke('autofill_dismiss').catch(() => {});
    }
  }

  function scheduleClose() {
    clearTimeout(closeTimer);
    closeTimer = setTimeout(() => {
      if (!isMouseOverMenu && document.activeElement !== currentField) {
        hideDropdown();
      }
    }, 200);
  }

  function escapeHtml(s) {
    return String(s || '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  }

  // Global API invoked from UI webview via tab_eval
  window.__kiyo_set_autofill = function (data) {
    autofillData = data || { status: 'empty', host: '', entries: [] };
    const isHovered = currentField && currentField.matches && currentField.matches(':hover');
    const isFocused = document.activeElement === currentField;
    if (currentField && isVisible(currentField) && (isFocused || isHovered || (menuEl && menuEl.style.display === 'block'))) {
      renderDropdown(currentField);
    }
  };

  // Event Listeners on the document (optimized - trigger on focus/click, avoiding continuous mouseover overhead)
  document.addEventListener('focusin', function (e) {
    const target = e.target;
    if (isAuthInput(target)) {
      clearTimeout(hoverTimer);
      clearTimeout(closeTimer);
      showDropdownFor(target);
    }
  }, true);

  document.addEventListener('focusout', function (e) {
    const target = e.target;
    if (isAuthInput(target)) {
      scheduleClose();
    }
  }, true);

  document.addEventListener('pointerdown', function (e) {
    if (menuEl && menuEl.style.display === 'block') {
      const path = e.composedPath ? e.composedPath() : [];
      if (!path.includes(currentField) && !path.includes(containerEl)) {
        hideDropdown();
      }
    }
  }, true);

  window.addEventListener('scroll', function () {
    if (menuEl && menuEl.style.display === 'block') {
      if (currentField && isVisible(currentField)) {
        const rect = currentField.getBoundingClientRect();
        const scrollX = window.pageXOffset || document.documentElement.scrollLeft || 0;
        const scrollY = window.pageYOffset || document.documentElement.scrollTop || 0;
        menuEl.style.left = (rect.left + scrollX) + 'px';
        menuEl.style.top = (rect.bottom + scrollY + 4) + 'px';
      } else {
        hideDropdown();
      }
    }
  }, true);

  document.addEventListener('keydown', function (e) {
    if (!menuEl || menuEl.style.display !== 'block') return;
    const items = menuEl.querySelectorAll('.item');
    if (!items.length) return;

    if (e.key === 'ArrowDown') {
      e.preventDefault();
      selectedIndex = (selectedIndex + 1) % items.length;
      updateSelected();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      selectedIndex = (selectedIndex - 1 + items.length) % items.length;
      updateSelected();
    } else if (e.key === 'Enter') {
      const cur = items[selectedIndex];
      if (cur) {
        e.preventDefault();
        cur.click();
      }
    } else if (e.key === 'Escape') {
      e.preventDefault();
      hideDropdown();
    }
  }, true);

  // Mouse back / forward side buttons (button 3 = back, 4 = forward)
  function handleSideMouseNav(e) {
    if (e.button === 3) {
      e.preventDefault();
      window.history.back();
    } else if (e.button === 4) {
      e.preventDefault();
      window.history.forward();
    }
  }
  window.addEventListener('mouseup', handleSideMouseNav, true);
  window.addEventListener('auxclick', handleSideMouseNav, true);

  // Focus & Click Notification for Split View & Menu Dismissal (closes context menu instantly on web content click)
  function notifyTabMouseDown() {
    try {
      const iv = getInvoke();
      if (iv && window.__KIYO_TAB_ID) {
        iv('tab_focused', { id: window.__KIYO_TAB_ID }).catch(function () {});
      }
    } catch (_) {}
  }
  window.addEventListener('focus', notifyTabMouseDown, true);
  window.addEventListener('mousedown', notifyTabMouseDown, true);
  window.addEventListener('pointerdown', notifyTabMouseDown, true);

  // Ctrl+Click, Cmd+Click, or Alt+Click on any link opens Glance Quick Look sheet
  document.addEventListener('click', function (e) {
    if (!e.ctrlKey && !e.metaKey && !e.altKey) return;
    const link = e.target.closest('a');
    if (link && link.href) {
      const href = link.href;
      if (href.startsWith('http://') || href.startsWith('https://')) {
        e.preventDefault();
        e.stopPropagation();
        try {
          const iv = getInvoke();
          if (iv) {
            iv('request_glance', { url: href }).catch(function () {});
          }
        } catch (_) {}
      }
    }
  }, true);

  // DevTools Toggle Shortcut (F12 or Ctrl+Shift+I) for any tab
  window.addEventListener('keydown', function (e) {
    if (e.key === 'F12' || (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === 'i')) {
      e.preventDefault();
      try {
        const iv = getInvoke();
        if (iv && window.__KIYO_TAB_ID) {
          iv('tab_devtools', { id: window.__KIYO_TAB_ID }).catch(function () {});
        }
      } catch (_) {}
    }
  }, true);

  // Picture in Picture Toggle (Alt+P or via shell command)
  window.__KIYO_TOGGLE_PIP = async function () {
    try {
      if (document.pictureInPictureElement) {
        await document.exitPictureInPicture();
        return false;
      }
      const videos = Array.from(document.querySelectorAll('video')).filter(function (v) {
        return v.readyState >= 1 || !v.paused;
      });
      const target = videos.length ? videos[0] : document.querySelector('video');
      if (target) {
        await target.requestPictureInPicture();
        return true;
      }
      return false;
    } catch (err) {
      console.warn('PiP notice:', err);
      return false;
    }
  };
  window.addEventListener('keydown', function (e) {
    if (e.altKey && e.key.toLowerCase() === 'p') {
      e.preventDefault();
      window.__KIYO_TOGGLE_PIP();
    }
  }, true);

  // Ctrl+Click any link to Glance peek
  document.addEventListener('click', function (e) {
    if (e.ctrlKey || e.metaKey) {
      const a = e.target.closest('a');
      if (a && a.href && /^https?:\/\//i.test(a.href)) {
        e.preventDefault();
        e.stopPropagation();
        const invoke = getInvoke();
        if (typeof invoke === 'function') {
          invoke('request_glance', { url: a.href }).catch(() => {});
        }
      }
    }
  }, true);

  // Ctrl+G shortcut to Glance peek current page
  window.addEventListener('keydown', function (e) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'g' && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      const invoke = getInvoke();
      if (typeof invoke === 'function') {
        invoke('request_glance', { url: window.location.href }).catch(() => {});
      }
    }
  }, true);

  // Command Palette Toggle (Ctrl+K or Cmd+K)
  window.addEventListener('keydown', function (e) {
    if ((e.ctrlKey || e.metaKey) && (e.key === 'k' || e.key === 'K') && !e.shiftKey && !e.altKey) {
      e.preventDefault();
      e.stopPropagation();
      try {
        const iv = getInvoke();
        if (iv) {
          iv('toggle_command_palette').catch(function () {});
        }
      } catch (_) {}
    }
  }, true);

  // Native OAuth & Login Popup Synchronization
  (function initOAuthSync() {
    try {
      const origOpen = window.open;
      if (typeof origOpen === 'function') {
        window.open = function (...args) {
          const win = origOpen.apply(this, args);
          const urlStr = String(args[0] || '');
          const isAuthTarget = /accounts\.google\.com|github\.com\/login|appleid\.apple\.com|auth0|login|signin|oauth/i.test(urlStr);
          const isAuthPage = /login|signin|signup|auth/i.test(window.location.pathname + window.location.search);

          if (win && (isAuthTarget || isAuthPage)) {
            let attempts = 0;
            const timer = setInterval(function () {
              attempts++;
              try {
                if (!win || win.closed || attempts > 600) {
                  clearInterval(timer);
                  if (win && win.closed) {
                    // Popup completed & closed. Give WebView2 network stack 400ms to persist session cookies.
                    setTimeout(function () {
                      if (/login|signin|signup/i.test(window.location.pathname)) {
                        window.location.reload();
                      }
                    }, 400);
                  }
                }
              } catch (_) {
                clearInterval(timer);
              }
            }, 350);
          }
          return win;
        };
      }

      // If opener receives message event with token or auth success
      window.addEventListener('message', function (ev) {
        try {
          const data = ev.data;
          if (data && (data.type === 'oauth-success' || data.type === 'authorized' || data.credential || data.code)) {
            setTimeout(function () {
              if (/login|signin|signup/i.test(window.location.pathname)) {
                window.location.reload();
              }
            }, 400);
          }
        } catch (_) {}
      }, false);
    } catch (_) {}
  })();

  // Page Zoom support
  window.__KIYO_SET_ZOOM = function (factor) {
    try {
      document.documentElement.style.zoom = factor;
    } catch (_) {}
  };

  // Ctrl + Mouse Wheel Zoom
  function handleCtrlWheel(e) {
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      e.stopImmediatePropagation();
      const delta = e.deltaY;
      const curZoom = parseFloat(document.documentElement.style.zoom) || 1.0;
      const step = delta < 0 ? 0.08 : -0.08;
      const nextZoom = Math.max(0.3, Math.min(3.0, +(curZoom + step).toFixed(2)));
      document.documentElement.style.zoom = nextZoom;
      try {
        const inv = getInvoke();
        if (typeof inv === 'function' && window.__KIYO_TAB_ID) {
          inv('tab_zoom_changed', { id: window.__KIYO_TAB_ID, zoom: nextZoom }).catch(function () {});
        }
      } catch (_) {}
    }
  }
  window.addEventListener('wheel', handleCtrlWheel, { passive: false, capture: true });
  document.addEventListener('wheel', handleCtrlWheel, { passive: false, capture: true });

  // Per-site permissions manager (Camera, Microphone, Location)
  window.__KIYO_PERMISSIONS = { camera: 'ask', mic: 'ask', location: 'ask' };
  window.__kiyo_set_permissions = function (p) {
    if (p) Object.assign(window.__KIYO_PERMISSIONS, p);
  };

  if (navigator.mediaDevices && navigator.mediaDevices.getUserMedia) {
    const origGUM = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
    navigator.mediaDevices.getUserMedia = function (constraints) {
      const wantVideo = constraints && (constraints.video === true || typeof constraints.video === 'object');
      const wantAudio = constraints && (constraints.audio === true || typeof constraints.audio === 'object');
      if (wantVideo && window.__KIYO_PERMISSIONS.camera === 'block') {
        return Promise.reject(new DOMException('Camera permission blocked by Kiyo settings', 'NotAllowedError'));
      }
      if (wantAudio && window.__KIYO_PERMISSIONS.mic === 'block') {
        return Promise.reject(new DOMException('Microphone permission blocked by Kiyo settings', 'NotAllowedError'));
      }
      return origGUM(constraints);
    };
  }

  if (navigator.geolocation) {
    const origGetPos = navigator.geolocation.getCurrentPosition.bind(navigator.geolocation);
    navigator.geolocation.getCurrentPosition = function (success, error, options) {
      if (window.__KIYO_PERMISSIONS.location === 'block') {
        if (typeof error === 'function') {
          error({ code: 1, message: 'Location permission blocked by Kiyo settings', PERMISSION_DENIED: 1 });
        }
        return;
      }
      return origGetPos(success, error, options);
    };
    const origWatchPos = navigator.geolocation.watchPosition.bind(navigator.geolocation);
    navigator.geolocation.watchPosition = function (success, error, options) {
      if (window.__KIYO_PERMISSIONS.location === 'block') {
        if (typeof error === 'function') {
          error({ code: 1, message: 'Location permission blocked by Kiyo settings', PERMISSION_DENIED: 1 });
        }
        return -1;
      }
      return origWatchPos(success, error, options);
    };
  }

  // Cosmetic Ad & Promo Blocker (Pure CSS - never tampers with native fetch/XHR, so Google Auth & Identity Services run cleanly)
  (function initBlocker() {
    try {
      const style = document.createElement('style');
      style.textContent = `
        .adsbygoogle, [class*="ad-banner"], [id*="google_ads"], [id*="ad-slot"], [class*="ad_box"], iframe[src*="doubleclick"] { display: none !important; }
        #taw .gws-promo-callout, .gws-promo-callout {
          display: none !important;
          visibility: hidden !important;
        }
      `;
      (document.head || document.documentElement).appendChild(style);
    } catch (_) {}

    function purgeChromeNags() {
      if (!/(^|\.)google\./i.test(window.location.hostname)) return;
      const promo = document.querySelector('#taw .gws-promo-callout, .gws-promo-callout');
      if (promo) promo.remove();
    }

    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', purgeChromeNags);
    } else {
      purgeChromeNags();
    }
    setTimeout(purgeChromeNags, 400);
    setTimeout(purgeChromeNags, 1200);
  })();
})();
