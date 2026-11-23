<script>
  import { status } from '../status.svelte.js';
  import Select from './Select.svelte';
  import Switch from './Switch.svelte';
  import Head from './Head.svelte';
  import Ico from './Ico.svelte';
  import TryCommands from './TryCommands.svelte';
  import { data } from '../profiles.svelte.js';
  import { paint } from '../hues.js';
  import { openSettings } from '../settings.svelte.js';

  /** `main`: the Voice section's dictionary and fragments. `talk`: the talk button and microphone, beside the section title.
      `settings`: the rest, set once, in Settings. */
  let { part = 'main' } = $props();

  const { invoke } = window.__TAURI__?.core ?? { invoke: async () => { throw new Error('Not running inside OASIS.'); } };

  /** @typedef {{ id: string, label: string, hint: string, kind: string, value: any, min: number | null, max: number | null, step: number | null, options: { value: any, label: string }[] }} Field */

  /** every voice setting by id, as the engine describes it (its limits, choices and current value) */
  let fields = $state(/** @type {Record<string, Field>} */ ({}));
  let loaded = $state(false);
  let tryOpen = $state(false);
  let problem = $state('');
  let saved = $state('');
  const ready = $derived(!!status.engine);
  const locked = $derived(!!status.engine && !status.engine.integrity_ok);

  /** @param {{ sections: { id: string, fields: Field[] }[] }} reply */
  function take(reply) {
    const voice = reply.sections.find((s) => s.id === 'voice');
    fields = Object.fromEntries([...(voice?.fields ?? []), ...reply.sections.flatMap((s) => s.fields).filter((f) => ['pad.sounds', 'general.paste_dictation', 'preferences.browser'].includes(f.id))].map((f) => [f.id, f]));
    loaded = true;
  }
  async function load() {
    try { take(await invoke('settings_fields')); groqKey = await invoke('groq_key_status'); } catch (e) { problem = String(e); }
  }
  $effect(() => { if (ready && !loaded) load(); });
  // A change made elsewhere (a word the overlay learned from a fix) shows up here at once.
  $effect(() => {
    const off = window.__TAURI__?.event.listen('engine-event', (/** @type {{ payload: { type: string } }} */ e) => {
      if (e.payload.type === 'settings_changed' && loaded) load();
    });
    return () => { off?.then((/** @type {() => void} */ stop) => stop()); };
  });

  let savedTimer = 0;
  /** Every change saves at once; the engine checks it against the same limits as everywhere else. */
  async function set(/** @type {string} */ id, /** @type {any} */ value) {
    problem = '';
    try {
      take(await invoke('set_setting', { id, value }));
      saved = 'Saved';
      clearTimeout(savedTimer);
      savedTimer = setTimeout(() => (saved = ''), 1800);
    } catch (e) { problem = String(e); }
  }
  const v = (/** @type {string} */ id) => fields[id]?.value;
  const opts = (/** @type {string} */ id) => fields[id]?.options ?? [];
  const label = (/** @type {string} */ id) => opts(id).find((/** @type {{ value: any }} */ o) => o.value === v(id))?.label ?? '';

  // ---- push-to-talk state, as the engine reports it
  const WORDS = /** @type {Record<string, [string, string]>} */ ({
    ready: ['Ready', 'Hold the button, speak, and let go.'],
    off: ['Off', 'Turn on push-to-talk to use your voice.'],
    no_model: ['No speech model', 'Needs a model under Models\\stt and the speech-engine files beside the program.'],
    unavailable: ['Not available', 'This build was started without voice.'],
  });
  const talkButtons = $derived(String(v('voice.talk_buttons') ?? '0').split('+').map(Number).filter((n) => n >= 0 && n < 3));
  /** what to show on a button's key cap: its keyboard key when keyboard buttons are on, else its number */
  const capFor = (/** @type {number} */ i) => (data.input.keyboard && data.input.keys[i]) || `0${i + 1}`;
  /** the keys on a button's tile: its keyboard shortcut, split into caps ("Ctrl", "Shift", "1") */
  const keyParts = (/** @type {number} */ i) => (data.input.keyboard ? String(data.input.keys[i] ?? '') : '').split('+').map((p) => p.trim()).filter(Boolean);
  function pickTalk(/** @type {number} */ i) {
    let next = talkButtons.includes(i) ? talkButtons.filter((b) => b !== i) : [...talkButtons, i];
    if (!next.length) return; // one is always held to talk
    if (next.length > 2) next = next.slice(-2);
    set('voice.talk_buttons', [...next].sort().join('+'));
  }
  const talk = $derived(WORDS[status.engine?.voice ?? 'unavailable'] ?? WORDS.unavailable);

  // The gear opens a small panel; a click outside it, or Esc, closes it.
  let talkOpen = $state(false);
  /** @type {HTMLElement | undefined} */
  let talkBox = $state();
  $effect(() => {
    if (!talkOpen) return;
    const away = (/** @type {PointerEvent} */ e) => { if (!talkBox?.contains(/** @type {Node} */ (e.target))) talkOpen = false; };
    const esc = (/** @type {KeyboardEvent} */ e) => { if (e.key === 'Escape') talkOpen = false; };
    window.addEventListener('pointerdown', away);
    window.addEventListener('keydown', esc);
    return () => { window.removeEventListener('pointerdown', away); window.removeEventListener('keydown', esc); };
  });


  // ---- speech to text
  let groqKey = $state(false);
  let keyText = $state('');
  let keyNote = $state('');
  let askGroq = $state(false);
  const groq = $derived(v('voice.stt_provider') === 1);
  async function saveKey() {
    keyNote = '';
    try { await invoke('set_groq_key', { key: keyText }); groqKey = await invoke('groq_key_status'); keyText = ''; keyNote = groqKey ? 'Key saved.' : 'Key removed.'; } catch (e) { keyNote = String(e); }
  }
  async function testKey() {
    keyNote = 'Testing…';
    try { keyNote = await invoke('test_groq_key'); } catch (e) { keyNote = String(e); }
  }

  // ---- the dictionary: what the model hears → what you meant; and fragments: what you say → your own text to
  // type (an email address, say). Each list is its own card, with its own add box, search and editing state.
  const TABS = {
    words: { id: 'voice.dictation_glossary', title: 'Dictionary', heard: 'Heard as (optional), e.g. kay cad, keycat', write: 'Write as, e.g. KiCad', add: 'Add word', empty: 'No words yet. Add a name or term you use, and, if the model mishears it, how it sounds.',
      hint: 'Names and terms you use. Add just the word, or also how it is misheard: that is fixed after speech-to-text, before anything is typed or a command runs. The words are also given to the cleanup model, and, with Groq on, sent with each recording.' },
    repl: { id: 'voice.replacements', title: 'Fragments', heard: 'When you say, e.g. my mail', write: 'Type, e.g. you@example.com', add: 'Add fragment', empty: 'No fragments yet. Add a phrase you say, and the text to type instead.',
      hint: 'Your own text for a phrase you say. Say it on its own, or after “insert”, to type just the text: “insert my mail”. Inside a sentence it is swapped in: “send it to my mail”. Typed exactly as written.' },
  };
  /** @typedef {'words' | 'repl'} ListKey */
  const parse = (/** @type {string} */ id) =>
    String(v(id) ?? '')
      .split('\n')
      .map((line) => line.split('='))
      .filter((p) => (p.length >= 2 ? p[0].trim() && p.slice(1).join('=').trim() : p[0].trim()))
      // a line with no = is a word on its own: heard the same as it is written
      .map((p) => (p.length >= 2 ? { heard: p[0].trim(), write: p.slice(1).join('=').trim() } : { heard: p[0].trim(), write: p[0].trim() }));
  const words = $derived(parse(TABS.words.id));
  const repls = $derived(parse(TABS.repl.id));
  const lists = $derived({ words, repl: repls });
  // One row per word as it is written: every way it is heard sits on that row ("key CAD · keycat → KiCad").
  // Saving keeps that: entries written the same way (ignoring capitals) are merged, in the newest spelling.
  const form = $state({ words: { heard: '', write: '', editing: '', query: '', more: false }, repl: { heard: '', write: '', editing: '', query: '', more: false } });
  /** @typedef {{ key: string, write: string, heard: string[] }} Group */
  const groupsOf = (/** @type {{ heard: string, write: string }[]} */ pairs) => {
    /** @type {Map<string, Group>} */
    const by = new Map();
    for (const p of pairs) {
      const key = p.write.toLowerCase();
      const g = by.get(key) ?? { key, write: p.write, heard: [] };
      g.write = p.write; // the newest spelling wins
      if (!g.heard.some((h) => h.toLowerCase() === p.heard.toLowerCase())) g.heard.push(p.heard);
      by.set(key, g);
    }
    return [...by.values()];
  };
  const groups = $derived({ words: groupsOf(words), repl: groupsOf(repls) });
  const shown = (/** @type {ListKey} */ k) => {
    const q = form[k].query.trim().toLowerCase();
    return groups[k].filter((g) => !q || `${g.heard.join(' ')} ${g.write}`.toLowerCase().includes(q));
  };
  /** Saved one line per way it is heard, as the engine reads it; merged first. */
  const saveList = (/** @type {ListKey} */ k, /** @type {{ heard: string, write: string }[]} */ next) =>
    set(TABS[k].id, groupsOf(next).flatMap((g) => g.heard.map((h) => (h === g.write ? g.write : `${h} = ${g.write}`))).join('\n'));
  function addWord(/** @type {ListKey} */ k) {
    const fm = form[k];
    // several ways it is heard can go in at once, separated by commas
    const said = fm.heard.split(',').map((h) => h.trim()).filter(Boolean);
    const w = fm.write.trim();
    if (!w) return;
    if (!said.length) { if (k !== 'words') return; said.push(w); } // a dictionary word needs no 'heard as'
    if (said.some((h) => h.includes('='))) { problem = 'What you say cannot contain “=”.'; return; }
    const lower = said.map((h) => h.toLowerCase());
    // the row being edited is replaced; a heard form already used elsewhere moves to this word
    const rest = lists[k].filter((x) => x.write.toLowerCase() !== fm.editing && !lower.includes(x.heard.toLowerCase()));
    saveList(k, [...rest, ...said.map((h) => ({ heard: h, write: w }))]);
    fm.heard = fm.write = '';
    fm.editing = '';
    fm.more = false;
  }
  function editWord(/** @type {ListKey} */ k, /** @type {Group} */ g) { const fm = form[k]; fm.editing = g.key; fm.heard = g.heard.filter((h) => h !== g.write).join(', '); fm.write = g.write; fm.more = !!fm.heard; }
  function removeWord(/** @type {ListKey} */ k, /** @type {Group} */ g) {
    saveList(k, lists[k].filter((x) => x.write.toLowerCase() !== g.key));
    if (form[k].editing === g.key) { form[k].editing = ''; form[k].heard = form[k].write = ''; form[k].more = false; }
  }
  /** The Dictionary shows a few rows of each kind; the rest, with search, is in a window. */
  const MY_SHOWN = 12; // chips; a shortcut over measuring the box, since a chip's width varies
  let allOpen = $state(false);
  /** True when the corrected-words list is taller than the room the card leaves it: only then the dots show. */
  let over = $state(false);
  /** @param {HTMLElement} node @param {number} _n */
  function overflows(node, _n) {
    const f = () => (over = node.scrollHeight - node.clientHeight > 1);
    const ro = new ResizeObserver(f);
    ro.observe(node);
    f();
    return { update: () => queueMicrotask(f), destroy: () => ro.disconnect() };
  }
  const portal = (/** @type {HTMLElement} */ node) => { document.body.appendChild(node); return { destroy: () => node.remove() }; };
  const stopEditing = (/** @type {ListKey} */ k) => { form[k].editing = ''; form[k].heard = form[k].write = ''; form[k].more = false; };
