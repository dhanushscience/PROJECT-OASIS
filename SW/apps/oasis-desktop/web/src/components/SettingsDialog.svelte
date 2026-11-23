<script>
  import { tick, untrack } from 'svelte';
  import { settingsUi, closeSettings } from '../settings.svelte.js';
  import InputScreen from './InputScreen.svelte';
  import VoiceScreen from './VoiceScreen.svelte';
  import ModelsCard from './ModelsCard.svelte';
  import OverlayScreen from './OverlayScreen.svelte';
  import AppearanceCard from './AppearanceCard.svelte';
  import UpdateCard from './UpdateCard.svelte';
  import PrivacyCard from './PrivacyCard.svelte';
  import Head from './Head.svelte';
  import Ico from './Ico.svelte';
  import { sure } from '../confirm.svelte.js';
  import { send as guideSend } from '../guide/onboarding.svelte.js';
  import '../settings.css';

  const wipe = $state({ msg: '', ok: true });
  async function deleteAll() {
    wipe.msg = '';
    if (!(await sure({ title: 'Delete all data?', body: 'Every dictation recording and its text in Insights will be deleted. This cannot be undone.', yes: 'Delete all data', keep: 'Cancel' }))) return;
    try { await window.__TAURI__.core.invoke('history_delete_all'); Object.assign(wipe, { msg: 'All recordings and transcripts are deleted.', ok: true }); } catch (e) { Object.assign(wipe, { msg: String(e), ok: false }); }
  }

  // Most used first (DESIGN.md: order by frequency); rare and technical ones at the bottom.
  const SECTIONS = [
    { id: 'input', name: 'Pad & keys', about: 'How you press the three buttons: the OASIS interface, keyboard keys, or other ways. At least one stays on.', icon: 'M7 3h10a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM12 8h.01M12 12h.01M12 16h.01' },
    { id: 'voice', name: 'Voice', about: 'How a recording is turned into text and tidied, and what voice commands may do.', icon: 'M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM6 11a6 6 0 0 0 12 0M12 17v4' },
    { id: 'models', name: 'Voice models', about: 'The speech and polish models on this PC, and which set it uses.', icon: 'M4 7h16v10H4zM8 7V4M16 7V4M8 20v-3M16 20v-3M9 11h6v2H9z' },
    { id: 'overlay', name: 'Overlay & look', about: 'The three bars on your screen and how OASIS itself looks.', icon: 'M3 5h18v11H3zM8 20h8M12 16v4' },
    { id: 'gestures', name: 'Gestures', about: 'Pad gestures that work in every program, and the hidden YouTube gestures.', icon: 'M9 11V5a2 2 0 0 1 4 0v6M13 10V8a2 2 0 0 1 4 0v5a7 7 0 0 1-7 7 5 5 0 0 1-4.5-2.8L4 14a2 2 0 0 1 3.4-2L9 14' },
    { id: 'connections', soon: true, name: 'Connections', about: 'Your calendar, Spotify and the browser extension.', icon: 'M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1' },
    { id: 'general', name: 'General', about: 'Starting with Windows, button labels and scripts.', icon: 'M4 7h10M18 7h2M4 17h2M10 17h10M16 4v6M8 14v6' },
    { id: 'updates', name: 'Updates & data', about: 'New versions of OASIS, and reading your profiles from the file again.', icon: 'M12 4v11M7 10l5 5 5-5M5 20h14' },
    { id: 'privacy', name: 'Privacy', about: 'What OASIS may send to help improve it. All off until you switch it on.', icon: 'M12 3l7 3v5c0 5-3 8-7 10-4-2-7-5-7-10V6z' },
    { id: 'activity', name: 'Activity', about: 'The last button press and what OASIS did recently.', icon: 'M3 12h4l3-8 4 16 3-8h4' },
  ];

  /** @type {Record<string, HTMLElement>} */
  const panes = {};
  let query = $state('');
  let active = $state(0); // the highlighted search result
  /** @type {HTMLInputElement} */
  let searchBox;
  /** @type {HTMLElement} */
  let body;

  const current = $derived(SECTIONS.find((s) => s.id === settingsUi.section) ?? SECTIONS[0]);

  // ---- search: read the labels straight from the sections, so every setting is found without a list to keep up
  const LABELS = 'h3, strong, legend, .lab, .label, .name, .rname, .fl, .nm';
  /** The row a label names: only its own words count, not the whole card around it. */
  const ROW = '.setting, .srow, li, label, .what';

  /** Lower case, with the British and American spellings alike: "color" finds "colour". */
  const fold = (/** @type {string} */ t) => t.trim().toLowerCase().replace(/colour/g, 'color');

  /** Text with a space between the pieces (textContent runs "ends." and "15" together). */
  function words(/** @type {Element | null | undefined} */ el) {
    if (!el) return '';
    const parts = [];
    const walk = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let n = walk.nextNode(); n; n = walk.nextNode()) { const t = n.textContent?.trim(); if (t) parts.push(t); }
    return parts.join(' ').replace(/\s+/g, ' ');
  }

  /** @typedef {{ sid: string, section: string, label: string, hint: string, el: HTMLElement, score: number }} Hit */
  const results = $derived.by(() => {
    const want = fold(query).split(/\s+/).filter(Boolean);
    if (!want.length) return /** @type {Hit[]} */ ([]);
    /** @type {Hit[]} */
    const hits = [];
    for (const s of SECTIONS) {
      const pane = panes[s.id];
      if (!pane) continue;
      const seen = new Set();
      for (const el of /** @type {NodeListOf<HTMLElement>} */ (pane.querySelectorAll(LABELS))) {
        const label = words(el);
        if (label.length < 2 || label.length > 70 || seen.has(label)) continue;
        // a heading of a group has no row: its own description is what it is about
        const row = el.closest(ROW);
        const own = row ? words(row) : `${label} ${words(el.nextElementSibling)}`;
        const hint = (own.startsWith(label) ? own.slice(label.length) : own).trim();
        if (!want.every((w) => fold(`${label} ${hint} ${s.name}`).includes(w))) continue;
        seen.add(label);
        const low = fold(label);
        const score = (want.every((w) => low.includes(w)) ? (low.startsWith(want[0]) ? 0 : 1) : 2) + (row ? 0 : 0.5);
        hits.push({ sid: s.id, section: s.name, label, hint: hint.slice(0, 120), el, score });
      }
      // the section itself, by its name or what it is for
      if (want.every((w) => fold(`${s.name} ${s.about}`).includes(w)))
        hits.push({ sid: s.id, section: s.name, label: s.name, hint: s.about, el: pane, score: 0.5 });
    }
    return hits.sort((a, b) => a.score - b.score).slice(0, 40);
  });
  const counts = $derived(results.reduce((/** @type {Record<string, number>} */ m, h) => ((m[h.sid] = (m[h.sid] ?? 0) + 1), m), {}));
  $effect(() => { void query; active = 0; });

  /** @param {string} id */
  function show(id) {
    settingsUi.section = id;
    body?.scrollTo({ top: 0 });
  }

  /** Open the section a result is in, bring the setting into view and light it up for a moment. */
  async function go(/** @type {Hit} */ hit) {
    query = '';
    settingsUi.section = hit.sid;
    await tick();
    const row = /** @type {HTMLElement} */ (hit.el.closest('.setting, .srow, .card') ?? hit.el);
    row.scrollIntoView({ block: 'center', behavior: 'smooth' });
    row.classList.remove('s-flash');
    void row.offsetWidth; // restart the animation when the same row is picked twice
    row.classList.add('s-flash');
    setTimeout(() => row.classList.remove('s-flash'), 1800);
  }

  /** @param {KeyboardEvent} e */
  function searchKeys(e) {
    if (!results.length) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      active = (active + (e.key === 'ArrowDown' ? 1 : -1) + results.length) % results.length;
      document.getElementById(`s-hit-${active}`)?.scrollIntoView({ block: 'nearest' });
    } else if (e.key === 'Enter') {
      e.preventDefault();
      go(results[active]);
    }
  }

  /** Esc clears the search first, then closes. */
  function keys(/** @type {KeyboardEvent} */ e) {
    if (!settingsUi.open || e.key !== 'Escape') return;
    if (query) { query = ''; searchBox?.focus(); } else closeSettings();
  }

  // Ctrl+F goes to the search box; the pad's "back" (hold 02) closes the pop-up.
  function shortcut(/** @type {KeyboardEvent} */ e) {
    if (settingsUi.open && (e.ctrlKey || e.metaKey) && e.key === 'f') { e.preventDefault(); searchBox?.focus(); searchBox?.select(); }
  }
  // Opened from the top bar's search: the words typed there are searched here at once.
  $effect(() => {
    if (settingsUi.open) { query = untrack(() => settingsUi.query); settingsUi.query = ''; tick().then(() => searchBox?.focus()); } else query = '';
  });
