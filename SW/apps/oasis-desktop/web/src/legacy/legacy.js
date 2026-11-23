// @ts-nocheck: the old window script, untyped; restyled and typed screen by screen as each is rebuilt.
// The old window's script, moved here as it was; the Profiles editor is gone (rebuilt in step 3.3). Changed: tabs (the shell navigates), and the
// pad, Live and banner controls (the shell draws them). Loaded after the shell's markup exists.
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const $ = (id) => document.getElementById(id);
  let latest = null;

  // ---------------------------------------------------------------- sections
  // The shell (App.svelte) owns the navigation, the pad card, the Live switch and the banners.
  // It calls this when a section opens so the old screens load their data.
  export function openSection(name) {
    if (name === 'settings') loadSettingsTab();
  }

  // ---------------------------------------------------------------- settings
  // The Settings tab draws itself from what the engine says its settings are (oasis-settings::fields):
  // each has a kind, a limit and a current value, so nothing here knows about any one setting.
  async function loadSettingsTab() {
    try { drawSettings(await invoke('settings_fields')); }
    catch (e) { $('settingsMsg').className = 'msg bad'; $('settingsMsg').textContent = String(e); }
  }

  function drawSettings(reply) {
    const warnings = $('settingsWarnings');
    warnings.hidden = reply.warnings.length === 0;
    warnings.textContent = reply.warnings.join(' ');
    // Each section of fields goes to its place in the Settings pop-up (SettingsDialog.svelte).
    const hosts = { general: 'settings-general', pad: 'settings-gestures', youtube: 'settings-gestures', meetings: 'settings-connections' };
    // Shown on the Voice screen instead; the browser link joins the YouTube gestures it serves.
    const onVoiceScreen = new Set(['pad.sounds', 'general.paste_dictation', 'general.paste_tag', 'preferences.browser']);
    const toYoutube = new Set(['general.bridge_enabled', 'general.bridge_port']);
    for (const s of reply.sections) s.fields = s.fields.filter((f) => !onVoiceScreen.has(f.id));
    const general = reply.sections.find((s) => s.id === 'general');
    const youtube = reply.sections.find((s) => s.id === 'youtube');
    if (general && youtube) {
      youtube.fields.push(...general.fields.filter((f) => toYoutube.has(f.id)));
      general.fields = general.fields.filter((f) => !toYoutube.has(f.id));
    }
    for (const id of new Set(Object.values(hosts))) $(id)?.replaceChildren();
    for (const section of reply.sections) {
      const host = $(hosts[section.id]);
      if (!host) continue; // Voice and Overlay have their own screens.
      const card = document.createElement('section');
      card.className = 'card';
      const title = document.createElement('div');
      title.className = 'label';
      title.textContent = section.title;
      const hint = document.createElement('div');
      hint.className = 'hint';
      hint.textContent = section.hint;
      card.append(title, hint);
      for (const field of section.fields) card.append(settingRow(field));
      host.append(card);
    }
    $('settings-accounts')?.replaceChildren(meetingsCard(), spotifyCard());
  }

  async function saveSetting(call) {
    const msg = $('settingsMsg');
    try {
      drawSettings(await call());
      msg.className = 'msg ok';
      msg.textContent = 'Saved.';
    } catch (e) {
      msg.className = 'msg bad';
      msg.textContent = String(e);
      loadSettingsTab();
    }
  }

  // A passing mention: shown for a few seconds, never takes the window forward.
  function showToast(text) {
    const toast = document.createElement('div');
    toast.setAttribute('role', 'status');
    toast.style.cssText = 'position:fixed;left:16px;bottom:16px;z-index:8;max-width:calc(100% - 32px);padding:10px 14px;border-radius:8px;background:var(--surface,#222);color:var(--text,#fff);border:1px solid var(--border,#555);box-shadow:0 4px 18px rgba(0,0,0,.3)';
    toast.textContent = text;
    document.body.append(toast);
    setTimeout(() => toast.remove(), 6000);
  }

  // A reminder stays on screen until it is dismissed, so a missed chime is not a missed alarm.
  function showReminder(title, body) {
    if (title.includes('Low battery')) return; // shown in the side navigation instead
    const bar = document.createElement('div');
    bar.setAttribute('role', 'alert');
    bar.style.cssText = 'position:fixed;left:16px;right:16px;bottom:16px;z-index:9;padding:12px 14px;border-radius:8px;background:var(--accent,#2f6fed);color:#fff;display:flex;gap:12px;align-items:center;box-shadow:0 4px 18px rgba(0,0,0,.35)';
    const words = document.createElement('div');
    words.style.flex = '1';
    words.textContent = title + ' — ' + body;
    const dismiss = document.createElement('button');
    dismiss.className = 'action quiet';
    dismiss.textContent = 'Dismiss';
    dismiss.addEventListener('click', () => bar.remove());
    bar.append(words, dismiss);
    document.body.append(bar);
  }

  function spotifyCard() {
    const card = document.createElement('section');
    card.className = 'card';
    const title = document.createElement('div');
    title.className = 'label';
    title.textContent = 'Spotify';
    const hint = document.createElement('div');
    hint.className = 'hint';
    hint.textContent = 'Say "pause spotify", "next track on spotify" or "play loser on spotify". Uses the same sign-in as the older OASIS app.';
    const status = document.createElement('div');
    status.className = 'hint';
    const connect = document.createElement('button');
    connect.className = 'action quiet';
    connect.textContent = 'Connect / reconnect';
    const disconnect = document.createElement('button');
    disconnect.className = 'action quiet';
    disconnect.textContent = 'Disconnect';
    const say = (t) => { status.textContent = t; };
    invoke('spotify_status').then(say, (e) => say(String(e)));
    connect.addEventListener('click', async () => {
      say('Finish signing in in your browser…');
      try { say(await invoke('spotify_connect')); } catch (e) { say(String(e)); }
    });
    disconnect.addEventListener('click', async () => {
      try { say(await invoke('spotify_disconnect')); } catch (e) { say(String(e)); }
    });
    card.append(title, accountRow('Spotify account', [hint, status], [connect, disconnect]));
    return card;
  }

  function meetingsCard() {
    const card = document.createElement('section');
    card.className = 'card';
    const title = document.createElement('div');
    title.className = 'label';
    title.textContent = 'Google Calendar';
    const hint = document.createElement('div');
    hint.className = 'hint';
    hint.textContent = 'Shows your next meetings, reminds you, and lets "join my meeting" open the one that is starting. Uses the same sign-in as the older OASIS app.';
    const status = document.createElement('div');
    status.className = 'hint';
    const list = document.createElement('div');
    list.className = 'hint';
    const connect = document.createElement('button');
    connect.className = 'action quiet';
    connect.textContent = 'Connect / reconnect';
    const disconnect = document.createElement('button');
    disconnect.className = 'action quiet';
    disconnect.textContent = 'Disconnect';
    const draw = (reply) => {
      status.textContent = reply.status + (reply.error ? ' Last problem: ' + reply.error : '');
      list.replaceChildren(...reply.upcoming.map((m) => {
        const row = document.createElement('div');
        row.textContent = m.when + ' — ' + m.title + (m.can_join ? ' (has a link)' : '');
        return row;
      }));
    };
    const load = () => invoke('meetings_status').then(draw, (e) => { status.textContent = String(e); });
    load();
    connect.addEventListener('click', async () => {
      status.textContent = 'Finish signing in in your browser…';
      try { await invoke('meetings_connect'); } catch (e) { status.textContent = String(e); return; }
      load();
    });
    disconnect.addEventListener('click', async () => {
      try { await invoke('meetings_disconnect'); } catch (e) { status.textContent = String(e); return; }
      load();
    });
    card.append(title, accountRow('Calendar account', [hint, status, list], [connect, disconnect]));
    return card;
  }

  // A connected account as a settings row: its name and state on the left, the buttons on the right.
  function accountRow(name, texts, buttons) {
    const row = document.createElement('div');
    row.className = 'srow';
    const text = document.createElement('div');
    text.className = 'rtext';
    const label = document.createElement('div');
    label.className = 'rname';
    label.textContent = name;
    text.append(label, ...texts);
    const ctl = document.createElement('div');
    ctl.className = 'ctl';
    ctl.append(...buttons);
    row.append(text, ctl);
    return row;
  }

  // The reminder for a meeting about to start: Join, Snooze or Dismiss.
  let meetingBar = null;
  function showMeetingReminder(e) {
    hideMeetingReminder();
    const bar = document.createElement('div');
    bar.setAttribute('role', 'alert');
    bar.style.cssText = 'position:fixed;left:16px;right:16px;bottom:16px;z-index:10;padding:12px 14px;border-radius:8px;background:var(--accent,#2f6fed);color:#fff;display:flex;gap:12px;align-items:center;flex-wrap:wrap;box-shadow:0 4px 18px rgba(0,0,0,.35)';
    const words = document.createElement('div');
    words.style.flex = '1 1 200px';
    words.textContent = e.message + '  ' + e.when;
    bar.append(words);
    const add = (label, action) => {
      const b = document.createElement('button');
      b.className = 'action quiet';
      b.textContent = label;
      b.addEventListener('click', () => invoke('meeting_reminder_action', { action }).catch((err) => log(String(err), 'bad')));
      bar.append(b);
    };
    if (e.can_join) add('Join', 'join');
    add('Snooze 5 min', 'snooze');
    add('Dismiss', 'dismiss');
    document.body.append(bar);
    meetingBar = bar;
  }
  function hideMeetingReminder() {
    if (meetingBar) { meetingBar.remove(); meetingBar = null; }
  }

  function groqKeyRow() {
    const row = document.createElement('div');
    row.className = 'setting';
    const text = document.createElement('div');
    text.className = 'text';
    const name = document.createElement('div');
    name.className = 'name';
    name.textContent = 'Groq API key';
    const hint = document.createElement('div');
    hint.className = 'hint';
    text.append(name, hint);
    const control = document.createElement('div');
    control.className = 'control';
    const input = document.createElement('input');
    input.type = 'password';
    input.autocomplete = 'off';
    input.placeholder = 'gsk_…';
    input.setAttribute('aria-label', 'Groq API key');
    const save = document.createElement('button');
    save.className = 'action quiet';
    save.textContent = 'Save key';
    const test = document.createElement('button');
    test.className = 'action quiet';
    test.textContent = 'Test key';
    const say = (t) => { hint.textContent = t; };
    const refresh = async () => say((await invoke('groq_key_status')) ? 'A key is stored (protected by Windows). Save a blank box to remove it.' : 'No key stored. Only needed for the online option above.');
    save.addEventListener('click', async () => {
      try { await invoke('set_groq_key', { key: input.value }); input.value = ''; await refresh(); }
      catch (e) { say(String(e)); }
    });
    test.addEventListener('click', async () => {
      say('Testing…');
      try { say(await invoke('test_groq_key')); } catch (e) { say(String(e)); }
    });
    control.append(input, save, test);
    row.append(text, control);
    refresh();
    return row;
  }

  /**
   * The app's rounded drop-down in place of the browser's own list (which cannot be styled). The
   * `<select>` stays, hidden, as the place the value lives and the 'change' comes from; the button
   * and the list only show it and set it. Dispatch 'refresh' on the select after setting its value
   * from code. Same look as the Select component the Svelte screens use.
   */
  function themedSelect(select) {
    select.hidden = true;
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'lsel';
    button.setAttribute('aria-haspopup', 'listbox');
    button.setAttribute('aria-expanded', 'false');
    button.setAttribute('aria-label', select.getAttribute('aria-label') || '');
    const shown = document.createElement('span');
    button.append(shown);
    button.insertAdjacentHTML('beforeend', '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 9 6 6 6-6"/></svg>');
    const refresh = () => { shown.textContent = select.selectedOptions[0]?.textContent ?? ''; };
    select.addEventListener('change', refresh);
    select.addEventListener('refresh', refresh);
    refresh();

    let list = null;
    let active = 0;
    const outside = (e) => { if (!list?.contains(e.target) && !button.contains(e.target)) close(); };
    function close() {
      list?.remove();
      list = null;
      button.setAttribute('aria-expanded', 'false');
      document.removeEventListener('pointerdown', outside, true);
    }
    function pick(i) {
      const changed = select.selectedIndex !== i;
      select.selectedIndex = i;
      close();
      button.focus();
      if (changed) select.dispatchEvent(new Event('change', { bubbles: true }));
    }
    function paint() {
      [...list.children].forEach((li, i) => li.classList.toggle('on', i === active));
      list.children[active]?.scrollIntoView({ block: 'nearest' });
    }
    function open() {
      const r = button.getBoundingClientRect(); // fixed, so no panel's clipping can cut the list
      list = document.createElement('ul');
      list.className = 'lsel-list';
      list.setAttribute('role', 'listbox');
      list.style.cssText = `left:${r.left}px;top:${r.bottom + 6}px;min-width:${r.width}px`;
      [...select.options].forEach((o, i) => {
        const li = document.createElement('li');
        li.setAttribute('role', 'option');
        li.setAttribute('aria-selected', String(i === select.selectedIndex));
        li.textContent = o.textContent;
        // mousedown, not click: the button must not lose focus first
        li.addEventListener('mousedown', (e) => { e.preventDefault(); pick(i); });
        li.addEventListener('mouseenter', () => { active = i; paint(); });
        list.append(li);
      });
      document.body.append(list);
      active = Math.max(0, select.selectedIndex);
      paint();
      button.setAttribute('aria-expanded', 'true');
      document.addEventListener('pointerdown', outside, true);
    }
    button.addEventListener('click', () => (list ? close() : open()));
    button.addEventListener('keydown', (e) => {
      const n = select.options.length;
      if (!list) {
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); open(); }
        return;
      }
      if (e.key === 'ArrowDown') { e.preventDefault(); active = (active + 1) % n; paint(); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); active = (active - 1 + n) % n; paint(); }
      else if (e.key === 'Enter') { e.preventDefault(); pick(active); }
      else if (e.key === 'Escape') { e.stopPropagation(); close(); }
    });
    button.addEventListener('blur', () => setTimeout(() => { if (list && document.activeElement !== button) close(); }, 0));
    return button;
  }

  function settingRow(field) {
    const row = document.createElement('div');
    row.className = 'setting';
    const text = document.createElement('div');
    text.className = 'text';
    const name = document.createElement('div');
    name.className = 'name';
    name.textContent = field.label;
    text.append(name);
    if (field.hint) {
      const hint = document.createElement('div');
      hint.className = 'hint';
      hint.textContent = field.hint;
      text.append(hint);
    }
    const control = document.createElement('div');
    control.className = 'control';
    const save = (value) => saveSetting(() => invoke('set_setting', { id: field.id, value }));

    if (field.kind === 'toggle') {
      const label = document.createElement('label');
      label.className = 'switch';
      label.setAttribute('aria-label', field.label);
      const input = document.createElement('input');
      input.type = 'checkbox';
      input.checked = !!field.value;
      input.addEventListener('change', () => save(input.checked));
      const track = document.createElement('span');
      track.className = 'track';
      label.append(input, track);
      control.append(label);
    } else if (field.kind === 'number' && (field.max - field.min) / field.step > 300) {
      // A very wide range (a port number) is typed, not dragged.
      const input = document.createElement('input');
      input.type = 'number';
      input.min = field.min; input.max = field.max; input.step = field.step;
      input.value = field.value;
      input.style.width = '110px';
      input.setAttribute('aria-label', field.label);
      input.addEventListener('change', () => save(Number(input.value)));
      control.append(input);
    } else if (field.kind === 'number') {
      const input = document.createElement('input');
      input.type = 'range';
      input.min = field.min; input.max = field.max; input.step = field.step;
      input.value = field.value;
      input.setAttribute('aria-label', field.label);
      const shown = document.createElement('span');
      shown.className = 'num';
      const show = () => { shown.textContent = Number(input.value).toString(); };
      show();
      input.addEventListener('input', show);
      input.addEventListener('change', () => save(Number(input.value)));
      control.append(input, shown);
    } else if (field.kind === 'glossary') {
      const box = document.createElement('textarea');
      box.rows = 5;
      box.value = field.value;
      box.spellcheck = false;
      box.placeholder = 'kay cad = KiCad';
      box.setAttribute('aria-label', field.label);
      box.style.cssText = 'width:100%;min-width:260px;font:inherit';
      box.addEventListener('change', () => save(box.value));
      control.append(box);
    } else {
      const select = document.createElement('select');
      select.setAttribute('aria-label', field.label);
      for (const option of field.options) {
        const item = document.createElement('option');
        item.value = JSON.stringify(option.value);
        item.textContent = option.label;
        select.append(item);
      }
      select.value = JSON.stringify(field.value);
      select.addEventListener('change', () => {
        const chosen = JSON.parse(select.value);
        if (field.id === 'voice.stt_provider' && chosen === 1 &&
            !confirm('Groq turns your speech into text on its servers. Every recording you make is sent to api.groq.com, using your own API key. Use Groq?')) {
          select.value = JSON.stringify(field.value);
          select.dispatchEvent(new Event('refresh'));
          return;
        }
        save(chosen);
      });
      if (field.kind === 'color') {
        const swatch = document.createElement('span');
        swatch.className = 'swatch';
        swatch.style.background = field.value;
        control.append(swatch);
      }
      control.append(select, themedSelect(select));
    }
    row.append(text, control);
    return row;
  }

  // ---------------------------------------------------------------- status
  function render(reply) {
    latest = reply;
    const s = reply.engine;

    if (s && s.last_press) showLast(s.last_press);

    const voiceWords = {
      ready: ['Ready', 'Hold the talk button, speak, and let go: what you said is typed where the cursor is.'],
      off: ['Off', 'Voice is switched off in the settings.'],
      no_model: ['No speech model', 'Needs both: the model folder under Models\\stt, and the two speech-engine files (sherpa-onnx-c-api.dll and onnxruntime.dll) in a native folder beside the program. scripts\\copy-speech-engine.ps1 puts them there.'],
      unavailable: ['Not available', 'This build was started without voice.'],
    };
    const voiceNow = voiceWords[s ? s.voice : 'unavailable'] || voiceWords.unavailable;
    $('voiceText').textContent = voiceNow[0];
    $('voiceHint').textContent = voiceNow[1];

    const status = s && s.bridge_status ? s.bridge_status : 'OFF';
    $('bridgeText').textContent = status;
    $('bridgeHint').textContent = !s ? '' : s.bridge_clients > 0
      ? s.bridge_clients + ' browser' + (s.bridge_clients > 1 ? 's' : '') + ' connected'
      : (status.startsWith('LISTENING') ? 'No browser connected yet.' : '');
    const youtubeWords = { 'youtube.playing': 'A video is playing', 'youtube.ended': 'A video has ended', 'youtube.shorts': 'Watching Shorts' };
    $('youtubeText').textContent = s && s.youtube
      ? (youtubeWords[s.youtube] || s.youtube) + (s.youtube_title ? ': ' + s.youtube_title : '') : '';
  }

  function describe(p) {
    return p.kind ? p.kind + (p.value ? ' ' + p.value : '') : 'nothing bound to this button';
  }

  function showLast(p) {
    $('last').textContent = 'Button ' + p.button + ' → ' + describe(p);
    $('lastDetail').textContent = p.profile + (p.config && p.config !== p.profile ? ' / ' + p.config : '') +
      (p.runs ? '' : ' · not run');
  }

  function log(text, cls) {
    $('empty').hidden = true;
    const li = document.createElement('li');
    if (cls) li.className = cls;
    const t = document.createElement('time');
    t.textContent = new Date().toTimeString().slice(0, 8);
    const span = document.createElement('span');
    span.textContent = text;
    li.append(t, span);
    const feed = $('feed');
    feed.prepend(li);
    while (feed.children.length > 40) feed.lastChild.remove();
  }

  listen('engine-event', ({ payload: e }) => {
    switch (e.type) {
      case 'press':
        showLast(e);
        log('Button ' + e.button + ' [' + e.profile + '] ' + describe(e) + (e.runs ? '' : ' (not run)'),
            e.runs ? '' : 'skipped');
        break;
      case 'unmapped':   log('Button ' + e.button + ': ' + e.gesture + ' is not wired yet', 'skipped'); break;
      case 'action_failed': log(e.what + ' failed: ' + e.reason, 'bad'); break;
      case 'note':       log(e.text, 'note'); break;
      case 'meeting_reminder': log(e.message, 'note'); showMeetingReminder(e); break;
      case 'meeting_reminder_end': hideMeetingReminder(); break;
      case 'notice':     log(e.title + ': ' + e.body, 'note'); showToast(e.title + ' — ' + e.body); break;
      case 'reminder':   log(e.title + ': ' + e.body, 'note'); showReminder(e.title, e.body); break;
      case 'pad_connected': log('Connected to ' + e.name); break;
      case 'pad_lost':   log('OASIS interface connection lost', 'note'); break;
      case 'pad_unavailable': log(e.reason, 'bad'); break;
      case 'settings_changed': log('Settings saved'); break;
      case 'voice_command': {
        const said = '“' + e.heard + '”';
        const words = {
          ran: 'Ran ' + e.label,
          preview: said + ' → ' + e.label + ' (not run: actions are off)',
          asking: said + ' → waiting for you: ' + e.detail,
          blocked: said + ' → blocked' + (e.detail ? ': ' + e.detail : ''),
          cancelled: said + ' → cancelled',
          no_match: e.detail || (said + ': no match'),
          ignored: e.detail || (said + ': nothing usable'),
        };
        const text = e.outcome === 'ran' ? said + ' → ' + words.ran : (words[e.outcome] || said);
        log(text, e.outcome === 'ran' ? '' : (e.outcome === 'blocked' ? 'bad' : 'skipped'));
        break;
      }
      case 'transcript': log('Heard: “' + e.text + '”' + (e.typed ? ' (typed)' : ' (not typed: actions are off)'), e.typed ? '' : 'skipped'); break;
      default: break;
    }
    refresh();
  });

  $('openExtension').addEventListener('click', async () => {
    try { await invoke('open_extension_folder'); $('bridgeMsg').textContent = ''; }
    catch (err) { $('bridgeMsg').textContent = String(err); }
  });

  async function refresh() {
    try { render(await invoke('status')); } catch (err) { log(String(err), 'bad'); }
  }

  function showSystem(s) {
    $('autostart').checked = s.start_with_windows;
    $('admin').checked = s.run_as_administrator;
    $('adminText').textContent = s.admin_text;
    $('restart').hidden = !s.needs_restart;
    if (s.rehearsal) $('systemMsg').textContent = 'Rehearsal mode: these switches still change the real Windows entries.';
  }
  async function refreshSystem() {
    try { showSystem(await invoke('system_status')); } catch (e) { $('systemMsg').textContent = String(e); }
  }
  async function changeSystem(command, input, on) {
    $('systemMsg').textContent = command === 'set_run_as_administrator' ? 'Waiting for Windows approval…' : '';
    input.disabled = true;
    try {
      showSystem(await invoke(command, { on }));
      $('systemMsg').textContent = 'Saved.';
    } catch (e) {
      $('systemMsg').textContent = String(e);
      await refreshSystem();
    }
    input.disabled = false;
  }
  $('autostart').addEventListener('change', (ev) => changeSystem('set_start_with_windows', ev.target, ev.target.checked));
  $('admin').addEventListener('change', (ev) => changeSystem('set_run_as_administrator', ev.target, ev.target.checked));
  $('restart').addEventListener('click', async () => {
    try { await invoke('restart_oasis'); } catch (e) { $('systemMsg').textContent = String(e); }
  });
  refreshSystem();


  // ---------------------------------------------------------------- reload
  function say(el, text, kind) {
    el.textContent = text || '';
    el.className = 'msg' + (kind ? ' ' + kind : '');
  }

  $('reload').addEventListener('click', async () => {
    say($('reloadMsg'), 'Reloading…');
    try {
      await invoke('reload_settings');
      say($('reloadMsg'), 'Reloaded from the file.', 'ok');
    } catch (err) {
      say($('reloadMsg'), String(err), 'bad');
    }
  });

  refresh();
  setInterval(refresh, 2000);
