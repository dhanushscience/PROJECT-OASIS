<script>
  import Head from './Head.svelte';
  import Ico from './Ico.svelte';
  import { trackUnsaved } from '../busy.svelte.js';
  import { data, saveInput } from '../profiles.svelte.js';
  import { status } from '../status.svelte.js';
  import KeyBox from './KeyBox.svelte';
  import { sameChord } from '../actionKinds.js';
  import Switch from './Switch.svelte';
  import { HUES, paint } from '../hues.js';

  // What is being changed here; it starts as, and goes back to, what is saved.
  let draft = $state({ ...data.input, keys: [...data.input.keys] });
  $effect(() => { draft = { ...data.input, keys: [...data.input.keys] }; });
  // Only the keys wait for Save; the switches save on their own, so they never count as unsaved.
  const dirty = $derived(JSON.stringify(draft.keys) !== JSON.stringify(data.input.keys));
  trackUnsaved('input', () => dirty);

  let problem = $state('');
  let busy = $state(false);
  async function save() {
    problem = '';
    busy = true;
    try { await saveInput($state.snapshot(draft)); } catch (e) { problem = String(e); }
    busy = false;
  }
  // A switch saves the moment it is flipped; only the key boxes wait for Save.
  const flip = (/** @type {'hardware'|'keyboard'|'exclusive'} */ field, /** @type {boolean} */ on) => { draft[field] = on; save(); };
  const discard =() => { draft = { ...data.input, keys: [...data.input.keys] }; problem = ''; clash = ''; };

  // Two buttons cannot have the same key: it would press both. A key already taken is not accepted, and the
  // box keeps what it had.
  let clash = $state('');
  function setKey(/** @type {number} */ i, /** @type {string} */ value) {
    const other = value ? draft.keys.findIndex((k, j) => j !== i && sameChord(k, value)) : -1;
    if (other >= 0) {
      clash = `${value} is already the key for Button ${other + 1}. Choose a different key, or clear that one first.`;
      return;
    }
    clash = '';
    draft.keys[i] = value;
  }

  const engine = $derived(status.engine);
  const padLine = $derived(
    !engine ? 'Not running'
      : engine.pad === 'Off' ? 'Switched off'
      : engine.pad === 'Connected' ? `Connected${engine.battery_percent != null ? ` · battery ${engine.battery_percent}%` : ''}`
      : engine.pad === 'Unavailable' ? 'Bluetooth is off'
      : 'Looking for the OASIS interface…',
  );
  /** The dot beside the OASIS interface: green when connected, amber while it looks, grey when off. */
  const padDot = $derived(engine?.pad === 'Connected' ? 'ok' : !engine || engine.pad === 'Off' ? 'off' : 'warn');

  let btBusy = $state(false);
  let btNote = $state('');
  async function turnOnBluetooth() {
    btBusy = true;
    btNote = '';
    try { await window.__TAURI__.core.invoke('bluetooth_turn_on'); btNote = 'Bluetooth is turning on. Searching for the OASIS interface…'; }
    catch (e) { btNote = String(e); }
    setTimeout(() => (btBusy = false), 6000); // the pad connector retries by itself every few seconds
  }

  /** The ways that are coming later: shown, so people know the OASIS interface is not the only future. */
  const LATER = [
    { name: 'Phone as a deck', text: 'An Android app that is a virtual OASIS interface, connected over Bluetooth.', icon: 'M8 3h8a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H8a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM11 18h2' },
    { name: 'Laptop trackpad', text: 'Regions and gestures on the trackpad, mapped to buttons.', icon: 'M3 6h18v12H3zM3 14h18M12 14v4' },
    { name: 'Mouse ring menu', text: 'A side button opens a ring of choices at the cursor; pick by moving or scrolling.', icon: 'M12 3a6 6 0 0 1 6 6v6a6 6 0 0 1-12 0V9a6 6 0 0 1 6-6zM12 3v6' },
  ];
</script>