</script>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape') allOpen = false; }} />

{#snippet wordRow(/** @type {ListKey} */ k, /** @type {Group} */ g)}
  {@const fm = form[k]}
  <div class="word" class:sel={g.key === fm.editing}>
    <button class="pick" onclick={() => editWord(k, g)} title={g.heard.length > 1 ? `Heard as: ${g.heard.join(', ')}` : 'Edit'}>{#if k === 'words'}<span class="w">{g.write}</span>{#if g.heard.some((h) => h !== g.write)}<span class="to" aria-hidden="true">←</span><span class="h mono">{g.heard.filter((h) => h !== g.write).join(' · ')}</span>{/if}{:else}{#if g.heard.some((h) => h !== g.write)}<span class="h mono">{g.heard.filter((h) => h !== g.write).join(' · ')}</span><span class="to" aria-hidden="true">→</span>{/if}<span class="w">{g.write}</span>{/if}</button>
    <button class="del" aria-label="Remove {g.write}" onclick={() => removeWord(k, g)}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg></button>
  </div>
{/snippet}

{#snippet addForm(/** @type {ListKey} */ k)}
  {@const t = TABS[k]}
  {@const fm = form[k]}
  {#if k === 'words'}
    <!-- one thing is needed: the word. How it gets misheard is a second row that opens on request. -->
    <div class="add1">
      <input class="main" bind:value={fm.write} placeholder="Word to add, e.g. KiCad" aria-label="Word to add" autocomplete="off" onkeydown={(e) => e.key === 'Enter' && addWord(k)} />
      <span class="addbtns">
        {#if saved}<span class="saved mono" role="status">✓ {saved}</span>{/if}
        <button class="dark" disabled={!fm.write.trim()} onclick={() => addWord(k)}>{fm.editing ? 'Update' : t.add}</button>
        {#if fm.editing}<button class="quiet" onclick={() => stopEditing(k)}>Cancel</button>{/if}
      </span>
      {#if fm.more}
        <input class="main" bind:value={fm.heard} placeholder="Misheard as, e.g. kay cad, keycat" title="Separate several with commas" aria-label="Misheard as" autocomplete="off" onkeydown={(e) => e.key === 'Enter' && addWord(k)} />
      {:else}
        <button class="more" title="If it gets misheard, add how it sounds" onclick={() => (fm.more = true)}><b aria-hidden="true">+</b> Misheard as</button>
      {/if}
    </div>
  {:else}
  <div class="add">
    <input bind:value={fm.heard} placeholder={t.heard} aria-label={t.heard} autocomplete="off" onkeydown={(e) => e.key === 'Enter' && addWord(k)} />
    <span class="to" aria-hidden="true">→</span>
    <input bind:value={fm.write} placeholder={t.write} aria-label={t.write} autocomplete="off" onkeydown={(e) => e.key === 'Enter' && addWord(k)} />
    <span class="addbtns">
      {#if saved}<span class="saved mono" role="status">✓ {saved}</span>{/if}
      <button class="dark" disabled={!fm.heard.trim() || !fm.write.trim()} onclick={() => addWord(k)}>{fm.editing ? 'Update' : t.add}</button>
      {#if fm.editing}<button class="quiet" onclick={() => stopEditing(k)}>Cancel</button>{/if}
    </span>
  </div>
  {/if}
{/snippet}

{#snippet chip(/** @type {ListKey} */ k, /** @type {Group} */ g)}
  <span class="chip" class:sel={g.key === form[k].editing}>
    <button class="pick" title="Edit" onclick={() => editWord(k, g)}>{g.write}</button>
    <button class="del" aria-label="Remove {g.write}" onclick={() => removeWord(k, g)}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg></button>
  </span>
{/snippet}

{#snippet viewAll(/** @type {number} */ n)}
  <button class="viewall" title="Open the whole dictionary: search, edit, remove" onclick={() => (allOpen = true)}>
    View all {n}<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 17 17 7M9 7h8v8" /></svg>
  </button>
{/snippet}

{#snippet listCard(/** @type {ListKey} */ k)}
  {@const t = TABS[k]}
  {@const fm = form[k]}
  <section class="card dict" aria-label={t.title}>
    <div class="chead">
      <span class="lab">{t.title} <span class="cnt">· {groups[k].length}</span></span>
      <input class="search" bind:value={fm.query} placeholder="Search" aria-label="Search {t.title}" autocomplete="off" />
    </div>
    <span class="hint">{t.hint}</span>
    {@render addForm(k)}
    <!-- fragments: one per row, nothing cut short: what you say on the left, what it types on the right, wrapped -->
    {#if k === 'repl' && shown(k).length}<div class="fhead" aria-hidden="true"><span>You say</span><span>It types</span></div>{/if}
    {#if k === 'words'}
      {@const mine = shown(k).filter((g) => g.heard.every((h) => h === g.write))}
      {@const fixed = shown(k).filter((g) => !g.heard.every((h) => h === g.write))}
      <!-- your own words on top, the mis-heard ones with their fix below; a search looks through both -->
      {#if mine.length}
        <div class="subrow"><span class="sub">My words <span class="cnt">· {mine.length}</span></span>
          {#if mine.length > MY_SHOWN || over}{@render viewAll(groups.words.length)}{/if}</div>
        <div class="chips">{#each mine.slice(0, MY_SHOWN) as g (g.key)}{@render chip(k, g)}{/each}</div>
      {/if}
      {#if fixed.length}
        <div class="subrow"><span class="sub">Corrected words <span class="cnt">· {fixed.length}</span></span>
          {#if over && !mine.length}{@render viewAll(groups.words.length)}{/if}</div>
        <div class="words clip" use:overflows={fixed.length}>{#each fixed as g (g.key)}{@render wordRow(k, g)}{/each}</div>
      {/if}
      {#if allOpen}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div class="scrim" use:portal onclick={() => (allOpen = false)}>
          <div class="dpop" role="dialog" tabindex="-1" aria-modal="true" aria-label="Dictionary" onclick={(e) => e.stopPropagation()}>
            <div class="top">
              <span class="lab">Dictionary <span class="cnt">· {groups.words.length}</span></span>
              <button class="x" aria-label="Close" onclick={() => (allOpen = false)}><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg></button>
            </div>
            {@render addForm(k)}
            <input class="search wide" bind:value={fm.query} placeholder="Search words" aria-label="Search words" autocomplete="off" />
            <div class="scroll">
              {#if mine.length}<span class="sub">My words <span class="cnt">· {mine.length}</span></span><div class="chips">{#each mine as g (g.key)}{@render chip(k, g)}{/each}</div>{/if}
              {#if fixed.length}<span class="sub">Corrected words <span class="cnt">· {fixed.length}</span></span><div class="words fit">{#each fixed as g (g.key)}{@render wordRow(k, g)}{/each}</div>{/if}
              {#if !mine.length && !fixed.length}<p class="hint pad">Nothing matches.</p>{/if}
            </div>
          </div>
        </div>
      {/if}
      {#if !mine.length && !fixed.length}<p class="hint pad">{fm.query ? 'Nothing matches.' : t.empty}</p>{/if}
    {:else}
      <div class="words full">
        {#each shown(k) as g (g.key)}{@render wordRow(k, g)}{:else}<p class="hint pad">{fm.query ? 'Nothing matches.' : t.empty}</p>{/each}
      </div>
    {/if}
  </section>
{/snippet}

{#snippet pick(/** @type {string} */ id, /** @type {string} */ name, hint = '', ico = '')}
  <div class="srow">
    {#if ico}<Ico n={ico} />{/if}
    <div class="rtext"><div class="rname">{name}</div>{#if hint || fields[id]?.hint}<div class="hint">{hint || fields[id]?.hint}</div>{/if}</div>
    <div class="ctl sel"><Select value={v(id)} options={opts(id)} label={name} onchange={(x) => set(id, x)} /></div>
  </div>
{/snippet}

{#snippet toggle(/** @type {string} */ id, /** @type {string} */ name, hint = '', ico = '')}
  <div class="srow">
    {#if ico}<Ico n={ico} />{/if}
    <div class="rtext"><div class="rname">{name}</div>{#if hint || fields[id]?.hint}<div class="hint">{hint || fields[id]?.hint}</div>{/if}</div>
    <Switch checked={!!v(id)} label={name} onchange={(on) => set(id, on)} />
  </div>
{/snippet}

<div class="vwrap" class:inline={part === 'talk'}>
  {#if !ready}
    {#if part !== 'talk'}<p class="hint">Voice settings appear here once OASIS is running.</p>{/if}
  {:else if loaded}
  {#if part === 'settings'}
  <div class="voice set" inert={locked}>
    <section class="group" aria-label="Hold to talk">
      <Head title="Hold to talk" text="Hold the button, speak, let go. A short tap still runs its own action.">
        <Switch checked={!!v('voice.enabled')} label="Push-to-talk" onchange={(on) => set('voice.enabled', on)} />
      </Head>
      <div class="srow">
        <Ico n="press" />
        <div class="rtext"><div class="rname">Button to hold</div><div class="hint">Pick one, or two to hold together.</div></div>
        <div class="tiles" role="group" aria-label="Button to hold to talk">
          {#each [0, 1, 2] as i}
            {@const on = talkButtons.includes(i)}
            {@const parts = keyParts(i)}
            <button type="button" class="tile" class:on aria-pressed={on} style={paint(i, on)} onclick={() => pickTalk(i)}
              aria-label="Button {i + 1}{parts.length ? `, ${parts.join(' ')}` : ''}">
              <span class="num mono">0{i + 1}</span>
              {#if parts.length}<span class="chord">{#each parts as k, j}{#if j}<span class="pl">+</span>{/if}<kbd class="mono">{k}</kbd>{/each}</span>{/if}
            </button>
          {/each}
        </div>
      </div>
      {@render toggle('pad.sounds', 'Click sounds', 'A short click when recording starts and ends.', 'waves')}
      {@render pick('voice.speaker', 'Sound output', 'Where those clicks play.', 'speaker')}
    </section>

    <section class="group" aria-label="Speech to text">
      <Head title="Speech to text" text={groq ? 'Each recording goes to api.groq.com with your own key.' : 'Private: the recording never leaves this computer.'} />
      <div class="srow">
        <Ico n={groq ? 'cloud' : 'screen'} />
        <div class="rtext"><div class="rname">Where it is turned into text</div></div>
        <div class="seg" role="radiogroup" aria-label="Where speech is turned into text">
          <button type="button" role="radio" aria-checked={!groq} class:on={!groq} onclick={() => { askGroq = false; set('voice.stt_provider', 2); }}>This computer</button>
          <button type="button" role="radio" aria-checked={groq} class:on={groq} onclick={() => { if (!groq) askGroq = true; }}>Groq online</button>
        </div>
      </div>
      {#if askGroq}
        <div class="ask">
          <span class="hint">Every recording would be sent to api.groq.com with your own key. Use Groq?</span>
          <span class="btns"><button class="quiet" onclick={() => (askGroq = false)}>Cancel</button><button class="dark" onclick={() => { askGroq = false; set('voice.stt_provider', 1); }}>Use Groq</button></span>
        </div>
      {:else if groq}
        <div class="srow">
          <Ico n="apikey" />
          <div class="rtext"><div class="rname">Groq API key</div><div class="hint">{keyNote || (groqKey ? 'Stored, protected by Windows.' : 'Needed for the online option.')}</div></div>
          <div class="ctl key">
            <input type="password" bind:value={keyText} placeholder={groqKey ? 'Key saved' : 'gsk_…'} aria-label="Groq API key" autocomplete="off" />
            <button class="quiet" onclick={saveKey} disabled={!keyText.trim() && !groqKey}>{keyText.trim() || !groqKey ? 'Save' : 'Remove'}</button>
            <button class="quiet" onclick={testKey} disabled={!groqKey}>Test</button>
          </div>
        </div>
        {@render toggle('voice.groq_fallback', 'If Groq fails, use this computer', 'Keeps working offline.', 'refresh')}
      {/if}
    </section>

    <section class="group" aria-label="Clean up">
      <Head title="Clean up" text={v('voice.dictation_cleanup') ? 'Punctuation, digits and capitals added; “um” removed.' : 'Typed exactly as it was heard.'}>
        <Switch checked={!!v('voice.dictation_cleanup')} label="Fix grammar and punctuation" onchange={(on) => set('voice.dictation_cleanup', on)} />
      </Head>
      {@render toggle('general.paste_dictation', 'Type it where the cursor is', 'Off: only kept in Insights.', 'text')}
      {@render pick('voice.capture_mode', 'A recording becomes', 'Text, a command, or decided by what you say.', 'branch')}
    </section>

    <section class="group" aria-label="Polish">
      <Head title="Polish" text={v('voice.polish') ? 'A local model smooths each sentence. Needs a graphics card.' : 'Off: the rules alone clean the text.'}>
        <Switch checked={!!v('voice.polish')} label="Polish with the local model" onchange={(on) => set('voice.polish', on)} />
      </Head>
      <div class="rows" class:off={!v('voice.polish')}>
        {@render pick('voice.polish_tone', 'Tone', 'Casual to formal.', 'smile')}
        {@render pick('voice.polish_structure', 'Structure', 'Sentences, or lists.', 'list')}
        {@render pick('voice.polish_context', 'Context', 'Email style in a mail program.', 'mail')}
        {@render pick('voice.polish_wait_ms', 'Wait limit', 'Longer on battery.', 'clock')}
      </div>
    </section>

    <section class="group" aria-label="Voice commands">
      <Head title="Voice commands" text="Say what to do, and OASIS does it.">
        {#if saved}<span class="saved mono" role="status">{saved}</span>{/if}
      </Head>
      {@render pick('voice.approval_mode', 'Ask before running', '', 'shield')}
      {@render toggle('voice.allow_launch_app', 'Let voice open apps', '', 'apps')}
      {@render toggle('voice.allow_type_text', 'Let voice type text', '', 'keyboard')}
      {@render toggle('voice.edit_commands', 'Editing commands', '', 'scissors')}
      {@render pick('preferences.browser', 'Preferred browser', 'For a spoken search.', 'globe')}
      {#if fields['voice.match_threshold']}
        <div class="srow">
          <Ico n="target" />
          <div class="rtext"><div class="rname">How exact a command must be</div><div class="hint">Lower catches more, higher only close matches.</div></div>
          <div class="ctl"><input type="range" aria-label="How exact a command must be" min={fields['voice.match_threshold'].min} max={fields['voice.match_threshold'].max} step={fields['voice.match_threshold'].step} value={v('voice.match_threshold')}
            onchange={(e) => set('voice.match_threshold', Number(e.currentTarget.value))} /><span class="num mono">{Number(v('voice.match_threshold')).toFixed(2)}</span></div>
        </div>
      {/if}
    </section>

    <section class="group" aria-label="Try voice commands">
      <Head title="Try voice commands" text="Practise select all, copy, paste, undo and save, out loud.">
        <button type="button" class="quiet" aria-expanded={tryOpen} aria-controls="try-body" onclick={() => (tryOpen = !tryOpen)}>{tryOpen ? 'Close' : 'Open'}</button>
      </Head>
      {#if tryOpen}<div id="try-body" class="trybody"><TryCommands /></div>{/if}
    </section>

    {#if problem}<span class="problem" role="alert">{problem}</span>{/if}
  </div>
  {:else if part === 'talk'}
  <!-- a small corner status: ready, the button held, the microphone in use. The gear opens the microphone choice and a way to the settings, where the button is chosen. -->
  <div class="tbar" bind:this={talkBox} inert={locked} role="group" aria-label="Hold to talk">
    <span class="tb-state lab" title="{talk[0]}. {talk[1]}"><span class="sdot" class:ok={status.engine?.voice === 'ready'} role="img" aria-label={talk[0]}></span>Hold to talk</span>
    <span class="tiles" aria-label="Held to talk">
      {#each talkButtons as i}
        {@const parts = keyParts(i)}
        <span class="tile on" style={paint(i, true)} title={parts.length ? parts.join('+') : `Button ${i + 1}`}>
          <span class="num mono">0{i + 1}</span>
          {#if parts.length}<span class="chord">{#each parts as k, j}{#if j}<span class="pl">+</span>{/if}<kbd class="mono">{k}</kbd>{/each}</span>{/if}
        </span>
      {/each}
    </span>
    <span class="tb-mic" title={label('voice.microphone')}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM6 11a6 6 0 0 0 12 0M12 17v4" /></svg>
      <span class="t">{label('voice.microphone') || 'Default microphone'}</span>
    </span>
    <button type="button" class="gear" aria-label="Change the microphone or the talk button" aria-expanded={talkOpen} aria-haspopup="dialog" title="Change the microphone or the talk button" onclick={() => (talkOpen = !talkOpen)}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" /></svg>
    </button>
    {#if talkOpen}
      <div class="pop" role="dialog" aria-label="Microphone and talk button">
        <span class="lab">Microphone</span>
        <Select value={v('voice.microphone')} options={opts('voice.microphone')} label="Microphone" onchange={(x) => set('voice.microphone', x)} />
        <button type="button" class="more" onclick={() => { talkOpen = false; openSettings('voice'); }}>Change the talk button in Settings →</button>
      </div>
    {/if}
    {#if problem}<span class="problem" role="alert">{problem}</span>{/if}
  </div>
  {:else}
  <div class="voice main" inert={locked}>

{@render listCard('words')}
    {@render listCard('repl')}
    {#if problem}<span class="problem" role="alert">{problem}</span>{/if}
  </div>
  {/if}
  {:else if part !== 'talk'}
    <p class="hint" role="status">{problem || 'Loading the voice settings…'}</p>
  {/if}
</div>

<style>
  /* fills the section on a normal screen; a short window keeps this height and scrolls */
  .vwrap { container-type: inline-size; container-name: voice; flex: 1 1 0; min-height: 480px; display: flex; flex-direction: column; min-width: 0; }
  /* Voice section: the talk button beside the dictionary, filling the screen */
  .voice.main { flex: 1; min-height: 420px; display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); grid-template-rows: minmax(0, 1fr) auto; gap: 16px; min-width: 0; align-items: stretch; }
  /* the talk picker beside the section title: three tiles (lit = held to talk), each with its key, and the microphone */
  /* the talk status in the corner beside the Voice title: small, no box. The coloured key and the green dot say what is set; the gear opens the rest. */
  .tbar { position: relative; display: flex; align-items: center; gap: 10px; max-width: 100%; }
  .tb-state { display: flex; align-items: center; gap: 7px; font-size: 10px !important; letter-spacing: .1em; white-space: nowrap; }
  .sdot { width: 7px; height: 7px; border-radius: 50%; background: var(--warn); }
  .sdot.ok { background: var(--ok); }
  .tb-mic { display: flex; align-items: center; gap: 5px; min-width: 0; max-width: 200px; font-size: 12px; color: var(--muted); }
  .tb-mic svg { width: 14px; height: 14px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
  .tb-mic .t { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tbar .tile { height: 30px; padding: 0 8px 0 4px; gap: 6px; }
  .tbar .tile .num { width: 22px; height: 22px; font-size: 10px; }
  .gear { width: 28px; height: 28px; flex: none; display: flex; align-items: center; justify-content: center; padding: 0; border-radius: 50%; border: 0; background: transparent; color: var(--muted); }
  .gear:hover, .gear[aria-expanded='true'] { background: var(--hair); color: var(--ink); }
  .gear svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
  .pop { position: absolute; top: calc(100% + 8px); right: 0; z-index: 30; width: 300px; max-width: 90vw; display: flex; flex-direction: column; gap: 8px; padding: 14px; border: 1px solid var(--line); border-radius: var(--r-card); background: var(--surface); box-shadow: 0 14px 36px var(--shadow); }
  .pop .more { align-self: flex-end; margin-top: 4px; padding: 4px 2px; border: 0; background: transparent; color: var(--ink); font-size: 12px; font-weight: 600; text-decoration: underline; text-underline-offset: 3px; }
  .tiles { display: flex; gap: 6px; }
  .tile { display: flex; align-items: center; gap: 8px; height: 40px; padding: 0 12px 0 6px; border-radius: 10px; border: 1px solid var(--line); background: var(--surface); color: var(--muted); box-shadow: 0 3px 0 var(--line);
    transition: background-color .18s var(--ease), color .18s var(--ease), border-color .18s var(--ease), box-shadow .18s var(--ease); }
  .tile .num { width: 28px; height: 28px; flex: none; border-radius: 7px; display: grid; place-items: center; font-size: 11px; background: var(--hair); color: var(--muted); }
  .tile.on { border-color: var(--e); background: color-mix(in srgb, var(--c) 12%, var(--surface)); color: var(--ink); box-shadow: 0 3px 0 var(--e); }
  .tile.on .num { background: var(--c); color: #fff; }
  .tile:not(.on):hover { border-color: var(--muted); color: var(--ink); }
  .chord { display: flex; align-items: center; gap: 3px; white-space: nowrap; }
  .chord kbd { font-size: 11px; font-weight: 600; padding: 2px 6px; border-radius: 5px; border: 1px solid var(--line); background: var(--sunk); color: inherit; }
  .chord .pl { font-size: 10px; color: var(--muted); }
  .voice.main > .problem { grid-column: 1 / -1; }
  /* Settings: stacked groups, a small heading and rows divided by hairlines, as in Pad & keys */
  .voice.set { display: flex; flex-direction: column; gap: 14px; min-width: 0; max-width: 860px; }
  .group { padding: 4px 20px 6px; border: 1px solid var(--line); border-radius: var(--r-card); background: var(--surface); box-shadow: 0 1px 3px var(--shadow); }
  .srow { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: center; gap: 6px 24px; padding: 13px 0; border-top: 1px solid var(--hair); }
  .rname { font-size: 14px; font-weight: 600; color: var(--ink); }
  .srow .hint { margin-top: 2px; }
  .ctl { display: flex; align-items: center; justify-content: flex-end; gap: 10px; min-width: 0; }
  .ctl.sel { width: 280px; }
  .ctl.key input { width: 170px; }
  .ctl input[type='range'] { width: 170px; }
  .num { min-width: 2.8em; text-align: right; font-size: 12px; color: var(--muted); }
  .rows.off { opacity: .45; }
  .trybody { padding: 4px 0 14px; }
  .ask { display: flex; align-items: center; justify-content: space-between; gap: 14px; flex-wrap: wrap; margin: 4px 0 10px; padding: 12px 14px; border-radius: var(--r-field); background: var(--sunk); }
  .btns { display: flex; gap: 8px; }
  .seg { min-width: 250px; }
  /* beside the title: as big as its content, nothing more */
  .vwrap.inline { container-type: normal; flex: none; min-height: 0; }
  .mono { font-family: var(--mono); }
  .lab { font: 500 11px/1.3 var(--mono); letter-spacing: .1em; text-transform: uppercase; color: var(--muted); }
  .hint { margin: 0; font-size: 12px; line-height: 1.45; color: var(--muted); }
  .pad { padding: 12px 4px; }
  button:disabled { opacity: .4; cursor: default; }
  input:not([type='range']) { height: 36px; min-width: 0; border-radius: var(--r-field); border: 1.5px solid var(--line); background: var(--surface); color: var(--ink); padding: 0 12px; font-size: 13px; outline: none; }
  input:not([type='range']):focus-visible { border-color: var(--ink); }

  /* ---- cards (the dictionary and fragments) */
  .card { border: 1px solid var(--line); background: var(--surface); border-radius: var(--r-card); padding: 16px 18px; display: flex; flex-direction: column; gap: 12px; min-width: 0; min-height: 0; overflow-y: auto; scrollbar-width: thin; }
  .card > * { flex-shrink: 0; }
  .chead { display: flex; align-items: center; justify-content: space-between; gap: 10px; min-height: 32px; }
  .seg { display: flex; gap: 2px; padding: 3px; border-radius: var(--r-pill); background: var(--hair); }
  .seg button { flex: 1; height: 30px; padding: 0 8px; border: 0; border-radius: var(--r-pill); background: transparent; color: var(--muted); font-size: 12px; font-weight: 600; white-space: nowrap; }
  .seg button.on { background: var(--surface); color: var(--ink); box-shadow: var(--shadow-chosen); }
  .ask { display: flex; flex-direction: column; gap: 8px; padding: 10px; border-radius: var(--r-field); background: var(--sunk); }
  .key { display: flex; gap: 6px; }
  .key input { flex: 1; }
  .quiet { height: 32px; padding: 0 12px; border-radius: var(--r-pill); border: 1px solid var(--line); background: transparent; color: var(--ink); font-size: 12px; font-weight: 600; white-space: nowrap; }
  .dark { height: 36px; padding: 0 16px; border-radius: var(--r-pill); border: 0; background: var(--ink); color: var(--inv); font: 500 12px/1 var(--mono); white-space: nowrap; }

  /* ---- dictionary and the rest */
  .cnt { color: var(--ink); }
  .search { width: 160px; border-radius: var(--r-pill) !important; font-family: var(--mono); font-size: 12px !important; }
  /* the two boxes side by side, the button under them, so a narrow card still fits */
  .add { display: grid; grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr); align-items: center; gap: 8px; }
  .add1 { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .add1 .main { flex: 1 1 140px; min-width: 0; }
  .add1 .more { flex: none; height: 36px; padding: 0 14px; font-size: 13px; }
  .add1 .addbtns { grid-column: auto; flex: none; }
  .more { align-self: flex-start; display: inline-flex; align-items: center; gap: 6px; border: 1px dashed var(--line); border-radius: 999px; background: transparent; padding: 5px 12px; color: var(--muted); font: inherit; font-size: 12px; font-weight: 500; cursor: pointer; }
  .more b { font-size: 14px; line-height: 1; font-weight: 500; }
  .more:hover, .more:focus-visible { background: var(--sunk); color: var(--ink); border-color: var(--muted); }
  .addbtns { grid-column: 1 / -1; display: flex; gap: 8px; align-items: center; justify-content: flex-end; }
  .to { color: var(--faint); flex: none; }
  .words { flex: 1; min-height: 0; overflow-y: auto; border-top: 1px solid var(--line); display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); align-content: start; column-gap: 18px; }
  .words.full { grid-template-columns: minmax(0, 1fr); border-top: 0; }
  .words.fit { flex: none; overflow: visible; }
  /* the card scrolls as a whole; the fragments list grows inside it instead of a second scrollbar */
  .card .words { flex: none; overflow: visible; }
  /* corrected words fill the room left in the card; what does not fit is behind the dots */
  .card .words.clip { flex: 1 1 0; min-height: 110px; overflow: hidden; }
  .subrow { display: flex; align-items: center; justify-content: space-between; }
  .subrow .sub { padding: 12px 0 6px; }
  .viewall { display: inline-flex; align-items: center; gap: 4px; border: 0; background: transparent; padding: 2px 0; color: var(--ink); font: 600 12px/1 var(--mono); cursor: pointer; text-decoration: underline; text-decoration-color: var(--line); text-underline-offset: 4px; }
  .viewall svg { width: 13px; height: 13px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; transition: transform .15s var(--ease); }
  .viewall:hover, .viewall:focus-visible { text-decoration-color: var(--ink); }
  .viewall:hover svg { transform: translate(1px, -1px); }
  .chips { display: flex; flex-wrap: wrap; gap: 8px; padding: 10px; border: 1px solid var(--line); border-radius: 12px; }
  .chip { display: inline-flex; align-items: center; border: 1px solid var(--line); border-radius: 999px; background: var(--sunk); max-width: 100%; }
  .chip.sel { border-color: var(--ink); }
  .chip .pick { border: 0; background: transparent; color: var(--ink); font: inherit; font-size: 13px; font-weight: 600; padding: 5px 4px 5px 12px; cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .chip .del { border: 0; background: transparent; color: var(--muted); width: 26px; height: 26px; padding: 0; cursor: pointer; display: grid; place-items: center; }
  .chip .del svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 2; }
  .chip .del:hover { color: var(--ink); }
  .scrim { position: fixed; inset: 0; z-index: 50; display: grid; place-items: center; padding: 16px; background: rgba(0, 0, 0, .35); }
  .dpop { width: min(720px, 100%); max-height: min(640px, 100%); display: flex; flex-direction: column; gap: 12px; padding: 20px; border-radius: var(--r-bar); background: var(--surface); box-shadow: 0 12px 40px var(--shadow); outline: none; }
  .dpop .top { display: flex; align-items: center; justify-content: space-between; }
  .dpop .x { border: 0; background: transparent; color: var(--ink); width: 28px; height: 28px; cursor: pointer; }
  .dpop .x svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 2; }
  .dpop .search.wide { width: 100%; }
  .dpop .scroll { overflow-y: auto; min-height: 0; }
  .sub { font: 500 11px/1.3 var(--mono); letter-spacing: .1em; text-transform: uppercase; color: var(--muted); padding: 12px 0 6px; }
  .sub .cnt { opacity: .7; }
  .fhead { display: grid; grid-template-columns: minmax(0, 2fr) 18px minmax(0, 3fr) 24px; gap: 10px; padding: 0 4px 6px; border-bottom: 1px solid var(--line); font: 500 10px/1.3 var(--mono); letter-spacing: .1em; text-transform: uppercase; color: var(--muted); }
  .fhead span:last-child { grid-column: 3; }
  .word { display: flex; align-items: center; gap: 6px; border-bottom: 1px solid var(--hair); }
  .word.sel { background: var(--sunk); border-radius: 6px; }
  .word .pick { flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; padding: 10px 4px; border: 0; background: transparent; color: var(--ink); text-align: left; }
  .word .h { flex: 0 1 auto; min-width: 0; font-size: 12px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .word .w { flex: 0 0 auto; max-width: 60%; font-size: 13px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .full .pick { display: grid; grid-template-columns: minmax(0, 2fr) 18px minmax(0, 3fr); align-items: start; gap: 10px; padding: 12px 4px; }
  .full .h, .full .w { max-width: none; white-space: normal; overflow: visible; overflow-wrap: anywhere; line-height: 1.4; }
  .full .h { font-size: 13px; color: var(--ink); }
  .full .w { font-weight: 500; }
  .full .word { align-items: flex-start; }
  .full .del { margin-top: 10px; }
  .del { width: 24px; height: 24px; flex: none; border-radius: 50%; border: 0; background: transparent; color: var(--muted); display: flex; align-items: center; justify-content: center; padding: 0; }
  .del:hover { color: var(--bad); }
  .del svg { width: 12px; height: 12px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; }
  .saved { font-size: 12px; font-weight: 600; color: var(--ok); white-space: nowrap; }
  .problem { font-size: 12px; color: var(--bad); }
  input[type='range'] { width: 100%; accent-color: var(--signal); }

  @container voice (max-width: 820px) { .words { grid-template-columns: minmax(0, 1fr); } }
  @container voice (max-width: 620px) { .voice.main { flex: none; min-height: 0; grid-template-columns: 1fr; grid-template-rows: auto; } .words { max-height: 280px; } }
  @container voice (max-width: 560px) { .add { flex-wrap: wrap; } }
  @media (max-width: 760px) { .srow { grid-template-columns: minmax(0, 1fr); } .ctl, .ctl.sel { width: auto; justify-content: flex-start; } .seg { min-width: 0; } }
</style>