</script>

<svelte:window onkeydown={(e) => { keys(e); shortcut(e); }} />

<!-- Always in the page (hidden while shut): the old script finds its fields here by id when it loads. -->
<div class="scrim" class:open={settingsUi.open} inert={!settingsUi.open} role="presentation"
  onclick={(e) => e.target === e.currentTarget && closeSettings()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="settings-title">
    <header class="top">
      <h2 id="settings-title">Settings</h2>
      <label class="search">
        <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" /></svg>
        <input bind:this={searchBox} bind:value={query} onkeydown={searchKeys} type="search" placeholder="Search settings" aria-label="Search settings"
          aria-controls="settings-results" autocomplete="off" spellcheck="false" />
        {#if query}<button class="clear" aria-label="Clear the search" onclick={() => { query = ''; searchBox.focus(); }}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg></button>
        {:else}<kbd>Ctrl F</kbd>{/if}
      </label>
      <button class="x" aria-label="Close settings" title="Close (Esc)" onclick={closeSettings}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg></button>
    </header>

    <div class="split">
      <nav aria-label="Settings sections">
        {#each SECTIONS as s}
          <button class:cur={!query && s.id === current.id} class:dim={!!query && !counts[s.id]} aria-current={!query && s.id === current.id ? 'page' : undefined}
            onclick={() => { query = ''; show(s.id); }}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d={s.icon} /></svg>
            <span class="w">{s.name}</span>
            {#if s.soon}<span class="soon">Coming soon</span>{/if}
            {#if query && counts[s.id]}<span class="n">{counts[s.id]}</span>{/if}
          </button>
        {/each}
      </nav>

      <div class="body" bind:this={body}>
        {#if query}
          <div id="settings-results" class="results" role="listbox" aria-label="Matching settings">
            {#if results.length}
              <p class="count">{results.length === 40 ? '40+' : results.length} {results.length === 1 ? 'match' : 'matches'} for “{query.trim()}”</p>
              {#each results as r, i}
                <button id="s-hit-{i}" role="option" aria-selected={i === active} class:on={i === active} onmouseenter={() => (active = i)} onclick={() => go(r)}>
                  <span class="rs">{r.section}</span>
                  <span class="rt">{r.label}</span>
                  {#if r.hint}<span class="rh">{r.hint}</span>{/if}
                </button>
              {/each}
            {:else}
              <div class="none"><strong>No setting matches “{query.trim()}”</strong><span>Try a shorter word, like “mic”, “start”, “colour” or “key”.</span></div>
            {/if}
          </div>
        {/if}

        <div class="sections" class:hidden={!!query}>
          <div class="shead">
            <h3>{current.name}</h3>
            <p>{current.about}</p>
          </div>
          <div id="settingsWarnings" class="banner" hidden></div>

          <div class="pane" bind:this={panes.input} hidden={current.id !== 'input'}><InputScreen /></div>
          <div class="pane" bind:this={panes.voice} hidden={current.id !== 'voice'}><VoiceScreen part="settings" /></div>
          <div class="pane" bind:this={panes.models} hidden={current.id !== 'models'}><ModelsCard /></div>
          <div class="pane" bind:this={panes.overlay} hidden={current.id !== 'overlay'}><OverlayScreen /><AppearanceCard /></div>
          <div class="pane legacy" bind:this={panes.gestures} hidden={current.id !== 'gestures'}><div id="settings-gestures" class="groups"></div></div>

          <div class="pane legacy greyed" bind:this={panes.connections} hidden={current.id !== 'connections'}>
            <p class="soonnote">Coming soon. Calendar, Spotify and the browser extension are still being tested, so they are switched off in this beta.</p>
            <div class="soonbody" inert>
            <div id="settings-connections" class="groups"></div>
            <div id="settings-accounts" class="groups"></div>
            <section class="card" id="bridgeCard">
              <Head title="Browser extension" text="Load it once: extensions page, Developer mode, Load unpacked, then pick this folder." />
              <div class="srow">
                <Ico n="link" />
                <div class="rtext"><div class="rname">Link to the browser</div><div id="bridgeHint" class="hint"></div><div id="youtubeText" class="hint"></div></div>
                <div class="ctl"><span id="bridgeText" class="state">Off</span><button id="openExtension" class="action quiet">Open extension folder</button></div>
              </div>
              <div id="bridgeMsg" class="msg" role="status"></div>
            </section>
            </div>
          </div>



          <div class="pane legacy" bind:this={panes.general} hidden={current.id !== 'general'}>
            <section class="card" id="systemCard">
              <Head title="Windows" text="How OASIS starts and what it may control." />
              <div class="srow">
                <Ico n="power" />
                <div class="rtext"><div class="rname">Start with Windows</div><div class="hint">Starts hidden in the tray.</div></div>
                <div class="ctl"><label class="switch" aria-label="Start with Windows"><input id="autostart" type="checkbox"><span class="track"></span></label></div>
              </div>
              <div class="srow">
                <Ico n="shield" />
                <div class="rtext"><div class="rname">Run as administrator</div><div class="hint">To control admin windows. Windows asks once.</div><div id="adminText" class="hint"></div></div>
                <div class="ctl"><label class="switch" aria-label="Run as administrator"><input id="admin" type="checkbox"><span class="track"></span></label></div>
              </div>
              <div class="tail"><div id="systemMsg" class="msg" role="status"></div><button id="restart" class="action quiet" hidden>Restart OASIS</button></div>
            </section>
            <div id="settings-general" class="groups"></div>
            <section class="card">
              <Head title="Guide" />
              <div class="srow">
                <Ico n="compass" />
                <div class="rtext"><div class="rname">Getting Started</div><div class="hint">Mr. Science shows you around again. Nothing is reset.</div></div>
                <div class="ctl"><button class="action quiet" onclick={() => { closeSettings(); guideSend('replay'); }}>Show me again</button></div>
              </div>
            </section>
          </div>

          <div class="pane legacy" bind:this={panes.updates} hidden={current.id !== 'updates'}>
            <UpdateCard />
            <section class="card">
              <Head title="Data" text="Your profiles and recordings on this PC." />
              <div class="srow">
                <Ico n="refresh" />
                <div class="rtext"><div class="rname">Reload profiles</div><div class="hint">Read profiles.json again.</div><div id="reloadMsg" class="msg" role="status"></div></div>
                <div class="ctl"><button id="reload" class="action quiet">Reload from file</button></div>
              </div>
              <div class="srow">
                <Ico n="trash" />
                <div class="rtext"><div class="rname">Delete all data</div><div class="hint">Every recording and its text in Insights. Cannot be undone.</div><div class="msg" class:ok={wipe.ok} class:bad={!wipe.ok} role="status">{wipe.msg}</div></div>
                <div class="ctl"><button class="action quiet danger" onclick={deleteAll}>Delete all data</button></div>
              </div>
            </section>
          </div>

          <div class="pane legacy" bind:this={panes.privacy} hidden={current.id !== 'privacy'}><PrivacyCard /></div>

          <div class="pane legacy" bind:this={panes.activity} hidden={current.id !== 'activity'}>
            <section class="card">
              <Head title="Last press" />
              <div id="last" class="big">Nothing yet</div>
              <div id="lastDetail" class="hint"></div>
            </section>
            <section class="card">
              <Head title="Recent activity" />
              <div id="empty" class="empty">Press an OASIS interface button to see what it would do.</div>
              <ul id="feed"></ul>
            </section>
          </div>
        </div>
      </div>
    </div>
    <footer><span id="settingsMsg" class="msg" role="status"></span><span class="tip">Changes save as you make them.</span></footer>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: 90; display: flex; align-items: center; justify-content: center; padding: 24px; background: rgb(20 20 24 / .38);
    opacity: 0; visibility: hidden; transition: opacity .22s var(--ease), visibility 0s linear .22s; }
  .scrim.open { opacity: 1; visibility: visible; transition-delay: 0s; }
  .dialog { width: min(1080px, 100%); height: min(780px, 100%); display: flex; flex-direction: column; background: var(--surface); border: 1px solid var(--line); border-radius: var(--r-panel);
    box-shadow: 0 24px 60px var(--shadow); overflow: hidden; transform: translateY(10px) scale(.985); transition: transform .26s var(--ease); }
  .scrim.open .dialog { transform: none; }

  /* top: the title, the search box in the middle, close on the right */
  .top { flex: none; display: flex; align-items: center; gap: 16px; padding: 14px 16px 14px 22px; border-bottom: 1px solid var(--hair); }
  .top h2 { margin: 0; width: 196px; flex: none; font-size: 20px; font-weight: 700; letter-spacing: -.02em; }
  .search { flex: 1; max-width: 560px; height: 40px; display: flex; align-items: center; gap: 10px; padding: 0 8px 0 14px; border-radius: var(--r-pill); border: 1.5px solid var(--line); background: var(--sunk); transition: border-color .2s var(--ease), background-color .2s var(--ease); }
  .search:focus-within { border-color: var(--ink); background: var(--surface); }
  .search > svg { width: 17px; height: 17px; flex: none; fill: none; stroke: var(--muted); stroke-width: 1.9; stroke-linecap: round; }
  .search input { flex: 1; min-width: 0; height: 100%; border: 0; background: transparent; color: var(--ink); font: 500 14px var(--sans); outline: none; }
  .search input::-webkit-search-cancel-button { display: none; }
  kbd { flex: none; font: 500 10px/1 var(--mono); letter-spacing: .06em; color: var(--faint); border: 1px solid var(--line); border-radius: 6px; padding: 4px 6px; }
  .clear, .x { flex: none; display: flex; align-items: center; justify-content: center; padding: 0; border-radius: 50%; color: var(--ink); }
  .clear { width: 26px; height: 26px; border: 0; background: var(--hair); }
  .x { width: 34px; height: 34px; margin-left: auto; border: 1px solid var(--line); background: var(--surface); }
  .clear svg, .x svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.9; stroke-linecap: round; }

  .split { flex: 1; min-height: 0; display: flex; }
  nav { width: 230px; flex: none; padding: 14px 12px; display: flex; flex-direction: column; gap: 2px; overflow-y: auto; border-right: 1px solid var(--hair); background: var(--bg); }
  nav button { display: flex; align-items: center; gap: 11px; height: 40px; padding: 0 12px; border: 0; border-radius: var(--r-row); background: transparent; color: var(--muted); font-size: 14px; font-weight: 500; text-align: left;
    transition: background-color .2s var(--ease), color .2s var(--ease), opacity .2s var(--ease); }
  nav button:hover { background: var(--hair); color: var(--ink); }
  nav button.cur { background: var(--ink); color: var(--inv); font-weight: 600; }
  nav button.dim { opacity: .38; }
  nav .soon { margin-left: auto; font: 500 10px/1 var(--mono); color: var(--faint); white-space: nowrap; }
  .soonnote { margin: 0 0 12px; padding: 10px 14px; border-radius: var(--r-row); background: var(--sunk); color: var(--muted); font-size: 13px; }
  .soonbody { opacity: .42; filter: grayscale(1); pointer-events: none; user-select: none; }
  nav svg { width: 18px; height: 18px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
  nav .w { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  nav .n { flex: none; min-width: 20px; height: 20px; padding: 0 6px; border-radius: var(--r-pill); background: var(--signal); color: var(--on-signal); font: 600 11px/20px var(--mono); text-align: center; }

  .body { flex: 1; min-width: 0; overflow-y: auto; padding: 22px 28px 28px; scroll-padding: 24px; }
  .hidden { display: none; }
  .shead { margin-bottom: 18px; }
  .shead h3 { margin: 0 0 4px; font-size: 26px; font-weight: 700; letter-spacing: -.03em; }
  .shead p { margin: 0; max-width: 70ch; font-size: 14px; line-height: 1.5; color: var(--muted); }
  .banner { margin-bottom: 14px; padding: 10px 12px; border-radius: var(--r-row); border: 1px solid var(--warn); color: var(--warn); font-size: 13px; }
  .pane { max-width: 860px; }
  /* the section's own title is shown above; the screens' built-in headings would repeat it */
  .pane :global(.page > header), .pane :global(.voice.set > header) { display: none; }

  /* search results: a flat list, the section in small type above each setting */
  .results { display: flex; flex-direction: column; gap: 2px; max-width: 860px; }
  .count { margin: 0 0 10px; font: 500 11px/1.3 var(--mono); letter-spacing: .1em; text-transform: uppercase; color: var(--muted); }
  .results button { display: grid; grid-template-columns: 1fr; gap: 1px; padding: 10px 14px; border: 1px solid transparent; border-radius: var(--r-row); background: transparent; text-align: left; color: var(--ink); }
  .results button.on { background: var(--sunk); border-color: var(--line); }
  .rs { font: 500 10px/1.4 var(--mono); letter-spacing: .1em; text-transform: uppercase; color: var(--signal-ink); }
  .rt { font-size: 14px; font-weight: 600; }
  .rh { font-size: 12px; color: var(--muted); line-height: 1.35; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .none { display: flex; flex-direction: column; gap: 4px; padding: 40px 0; text-align: center; }
  .none strong { font-size: 15px; }
  .none span { font-size: 13px; color: var(--muted); }

  footer { flex: none; display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 8px 22px; border-top: 1px solid var(--hair); min-height: 36px; }
  footer .tip { font: 500 11px/1.3 var(--mono); letter-spacing: .06em; color: var(--faint); }
  footer :global(.msg) { font-size: 12px; }
  footer :global(.msg.ok) { color: var(--ok); }
  footer :global(.msg.bad) { color: var(--bad); }

  /* ---- the settings drawn by the old script, in the profile-settings row style:
     a group with a small heading, rows of name and hint on the left and the control on the right */
  .groups { display: contents; }
  .legacy { max-width: 860px; display: flex; flex-direction: column; gap: 14px; font-size: 14px; }
  .legacy :global(.card) { background: var(--surface); border: 1px solid var(--line); border-radius: var(--r-card); padding: 6px 20px 8px; box-shadow: 0 1px 3px var(--shadow); }
  .legacy :global(.card > .label) { margin: 12px 0 2px; font: 600 11px/1.3 var(--mono); letter-spacing: .12em; text-transform: uppercase; color: var(--muted); }
  .legacy :global(.card > .hint) { margin: 0 0 6px; font-size: 13px; color: var(--muted); line-height: 1.45; max-width: 70ch; }
  .legacy :global(.setting), .legacy :global(.srow) { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: center; gap: 6px 24px; padding: 12px 0; border-top: 1px solid var(--hair); }
  .legacy :global(.card > .hint + .setting), .legacy :global(.card > .label + .setting), .legacy :global(.card > .hint + .srow), .legacy :global(.card > .label + .srow) { border-top: 0; }
  .legacy :global(.setting .name), .legacy :global(.rname) { font-size: 14px; font-weight: 600; color: var(--ink); }
  .legacy :global(.setting .hint), .legacy :global(.rtext .hint) { margin-top: 2px; font-size: 12px; color: var(--muted); line-height: 1.4; }
  .legacy :global(.setting .control), .legacy :global(.ctl) { display: flex; align-items: center; justify-content: flex-end; gap: 10px; flex-wrap: wrap; }
  .legacy :global(.setting select), .legacy :global(input[type=number]), .legacy :global(input[type=text]), .legacy :global(input[type=password]), .legacy :global(textarea) {
    height: 36px; min-width: 170px; max-width: 260px; padding: 0 12px; border-radius: var(--r-field); border: 1.5px solid var(--line); background: var(--surface); color: var(--ink); font: 500 13px var(--sans); outline: none; }
  .legacy :global(textarea) { height: auto; padding: 8px 12px; }
  /* the rounded drop-down (legacy.js `themedSelect`): the same look as the Select the Svelte screens use */
  .legacy :global(.lsel) { height: 36px; min-width: 170px; max-width: 260px; display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 0 12px; border-radius: var(--r-field); border: 1.5px solid var(--line); background: var(--surface); color: var(--ink); font: 500 13px var(--sans); text-align: left; cursor: pointer; }
  .legacy :global(.lsel:focus-visible), .legacy :global(.lsel[aria-expanded='true']) { border-color: var(--ink); outline: none; }
  .legacy :global(.lsel svg) { width: 14px; height: 14px; flex: none; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
  :global(.lsel-list) { position: fixed; z-index: 120; margin: 0; padding: 4px; list-style: none; background: var(--surface); border: 1px solid var(--line); border-radius: var(--r-row); box-shadow: 0 8px 28px var(--shadow); max-height: 260px; overflow: auto; }
  :global(.lsel-list li) { padding: 8px 10px; border-radius: var(--r-field); font: 400 13px var(--sans); color: var(--ink); cursor: pointer; }
  :global(.lsel-list li.on) { background: var(--sunk); }
  :global(.lsel-list li[aria-selected='true']) { font-weight: 600; }
  .legacy :global(select:focus-visible), .legacy :global(input:focus-visible), .legacy :global(textarea:focus-visible) { border-color: var(--ink); }
  .legacy :global(input[type=range]) { width: 170px; accent-color: var(--ink); }
  .legacy :global(.num) { min-width: 3em; text-align: right; font: 500 12px var(--mono); color: var(--muted); }
  .legacy :global(.state) { font: 600 11px/1 var(--mono); letter-spacing: .08em; text-transform: uppercase; color: var(--muted); }
  .legacy :global(.big) { margin: 4px 0 2px; font-size: 16px; font-weight: 600; }
  .legacy :global(.swatch) { width: 16px; height: 16px; border-radius: 5px; border: 1px solid var(--line); display: inline-block; }
  .legacy :global(.tail) { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .legacy :global(.msg) { font-size: 12px; color: var(--muted); }
  .legacy :global(.msg:empty) { display: none; }
  .legacy :global(.msg.ok) { color: var(--ok); }
  .legacy :global(.msg.bad) { color: var(--bad); }
  /* buttons: quiet pills, as in the profile settings */
  .legacy :global(button.action) { height: 32px; padding: 0 14px; border-radius: var(--r-pill); border: 0; background: var(--ink); color: var(--inv); font: 600 12px var(--sans); cursor: pointer; }
  .legacy :global(button.action.quiet) { border: 1px solid var(--line); background: transparent; color: var(--ink); }
  .legacy :global(button.action.danger) { color: var(--bad); }
  .legacy :global(.card > button.action) { margin: 6px 8px 8px 0; }
  .legacy :global(button.action:disabled) { opacity: .45; cursor: default; }
  /* the switch: the same pill as the profile settings, knob slides right and turns orange when on */
  .legacy :global(.switch) { position: relative; width: 52px; height: 30px; flex: none; }
  .legacy :global(.switch input) { position: absolute; inset: 0; z-index: 1; width: 100%; height: 100%; margin: 0; opacity: 0; cursor: pointer; }
  .legacy :global(.track) { position: absolute; inset: 0; border-radius: var(--r-pill); border: 1.5px solid var(--line); background: var(--hair); transition: background-color .2s var(--ease), border-color .2s var(--ease); }
  .legacy :global(.track::after) { content: ''; position: absolute; top: 2px; left: 2px; width: 23px; height: 23px; border-radius: 50%; background: var(--surface); box-shadow: var(--shadow-chosen); transition: transform .22s var(--ease), background-color .2s var(--ease); }
  .legacy :global(.switch input:checked + .track) { background: var(--ink); border-color: var(--ink); }
  .legacy :global(.switch input:checked + .track::after) { transform: translateX(22px); background: var(--signal); }
  .legacy :global(.switch input:focus-visible + .track) { outline: 2px solid var(--signal); outline-offset: 2px; }
  .legacy :global(.switch input:disabled + .track) { opacity: .5; }
  /* activity */
  .legacy :global(ul) { list-style: none; margin: 6px 0 4px; padding: 0; max-height: 360px; overflow: auto; }
  .legacy :global(li) { display: flex; gap: 12px; padding: 7px 0; border-top: 1px solid var(--hair); font-size: 13px; }
  .legacy :global(li:first-child) { border-top: 0; }
  .legacy :global(li time) { flex: none; font: 500 12px var(--mono); color: var(--muted); }
  .legacy :global(li.bad) { color: var(--bad); }
  .legacy :global(li.note) { color: var(--warn); }
  .legacy :global(li.skipped) { color: var(--muted); }
  .legacy :global(.empty) { padding: 8px 0 10px; font-size: 13px; color: var(--muted); }

  /* a setting found by search lights up for a moment */
  :global(.s-flash) { animation: s-flash 1.8s var(--ease); border-radius: var(--r-row); }
  @keyframes -global-s-flash { 0%, 35% { box-shadow: 0 0 0 3px var(--halo); background-color: color-mix(in srgb, var(--signal) 7%, transparent); } 100% { box-shadow: 0 0 0 3px transparent; } }

  /* a small window: the sections become a row of tabs over the content */
  @media (max-width: 820px) {
    .scrim { padding: 12px; }
    .top h2 { display: none; }
    .split { flex-direction: column; }
    nav { width: auto; flex-direction: row; overflow-x: auto; overflow-y: hidden; border-right: 0; border-bottom: 1px solid var(--hair); padding: 8px; scrollbar-width: none; }
    nav button { flex: none; height: 34px; }
    .body { padding: 18px; }
    .legacy :global(.setting), .legacy :global(.srow) { grid-template-columns: minmax(0, 1fr); }
    .legacy :global(.setting .control), .legacy :global(.ctl) { justify-content: flex-start; }
  }
</style>
