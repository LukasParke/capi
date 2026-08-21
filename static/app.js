/* capi — front-end behavior. No frameworks beyond htmx; no inline JS anywhere.
 *
 * Modules: theme, toasts, SSE feed + panel refresh, WebSocket OOB fallback,
 * long-press repeat, keyboard shortcuts, busy states, panel error/retry,
 * dev-console API shim (renders DevActionResult-shaped DOM), MQTT keep-password.
 */
(function () {
  'use strict';

  var doc = document;
  var root = doc.documentElement;
  var hasHtmx = typeof window.htmx !== 'undefined';

  /* ---- tiny helpers ------------------------------------------------------ */

  function $(sel, scope) { return (scope || doc).querySelector(sel); }
  function $all(sel, scope) { return Array.prototype.slice.call((scope || doc).querySelectorAll(sel)); }

  function trigger(target, name) {
    if (!hasHtmx || !target) return;
    window.htmx.trigger(target, name);
  }

  function triggerBody(name) {
    if (hasHtmx) { window.htmx.trigger(doc.body, name); return; }
    doc.body.dispatchEvent(new CustomEvent(name, { bubbles: true }));
  }

  var LA_NAMES = {
    0: 'TV', 1: 'Recording device 1', 2: 'Recording device 2', 3: 'Tuner 1',
    4: 'Playback device 1', 5: 'Audio system', 6: 'Tuner 2', 7: 'Tuner 3',
    8: 'Playback device 2', 9: 'Recording device 3', 10: 'Tuner 4',
    11: 'Playback device 3', 12: 'Reserved', 13: 'Reserved', 14: 'Free use',
    15: 'Broadcast'
  };

  function laName(addr) {
    var n = Number(addr);
    return LA_NAMES[n] || ('LA ' + addr);
  }

  var KEYCODE_NAMES = {
    0: 'select', 1: 'up', 2: 'down', 3: 'left', 4: 'right', 9: 'root menu',
    11: 'contents menu', 13: 'setup menu', 46: 'play', 49: 'pause', 48: 'stop',
    47: 'skip forward', 44: 'rewind', 45: 'eject', 70: 'f1 blue'
  };
  // CEC UI command codes under 0x40 block
  KEYCODE_NAMES[68] = 'play'; KEYCODE_NAMES[69] = 'pause'; KEYCODE_NAMES[70] = 'stop';
  KEYCODE_NAMES[71] = 'fast forward'; KEYCODE_NAMES[72] = 'rewind';
  KEYCODE_NAMES[73] = 'record'; KEYCODE_NAMES[83] = 'number 0';
  KEYCODE_NAMES[84] = 'number 1'; KEYCODE_NAMES[85] = 'number 2';
  KEYCODE_NAMES[86] = 'number 3'; KEYCODE_NAMES[87] = 'number 4';
  KEYCODE_NAMES[88] = 'number 5'; KEYCODE_NAMES[89] = 'number 6';
  KEYCODE_NAMES[90] = 'number 7'; KEYCODE_NAMES[91] = 'number 8';
  KEYCODE_NAMES[92] = 'number 9';

  function keyName(code) {
    return KEYCODE_NAMES[Number(code)] || ('0x' + Number(code).toString(16));
  }

  /* ---- theme -------------------------------------------------------------- */

  var THEME_KEY = 'capi-theme';

  function applyStoredTheme() {
    try {
      var stored = localStorage.getItem(THEME_KEY);
      if (stored === 'light' || stored === 'dark') root.dataset.theme = stored;
      else delete root.dataset.theme; // unset -> CSS prefers-color-scheme decides
    } catch (e) { /* storage unavailable */ }
  }

  function effectiveTheme() {
    var t = root.dataset.theme;
    if (t === 'light' || t === 'dark') return t;
    return window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches
      ? 'light' : 'dark';
  }

  function toggleTheme() {
    var next = effectiveTheme() === 'dark' ? 'light' : 'dark';
    root.dataset.theme = next;
    try { localStorage.setItem(THEME_KEY, next); } catch (e) { /* ignore */ }
  }

  /* ---- toasts ---------------------------------------------------------------- */

  var TOAST_MS = 4000;

  function toast(level, message) {
    var stack = $('#toast-stack');
    if (!stack || !message) return;
    var el = doc.createElement('div');
    el.className = 'toast toast-' + (level === 'ok' || level === 'err' ? level : 'info');
    el.setAttribute('role', 'status');

    var msg = doc.createElement('span');
    msg.className = 'toast-msg';
    msg.textContent = String(message);

    var close = doc.createElement('button');
    close.className = 'toast-close';
    close.type = 'button';
    close.setAttribute('aria-label', 'Dismiss');
    close.textContent = '\u00d7';

    el.appendChild(msg);
    el.appendChild(close);
    stack.appendChild(el);

    var timer = window.setTimeout(dismiss, TOAST_MS);
    function dismiss() {
      window.clearTimeout(timer);
      if (el.parentNode) el.parentNode.removeChild(el);
    }
    el.addEventListener('click', dismiss);
  }

  function toastFromHeader(xhr) {
    try {
      var v = xhr.getResponseHeader('x-capi-toast');
      if (!v) return false;
      var i = v.indexOf(': ');
      if (i === -1) { toast('info', v); return true; }
      var level = v.slice(0, i).trim() === 'ok' ? 'ok' : 'err';
      toast(level, v.slice(i + 2).trim());
      return true;
    } catch (e) { return false; }
  }

  function toastFromJsonEnvelope(text) {
    try {
      var env = JSON.parse(text);
      if (!env || typeof env !== 'object' || !('status' in env)) return false;
      toast(env.status === 'success' ? 'ok' : 'err', env.message || (env.status === 'success' ? 'Done' : 'Request failed'));
      return true;
    } catch (e) { return false; }
  }

  /* ---- activity feed ------------------------------------------------------------ */

  var FEED_CAP = 100;

  function summarize(ev) {
    var d = ev.data || {};
    switch (ev.type) {
      case 'power_change': return laName(d.address) + ' \u2192 ' + (d.status || 'unknown');
      case 'source_activated': return 'active source \u2192 ' + laName(d.address) +
        (d.activated === false ? ' (lost)' : '');
      case 'key_press': return 'key ' + keyName(d.keycode) +
        (d.duration ? ' (' + d.duration + ' ms)' : '');
      case 'command': {
        var op = d.opcode != null ? (typeof d.opcode === 'string' ? d.opcode : '0x' + Number(d.opcode).toString(16)) : '?';
        return 'cmd ' + laName(d.initiator) + ' \u2192 ' + laName(d.destination) + ' op ' + op;
      }
      case 'alert': return 'alert ' + d.alert + ' param ' + d.param;
      case 'devices_changed': {
        var addrs = Array.isArray(d.logical_addresses) ? d.logical_addresses.length : null;
        return 'devices changed' + (d.reason ? ' (' + d.reason + ')' : '') +
          (addrs != null ? ' \u2014 ' + addrs + ' address' + (addrs === 1 ? '' : 'es') + ' seen' : '');
      }
      case 'configuration_changed': return 'libcec configuration changed';
      case 'adapter_state': return 'adapter ' + (d.state || 'state changed');
      default: return ev.type;
    }
  }

  function buildFeedLine(ev) {
    var line = doc.createElement('div');
    line.className = 'feed-line entering';
    line.setAttribute('data-kind', ev.type);

    var time = doc.createElement('time');
    time.className = 'mono feed-time';
    var ts = ev.timestamp ? new Date(ev.timestamp) : new Date();
    time.textContent = isNaN(ts.getTime())
      ? ''
      : ts.toLocaleTimeString([], { hour12: false });

    var kind = doc.createElement('span');
    kind.className = 'chip chip-kind';
    kind.setAttribute('data-kind', ev.type);
    kind.textContent = ev.type;

    var summary = doc.createElement('span');
    summary.className = 'feed-summary';
    summary.textContent = summarize(ev);

    line.appendChild(time);
    line.appendChild(kind);
    line.appendChild(summary);
    return line;
  }

  function appendFeed(ev) {
    $all('#activity-feed, #dev-feed').forEach(function (feed) {
      var empty = $('.feed-empty', feed);
      if (empty) empty.remove();
      feed.insertBefore(buildFeedLine(ev), feed.firstChild);
      while (feed.children.length > FEED_CAP) feed.removeChild(feed.lastChild);
    });
  }

  function setConnState(state) {
    $all('[data-conn-state]').forEach(function (pill) {
      pill.classList.toggle('is-ok', state === 'live');
      pill.classList.toggle('is-err', state === 'down');
      var label = $('[data-conn-label]', pill);
      if (label) label.textContent = state === 'live' ? 'live' : state;
    });
  }

  /* ---- event -> refresh mapping --------------------------------------------------- */

  var EVENT_TYPES = ['power_change', 'source_activated', 'key_press', 'command',
    'alert', 'devices_changed', 'configuration_changed', 'adapter_state'];

  function handleEvent(ev) {
    if (!ev || !ev.type) return;
    appendFeed(ev);
    switch (ev.type) {
      case 'devices_changed':
        triggerBody('refresh-devices');
        triggerBody('refresh-topology');
        triggerBody('refresh-mqtt');
        break;
      case 'source_activated':
        triggerBody('refresh-source');
        triggerBody('refresh-devices');
        break;
      case 'power_change':
        triggerBody('refresh-devices');
        // observed power status lands a beat later via Report Power Status frames
        window.setTimeout(function () { triggerBody('refresh-devices'); }, 900);
        break;
      case 'adapter_state': {
        triggerBody('refresh-banner');
        triggerBody('refresh-volume');
        var state = (ev.data && ev.data.state) || '';
        updateAdapterPill(state === 'connected');
        if (state === 'disconnected') toast('err', 'CEC adapter disconnected');
        if (state === 'connected') toast('ok', 'CEC adapter connected');
        break;
      }
      case 'configuration_changed':
        triggerBody('refresh-banner');
        break;
    }
  }

  function updateAdapterPill(ready) {
    $all('[data-adapter-pill]').forEach(function (pill) {
      pill.classList.toggle('pill-ok', !!ready);
      pill.classList.toggle('pill-err', !ready);
      var label = $('[data-adapter-label]', pill) || pill.lastElementChild;
      if (label) label.textContent = ready ? 'Adapter open' : 'Adapter closed';
    });
  }

  /* ---- streams: SSE with WebSocket fallback ------------------------------------------ */

  var es = null;
  var esFailures = 0;
  var ws = null;
  var wsRetryTimer = null;

  function connectSSE() {
    if (!window.EventSource) { scheduleWsFallback(); return; }
    try { es = new EventSource('/api/events'); } catch (e) { scheduleWsFallback(); return; }

    es.onopen = function () {
      esFailures = 0;
      setConnState('live');
    };
    es.onerror = function () {
      esFailures += 1;
      setConnState('reconnecting');
      if (esFailures >= 2) {
        es.close();
        es = null;
        openWS();
      }
    };
    EVENT_TYPES.forEach(function (t) {
      es.addEventListener(t, function (e) {
        try { handleEvent(JSON.parse(e.data)); } catch (err) { /* malformed event */ }
      });
    });
    // fallback for unnamed events
    es.onmessage = function (e) {
      try { handleEvent(JSON.parse(e.data)); } catch (err) { /* malformed event */ }
    };
  }

  function scheduleWsFallback() {
    if (ws || wsRetryTimer) return;
    wsRetryTimer = window.setTimeout(function () { wsRetryTimer = null; openWS(); }, 1000);
  }

  function openWS() {
    if (ws) return;
    var proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    try { ws = new WebSocket(proto + '//' + location.host + '/api/events/ws'); } catch (e) { wsRetry(); return; }

    ws.onopen = function () { setConnState('live'); };
    ws.onmessage = function (e) {
      if (typeof e.data === 'string' && e.data.indexOf('hx-swap-oob') !== -1) applyOob(e.data);
    };
    ws.onclose = function () { ws = null; wsRetry(); };
    ws.onerror = function () { try { ws.close(); } catch (err) { /* already closed */ } };

    function wsRetry() {
      setConnState('reconnecting');
      if (!wsRetryTimer) {
        wsRetryTimer = window.setTimeout(function () {
          wsRetryTimer = null;
          esFailures = 0;
          connectSSE(); // give SSE another chance after a cooldown
        }, 5000);
      }
    }
  }

  /** Apply server-sent OOB fragments (htmx 2.x style, no hx-ws extension needed). */
  function applyOob(html) {
    if (!hasHtmx) return;
    var tpl = doc.createElement('template');
    tpl.innerHTML = html;
    var oobs = $all('[hx-swap-oob]', tpl.content);
    if (oobs.length === 0) return;
    oobs.forEach(function (el) {
      var spec = el.getAttribute('hx-swap-oob') || 'true';
      var strategy = 'outerHTML';
      var selector = '#' + el.id;
      if (spec !== 'true') {
        var idx = spec.indexOf(':');
        strategy = idx === -1 ? spec : spec.slice(0, idx);
        if (idx !== -1) selector = spec.slice(idx + 1);
      }
      var target = strategy === 'true' ? doc.getElementById(el.id) : $(selector);
      if (!target) return;
      try {
        if (strategy === 'innerHTML') {
          while (target.firstChild) target.removeChild(target.firstChild);
          while (el.firstChild) target.appendChild(el.firstChild);
        } else if (strategy === 'beforeend') {
          target.appendChild(el);
        } else { // outerHTML / true
          target.parentNode.replaceChild(el, target);
        }
      } catch (err) { /* skip malformed oob */ }
    });
    window.htmx.process(doc.body);
  }

  /* ---- long-press repeat ----------------------------------------------------------------- */

  var REPEAT_DELAY = 350;
  var MIN_REPEATS = 2;

  function fireSyntheticClick(el) {
    var ev = new MouseEvent('click', { bubbles: true, cancelable: true });
    ev.__capiSynthetic = true;
    el._capiLastFire = Date.now();
    el.dispatchEvent(ev);
  }

  function setupLongPress() {
    doc.addEventListener('pointerdown', function (e) {
      var el = e.target.closest ? e.target.closest('[data-repeat]') : null;
      if (!el || el.disabled) return;
      e.preventDefault(); // suppress the native click so we don't double-fire

      var fires = 1;
      fireSyntheticClick(el);

      var timer = window.setInterval(function () {
        fires += 1;
        fireSyntheticClick(el);
      }, REPEAT_DELAY);

      var held = true;
      el._capiHoldUntil = Date.now() + 700;

      function stop(extraFire) {
        if (!held) return;
        held = false;
        window.clearInterval(timer);
        doc.removeEventListener('pointerup', stopH);
        doc.removeEventListener('pointercancel', stopH);
        el.removeEventListener('pointerleave', stopH);
        // guarantee at least MIN_REPEATS total presses per the interaction spec
        if (extraFire && fires < MIN_REPEATS) fireSyntheticClick(el);
      }
      function stopH() { stop(true); }
      doc.addEventListener('pointerup', stopH);
      doc.addEventListener('pointercancel', stopH);
      el.addEventListener('pointerleave', stopH);
    });

    // swallow trusted clicks that land right after a hold on a repeat button
    doc.addEventListener('click', function (e) {
      var el = e.target.closest && e.target.closest('[data-repeat]');
      if (!el || e.__capiSynthetic) return;
      if (el._capiHoldUntil && Date.now() < el._capiHoldUntil) {
        e.preventDefault();
        e.stopPropagation();
      }
    }, true);
  }

  /* ---- busy states ------------------------------------------------------------------------- */

  function setupBusy() {
    if (!hasHtmx) return;
    doc.body.addEventListener('htmx:beforeRequest', function (e) {
      var el = e.detail.elt;
      if (!el || !el.classList) return;
      el.classList.add('busy');
      if (el.hasAttribute && el.hasAttribute('data-busy-disable')) el.disabled = true;
    });
    function done(e) {
      var el = e.detail.elt;
      if (!el || !el.classList) return;
      el.classList.remove('busy');
      if (el.hasAttribute && el.hasAttribute('data-busy-disable')) el.disabled = false;
    }
    doc.body.addEventListener('htmx:afterRequest', done);
  }

  /* ---- request/response global handlers -------------------------------------------------------- */

  function setupHtmxGlobal() {
    if (!hasHtmx) return;

    // inject remote target address into nav_key requests from the remote card
    doc.body.addEventListener('htmx:configRequest', function (e) {
      var elt = e.detail.elt;
      var card = elt && elt.closest && elt.closest('.card');
      var sel = card && $('[data-nav-target]', card);
      if ((elt.matches && elt.matches('[data-nav-group], [data-nav-group] *')) && sel && sel.value !== '') {
        e.detail.parameters.addr = sel.value;
      }
      var form = elt && elt.closest && elt.closest('[data-mqtt-form]');
      if (form && form.getAttribute('data-pass-set') === 'true') {
        var pass = form.querySelector('[name="pass"]');
        if (pass && pass.value === '') e.detail.parameters.pass = '***';
      }
    });

    // reroute not-yet-implemented UI dev actions to the JSON dev API
    doc.body.addEventListener('htmx:beforeRequest', function (e) {
      var path = e.detail.path || '';
      if (path.indexOf('/ui/dev/action/') !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      devActionShim(path, e.detail.parameters || {}, e.detail.elt);
    }, true);

    doc.body.addEventListener('htmx:afterRequest', function (e) {
      var xhr = e.detail.xhr;
      if (!xhr) return;
      if (toastFromHeader(xhr)) return;
      var ct = (xhr.getResponseHeader('content-type') || '');
        var isAction = e.detail.elt && e.detail.elt.getAttribute &&
          ((e.detail.elt.getAttribute('hx-post') || '').indexOf('/ui/action/') === 0);
        if (ct.indexOf('application/json') !== -1 && isAction) {
          try { toastFromJsonEnvelope(xhr.responseText); } catch (err) { /* non-json body */ }
        }
      autoscrollLogs();
    });

    doc.body.addEventListener('htmx:responseError', function (e) {
      var xhr = e.detail.xhr;
      if (xhr && xhr.status === 401) { location.assign('/login'); return; }
      if (!toastFromHeader(xhr)) {
        toast('err', 'Request failed' + (xhr ? ' (HTTP ' + xhr.status + ')' : ''));
      }
      showSlotError(e.detail.elt);
    });

    doc.body.addEventListener('htmx:sendError', function () {
      toast('err', 'Network error \u2014 is the service still running?');
      setConnState('down');
    });

    doc.body.addEventListener('htmx:afterSwap', autoscrollLogs);
  }

  /* ---- panel slots: error + retry ------------------------------------------------------------------ */

  function showSlotError(slot) {
    if (!slot || !slot.classList || !slot.classList.contains('panel-slot')) return;
    var url = slot.getAttribute('hx-get');
    if (!url) return;
    slot.innerHTML = '';

    var box = doc.createElement('div');
    box.className = 'panel-error';

    var text = doc.createElement('span');
    text.textContent = 'Panel failed to load.';

    var btn = doc.createElement('button');
    btn.className = 'btn btn-small';
    btn.type = 'button';
    btn.setAttribute('data-slot-retry', '');
    btn.textContent = 'Retry';

    box.appendChild(text);
    box.appendChild(btn);
    slot.appendChild(box);
  }

  function retrySlot(slot) {
    if (!hasHtmx || !slot) return;
    var url = slot.getAttribute('hx-get');
    if (!url) return;
    slot.classList.add('busy');
    window.htmx.ajax('GET', url, { target: slot, swap: 'innerHTML' })
      .then(function () { slot.classList.remove('busy'); });
  }

  /* ---- logs autoscroll ------------------------------------------------------------------------------- */

  function nearestLogView(el) {
    while (el) {
      if (el.hasAttribute && el.hasAttribute('data-log-view')) return el;
      el = el.parentElement;
    }
    return null;
  }

  function autoscrollLogs(e) {
    var lv = nearestLogView(e && e.target ? e.target : doc.body) || $('.log-view[data-log-view]');
    if (!lv) return;
    if (lv._stick === false) return;
    lv.scrollTop = lv.scrollHeight;
  }

  function watchLogStickiness() {
    doc.addEventListener('scroll', function (e) {
      var lv = nearestLogView(e.target);
      if (!lv) return;
      lv._stick = lv.scrollHeight - lv.scrollTop - lv.clientHeight < 24;
    }, true);
  }

  /* ---- keyboard shortcuts ---------------------------------------------------------------------------- */

  var SHORTCUTS = {
    ArrowUp: 'nav_up', ArrowDown: 'nav_down', ArrowLeft: 'nav_left', ArrowRight: 'nav_right',
    Enter: 'select', Escape: 'back', Backspace: 'back',
    '+': 'volume_up', '=': 'volume_up', '-': 'volume_down', '_': 'volume_down',
    m: 'volume_mute', M: 'volume_mute', h: 'home', H: 'home'
  };

  function sendShortcut(key) {
    var values = { key: key };
    if (key === 'volume_up') return postForm('/ui/action/volume_up', {});
    if (key === 'volume_down') return postForm('/ui/action/volume_down', {});
    if (key === 'volume_mute') return postForm('/ui/action/volume_mute', {});
    postForm('/ui/action/nav_key', values);
  }

  function postForm(url, values) {
    var params = new URLSearchParams();
    Object.keys(values).forEach(function (k) { if (values[k] !== undefined) params.append(k, values[k]); });
    return fetch(url, {
      method: 'POST',
      headers: { 'Content-Type': 'application/x-www-form-urlencoded;charset=UTF-8' },
      body: params.toString(),
      credentials: 'same-origin'
    }).then(function (res) {
      if (res.status === 401) { location.assign('/login'); return null; }
      return res.text().then(function (bodyText) {
        var ct = res.headers.get('content-type') || '';
        if (ct.indexOf('application/json') !== -1) toastFromJsonEnvelope(bodyText);
        else toastFromHeader(res);
        return res.ok;
      });
    }).catch(function () { toast('err', 'Network error'); return false; });
  }

  function setupShortcuts() {
    doc.addEventListener('keydown', function (e) {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      var t = e.target;
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' ||
                t.isContentEditable)) return;
      var key = SHORTCUTS[e.key];
      if (!key) {
        if (/^[0-9]$/.test(e.key)) {
          e.preventDefault();
          sendShortcut('number_' + e.key);
        }
        return;
      }
      e.preventDefault();
      sendShortcut(key);
    });
  }

  /* ---- dev console shim: /ui/dev/action/* -> /api/dev/* JSON ------------------------------- */

  var DEV_TITLES = {
    probe: 'Probe', run_strategies: 'Strategy bench', send_key: 'Send key',
    send_opcode: 'Raw opcode', save_strategy: 'Save strategy', mode: 'Session mode'
  };

  function num(v, fallback) {
    var n = Number(v);
    return v !== undefined && v !== '' && !isNaN(n) ? n : fallback;
  }

  function devPayload(action, p) {
    switch (action) {
      case 'mode':
        return p.mode === 'reconnect'
          ? { reconnect: true }
          : { monitor_only: String(p.monitor_only) === '1' || String(p.monitor_only).toLowerCase() === 'true' };
      case 'probe':
        return { address: num(p.addr, 0), kind: p.kind || 'all', observe_ms: num(p.observe_ms, 600) };
      case 'run_strategies':
        return {
          action: p.action || '', target: num(p.target, null),
          observe_ms: num(p.observe_ms, null), all_strategies: p.all_strategies === '1'
        };
      case 'send_key':
        return {
          address: num(p.addr, 0), key: p.key || '',
          hold_ms: num(p.hold_ms, 0), repeat: num(p.repeat, 0)
        };
      case 'send_opcode':
        return { dest: num(p.dest, 0), opcode: p.opcode || '', params_hex: p.params_hex || '' };
      case 'save_strategy':
        return { vendor: p.vendor || '', action: p.action || '', strategy: p.strategy || '' };
      default:
        return p;
    }
  }

  function devActionShim(uiPath, params, elt) {
    var action = uiPath.replace('/ui/dev/action/', '').split(/[/?]/)[0];
    fetch('/api/dev/' + action, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(devPayload(action, params)),
      credentials: 'same-origin'
    }).then(function (res) {
      if (res.status === 401) { location.assign('/login'); return null; }
      return res.json().catch(function () { return { status: 'error', message: 'HTTP ' + res.status }; })
        .then(function (env) { return { ok: res.ok, env: env }; });
    }).then(function (out) {
      if (!out) return;
      renderDevResult(action, out.env);
      if (action === 'mode') syncModeUi(out.env);
    }).catch(function () {
      toast('err', 'Dev request failed \u2014 network error');
    });
  }

  function el(tag, cls, text) {
    var node = doc.createElement(tag);
    if (cls) node.className = cls;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function renderDevResult(action, env) {
    var host = $('#dev-result');
    var envOk = env && env.status === 'success';
    if (!host) {
      toast(envOk ? 'ok' : 'err', env.message || (envOk ? 'Done' : 'Failed'));
      return;
    }
    host.innerHTML = '';

    var data = (env && env.data) || {};
    var wrap = el('div', 'dev-result' + (envOk ? '' : ' is-fail'));
    wrap.setAttribute('data-result', '');

    var head = el('header', 'dev-result-head');
    head.appendChild(el('span', 'badge ' + (envOk ? 'badge-ok' : 'badge-err'), envOk ? 'OK' : 'Failed'));
    head.appendChild(el('strong', null, DEV_TITLES[action] || action));
    wrap.appendChild(head);

    var detail = env.message || '';
    if (detail) wrap.appendChild(el('p', 'dev-result-detail', detail));

    var strategies = data.strategies;
    if (Array.isArray(strategies) && strategies.length > 0) {
      wrap.appendChild(strategyTable(strategies));
    }
    var steps = data.steps;
    if (Array.isArray(steps) && steps.length > 0) {
      wrap.appendChild(stepTable(steps));
      var meta = [];
      if (data.total_replies !== undefined) meta.push(data.total_replies + ' replies');
      if (data.kind) meta.push('kind: ' + data.kind);
      if (meta.length) wrap.appendChild(el('p', 'dim small', meta.join(' \u00b7 ')));
    }

    var details = el('details', 'raw-json');
    var summary = el('summary', null, 'Raw JSON');
    var pre = el('pre');
    pre.appendChild(el('code', null, JSON.stringify(env, null, 2)));
    details.appendChild(summary);
    details.appendChild(pre);
    wrap.appendChild(details);

    host.appendChild(wrap);
  }

  function strategyTable(strategies) {
    var scroll = el('div', 'table-scroll');
    var table = el('table', 'table');
    var thead = el('thead');
    var hr = el('tr');
    ['Strategy', 'Status', 'Ack', 'Reply', 'Abort code', 'Elapsed', 'Error'].forEach(function (h) {
      hr.appendChild(el('th', null, h));
    });
    thead.appendChild(hr);
    var tbody = el('tbody');
    strategies.forEach(function (s) {
      var tr = el('tr');
      tr.appendChild(el('td', 'mono', s.strategy || ''));
      tr.appendChild(el('td', null, s.status != null ? String(s.status) : ''));
      var ackTd = el('td');
      ackTd.appendChild(el('span', 'badge ' + (s.acked ? 'badge-ok' : 'badge-dim'), s.acked ? 'yes' : 'no'));
      tr.appendChild(ackTd);
      tr.appendChild(el('td', 'mono', s.reply_name || ''));
      tr.appendChild(el('td', 'mono', s.abort_opcode ? String(s.abort_opcode) : '\u2013'));
      tr.appendChild(el('td', 'mono', s.elapsed_ms != null ? s.elapsed_ms + ' ms' : ''));
      var errTd = el('td', s.error ? 'text-err' : null, s.error || '');
      tr.appendChild(errTd);
      tbody.appendChild(tr);
    });
    table.appendChild(thead);
    table.appendChild(tbody);
    scroll.appendChild(table);
    return scroll;
  }

  function stepTable(steps) {
    var scroll = el('div', 'table-scroll');
    var table = el('table', 'table');
    var thead = el('thead');
    var hr = el('tr');
    ['Step', 'Opcode', 'Result', 'Elapsed', 'Replies', 'Error'].forEach(function (h) {
      hr.appendChild(el('th', null, h));
    });
    thead.appendChild(hr);
    var tbody = el('tbody');
    steps.forEach(function (s) {
      var tr = el('tr');
      tr.appendChild(el('td', null, s.name || ''));
      tr.appendChild(el('td', 'mono', s.opcode || ''));
      tr.appendChild(el('td', null, s.result || ''));
      tr.appendChild(el('td', 'mono', s.elapsed_ms != null ? s.elapsed_ms + ' ms' : ''));
      tr.appendChild(el('td', 'mono', Array.isArray(s.replies) ? String(s.replies.length) : ''));
      var errTd = el('td', s.error ? 'text-err' : null, s.error || '');
      tr.appendChild(errTd);
      tbody.appendChild(tr);
    });
    table.appendChild(thead);
    table.appendChild(tbody);
    scroll.appendChild(table);
    return scroll;
  }

  function syncModeUi(env) {
    var monitor = env && env.data && env.data.monitor_only;
    if (monitor === undefined || monitor === null) {
      if (env && env.message && env.message.indexOf('monitor') !== -1) {
        monitor = env.message.indexOf('monitor-only') !== -1;
      }
    }
    $all('[data-mode-radio]').forEach(function (radio) {
      if (monitor === undefined || monitor === null) return;
      radio.checked = (radio.value === '1') === !!monitor;
    });
    $all('[data-mode-pill]').forEach(function (pill) {
      var label = $('[data-mode-label]', pill) || pill.lastElementChild;
      if (monitor === undefined || monitor === null) return;
      pill.classList.toggle('pill-warn', !!monitor);
      pill.classList.toggle('pill-ok', !monitor);
      if (label) label.textContent = monitor ? 'Monitor-only' : 'Passive';
    });
  }

  /* ---- delegated clicks: mode radios, confirm buttons, slot retry --------------------------------- */

  function setupDelegatedActions() {
    doc.addEventListener('change', function (e) {
      var radio = e.target && e.target.matches && e.target.matches('[data-mode-radio]');
      if (!radio) return;
      var value = e.target.value;
      fetch('/api/dev/mode', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ monitor_only: value === '1' }),
        credentials: 'same-origin'
      }).then(function (res) {
        if (res.status === 401) { location.assign('/login'); return null; }
        return res.json();
      }).then(function (env) {
        toast(env && env.status === 'success' ? 'ok' : 'err',
          (env && env.message) || 'Mode change failed');
        if (hasHtmx) window.htmx.trigger(doc.body, 'refresh-banner');
      }).catch(function () {
        toast('err', 'Mode change failed \u2014 network error');
      });
    });

    doc.addEventListener('click', function (e) {
      var btn = e.target.closest ? e.target.closest('[data-confirm][data-endpoint]') : null;
      if (btn) {
        if (!window.confirm(btn.getAttribute('data-confirm'))) return;
        e.preventDefault();
        fetch(btn.getAttribute('data-endpoint'), {
          method: btn.getAttribute('data-method') || 'POST',
          credentials: 'same-origin'
        }).then(function (res) {
          if (res.status === 401) { location.assign('/login'); return null; }
          return res.text().then(function (bodyText) { return { text: bodyText, ok: res.ok }; });
        }).then(function (out) {
          if (!out) return;
          var parsed = null;
          try { parsed = JSON.parse(out.text); } catch (err) { /* non-json */ }
          var msg = parsed && parsed.message ? parsed.message : (out.ok ? 'Update check complete' : 'Update failed');
          toast(out.ok && (!parsed || parsed.status === 'success') ? 'ok' : 'err', msg);
          var note = $('[data-update-note]');
          if (note) { note.textContent = msg; note.hidden = false; }
        }).catch(function () { toast('err', 'Update check failed \u2014 network error'); });
        return;
      }

      var retry = e.target.closest ? e.target.closest('[data-slot-retry]') : null;
      if (retry) {
        var slot = retry.closest('.panel-slot');
        if (slot) retrySlot(slot);
      }
    });
  }

  /* ---- hydrations ------------------------------------------------------------------------------------ */

  function hydrateOpcodeDatalist() {
    var list = $('#opcode-names');
    if (!list || list.options.length > 0) return;
    fetch('/api/dev/opcodes', { credentials: 'same-origin' })
      .then(function (r) { return r.ok ? r.json() : null; })
      .then(function (env) {
        var data = env && env.data;
        if (!data) return;
        var entries = [];
        if (Array.isArray(data)) {
          data.forEach(function (item) {
            if (Array.isArray(item)) entries.push([item[0], item[1]]);
            else if (item && typeof item === 'object') entries.push([item.name || item.mnemonic, item.code != null ? item.code : item.value]);
          });
        } else if (typeof data === 'object') {
          Object.keys(data).forEach(function (name) { entries.push([name, data[name]]); });
        }
        entries.forEach(function (pair) {
          var code = Number(pair[1]);
          if (isNaN(code)) return;
          var opt = doc.createElement('option');
          opt.value = '0x' + code.toString(16).padStart(2, '0');
          opt.label = pair[0];
          list.appendChild(opt);
        });
      })
      .catch(function () { /* optional enhancement */ });
  }

  /* ---- nav highlight ------------------------------------------------------------------------------------ */

  function highlightNav() {
    var path = location.pathname;
    $all('.topnav a').forEach(function (a) {
      var href = a.getAttribute('href');
      var active = href === '/' ? path === '/' : path.indexOf(href) === 0;
      if (active) a.setAttribute('aria-current', 'page');
      else a.removeAttribute('aria-current');
    });
  }

  /* ---- boot ------------------------------------------------------------------------------------------------ */

  applyStoredTheme();

  var toggleBtn = $('[data-action="toggle-theme"]');
  if (toggleBtn) toggleBtn.addEventListener('click', toggleTheme);

  highlightNav();
  setupLongPress();
  setupDelegatedActions();
  if (!doc.body.classList.contains('auth-body')) setupShortcuts();
  watchLogStickiness();
  if (hasHtmx) {
    setupBusy();
    setupHtmxGlobal();
  }
  hydrateOpcodeDatalist();

  // Streams only make sense on pages that display live data.
  if ($('#activity-feed') || $('#dev-feed')) {
    setConnState('connecting');
    connectSSE();
  }
})();