<div class="page">
  <section class="group ways" aria-label="Ways to press">
    <Head title="Ways to press" text="How you press the three buttons. At least one stays on." />
    <div class="mrow">
      <svg class="ico" viewBox="0 0 24 24" aria-hidden="true"><path d="M7 3h10a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zM12 8h.01M12 12h.01M12 16h.01" /></svg>
      <div class="rtext">
        <div class="rname">OASIS interface</div>
        <div class="hint"><span class="dot {padDot}" aria-hidden="true"></span>{draft.hardware ? padLine : 'Off'}{#if draft.hardware && engine?.pad === 'Connected' && engine.pad_name} · {engine.pad_name}{/if}</div>
      </div>
      {#if draft.hardware && engine?.pad === 'Unavailable'}
        <button class="quiet small" onclick={turnOnBluetooth} disabled={btBusy}>{btBusy ? 'Turning on…' : 'Turn on Bluetooth'}</button>
      {/if}
      <Switch checked={draft.hardware} label="OASIS interface" onchange={(on) => flip('hardware', on)} />
    </div>
    {#if btNote}<p class="note" role="status">{btNote}</p>{/if}

    <div class="mrow">
      <svg class="ico" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 7h18v10H3zM7 11h.01M11 11h.01M15 11h.01M8 14h8" /></svg>
      <div class="rtext"><div class="rname">Keyboard keys</div><div class="hint">{draft.keyboard ? 'On. Set the keys below.' : 'Off'}</div></div>
      <Switch checked={draft.keyboard} label="Keyboard keys" onchange={(on) => flip('keyboard', on)} />
    </div>
  </section>

  {#if draft.keyboard}
    <section class="group" aria-labelledby="keys">
      <Head id="keys" title="Keys for the three buttons" text="Click a tile, then press its shortcut. Backspace clears it." />

      <div class="tiles">
        {#each [0, 1, 2] as i}
          {@const chord = draft.keys[i]}
          {@const parts = (chord || '').split('+').map((x) => x.trim()).filter(Boolean)}
          <div class="tile" class:unset={!chord} style="--h:{HUES[i].c}">
            <div class="thead"><b class="num mono" aria-hidden="true">{i + 1}</b><span class="tname">Button {i + 1}</span></div>
            <div class="caps" aria-hidden="true">
              {#each parts as part, j}<span class="cap" style={paint(i, j === parts.length - 1)} class:main={j === parts.length - 1}>{part}</span>{:else}<span class="empty">Click to set keys</span>{/each}
            </div>
            <div class="rec" aria-hidden="true"><span class="pulse"></span>Press the keys…</div>
            <KeyBox bare label="Button {i + 1}" value={chord} onchange={(v) => setKey(i, v)} />
            {#if chord}<button class="clearkey" type="button" aria-label="Clear the key for Button {i + 1}" title="Clear" onclick={() => setKey(i, '')}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg></button>{/if}
          </div>
        {/each}
      </div>
      {#if clash}<div class="clash" role="alert">{clash}</div>{/if}

      <div class="srow plain">
        <Ico n="lock" />
        <div class="rtext"><div class="rname">Keep these keys from other programs</div><div class="hint">The program in front does not get them.</div></div>
        <Switch checked={draft.exclusive} label="Keep these keys from other programs" onchange={(on) => flip('exclusive', on)} />
      </div>
    </section>
  {/if}

  <div class="soonrow" inert>
    {#each LATER as later}
      <section class="card mini">
        <span class="badge">Coming soon</span>
        <svg class="ico" viewBox="0 0 24 24" aria-hidden="true"><path d={later.icon} /></svg>
        <h4>{later.name}</h4>
        <p class="sub">{later.text}</p>
      </section>
    {/each}
  </div>

  {#if dirty || problem}
    <div class="bar">
      {#if problem}<span class="problem" role="alert">{problem}</span>{/if}
      <span class="grow"></span>
      <button class="quiet" onclick={discard} disabled={busy}>Discard</button>
      <button class="primary" onclick={save} disabled={busy || !dirty}>Save</button>
    </div>
  {/if}
</div>

<style>
  .page { display: flex; flex-direction: column; gap: 14px; max-width: 860px; }
  .group { padding: 4px 20px 16px; border: 1px solid var(--line); border-radius: var(--r-card); background: var(--surface); box-shadow: 0 1px 3px var(--shadow); }
  .ways { padding-bottom: 4px; }
  .gl { margin: 14px 0 2px; font: 600 11px/1.3 var(--mono); letter-spacing: .12em; text-transform: uppercase; color: var(--muted); }
  .gh { margin: 0 0 12px; font-size: 13px; line-height: 1.45; color: var(--muted); max-width: 68ch; }
  /* the two ways to press: one slim row each, just a name and a line of state */
  .mrow { display: grid; grid-template-columns: 32px minmax(0, 1fr) auto auto; align-items: center; gap: 12px; padding: 9px 0; }
  .mrow + .mrow, .note + .mrow { border-top: 1px solid var(--hair); }
  .ico { width: 32px; height: 32px; padding: 7px; border-radius: 9px; background: var(--hair); fill: none; stroke: var(--ink); stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
  .rname { font-size: 14px; font-weight: 600; }
  .hint { margin-top: 1px; font-size: 12px; line-height: 1.4; color: var(--muted); }
  .note { margin: 0 0 8px 44px; font-size: 12px; color: var(--muted); }
  .dot { display: inline-block; width: 8px; height: 8px; margin-right: 7px; border-radius: 50%; background: var(--faint); vertical-align: 1px; }
  .dot.ok { background: var(--ok); }
  .dot.warn { background: var(--warn); }
  .srow { display: grid; align-items: center; gap: 14px; }
  .srow.plain { grid-template-columns: minmax(0, 1fr) auto; margin-top: 14px; padding-top: 12px; border-top: 1px solid var(--hair); }

  /* the three buttons: each carries its own colour, as on the Profiles keyboard */
  .tiles { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; }
  .tile { position: relative; display: flex; flex-direction: column; gap: 12px; min-height: 104px; padding: 14px 14px 14px; border-radius: var(--r-row); border: 1.5px solid color-mix(in srgb, var(--h) 40%, var(--line)); background: color-mix(in srgb, var(--h) 5%, var(--surface));
    transition: border-color .2s var(--ease), box-shadow .2s var(--ease), background-color .2s var(--ease); cursor: pointer; }
  .tile:hover { border-color: color-mix(in srgb, var(--h) 70%, var(--line)); }
  .tile:focus-within { border-color: var(--h); box-shadow: 0 0 0 3px color-mix(in srgb, var(--h) 30%, transparent); background: color-mix(in srgb, var(--h) 9%, var(--surface)); }
  .tile.unset { border-style: dashed; }
  .thead { display: flex; align-items: center; gap: 9px; }
  .num { width: 24px; height: 24px; flex: none; display: flex; align-items: center; justify-content: center; border-radius: 50%; background: var(--h); color: #fff; font-size: 12px; font-weight: 600; }
  .tname { font-size: 13px; font-weight: 600; color: var(--ink); }
  .caps { flex: 1; display: flex; flex-wrap: wrap; align-items: center; gap: 6px; min-height: 34px; }
  .cap { min-width: 32px; height: 32px; padding: 0 10px; display: inline-flex; align-items: center; justify-content: center; border-radius: 8px; background: var(--c); color: #4a4d55; font: 600 13px/1 var(--mono);
    box-shadow: 0 2px 0 var(--e), 0 3px 4px rgb(0 0 0 / .22); transform: translateY(-1px); }
  .cap.main { color: #fff; font-weight: 700; }
  .empty { font-size: 13px; color: var(--muted); }
  .rec { display: none; align-items: center; gap: 8px; min-height: 34px; font-size: 13px; font-weight: 600; color: var(--ink); }
  .pulse { width: 9px; height: 9px; border-radius: 50%; background: var(--h); animation: pulse 1.1s ease-in-out infinite; }
  @keyframes pulse { 50% { transform: scale(.55); opacity: .5; } }
  .tile:focus-within .caps { display: none; }
  .tile:focus-within .rec { display: flex; }
  .clearkey { position: absolute; z-index: 2; top: 10px; right: 10px; width: 24px; height: 24px; display: flex; align-items: center; justify-content: center; padding: 0; border: 0; border-radius: 50%; background: transparent; color: var(--muted); opacity: 0; transition: opacity .15s var(--ease), background-color .15s var(--ease); }
  .tile:hover .clearkey, .clearkey:focus-visible { opacity: 1; }
  .clearkey:hover { background: var(--hair); color: var(--ink); }
  .clearkey svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; }
  .clash { margin-top: 10px; font-size: 13px; font-weight: 600; color: var(--bad); }

  .card { border: 1px solid var(--line); border-radius: var(--r-card); }
  h4 { margin: 0; font-size: 14px; font-weight: 700; }
  .sub { margin: 3px 0 0; font-size: 12px; line-height: 1.4; color: var(--muted); }
  .soonrow { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; }
  .mini { position: relative; padding: 14px 16px 16px; border-style: dashed; background: transparent; box-shadow: none; opacity: .55; filter: grayscale(1); }
  .mini .ico { margin-bottom: 8px; }
  .small { height: 30px; padding: 0 14px; font-size: 12px; }
  .badge { position: absolute; top: 12px; right: 12px; padding: 3px 8px; border-radius: var(--r-pill); background: var(--hair); font: 600 10px/1 var(--mono); letter-spacing: .06em; text-transform: uppercase; color: var(--muted); }
  .bar { position: sticky; bottom: 0; display: flex; align-items: center; gap: 10px; padding: 14px 0 4px; background: linear-gradient(transparent, var(--surface) 30%); }
  .grow { flex: 1; }
  .problem { font-size: 13px; color: var(--bad); }
  .primary { height: 38px; padding: 0 22px; border-radius: var(--r-pill); border: 0; background: var(--ink); color: var(--inv); font-size: 13px; font-weight: 600; }
  .quiet { height: 38px; padding: 0 18px; border-radius: var(--r-pill); border: 1px solid var(--line); background: transparent; font-size: 13px; font-weight: 600; }
  button:disabled { opacity: .4; cursor: default; }
  @media (max-width: 760px) { .cols, .soonrow { grid-template-columns: minmax(0, 1fr); } }
</style>
