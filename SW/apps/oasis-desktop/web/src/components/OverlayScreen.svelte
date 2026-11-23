<script>
  import Head from './Head.svelte';
  import Ico from './Ico.svelte';
  import { status } from '../status.svelte.js';
  import Switch from './Switch.svelte';

  const { invoke } = window.__TAURI__?.core ?? { invoke: async () => { throw new Error('Not running inside OASIS.'); } };

  /** @typedef {{ id: string, label: string, hint: string, kind: string, value: any, min: number | null, max: number | null, step: number | null, options: { value: any, label: string }[] }} Field */

  let fields = $state(/** @type {Record<string, Field>} */ ({}));
  /** the look as the overlay itself reads it (colours checked, sizes clamped), for the preview */
  let look = $state(/** @type {any} */ (null));
  let loaded = $state(false);
  let problem = $state('');
  let moving = $state(false);
  /** the preview shows the overlay in use (labels out) or at rest */
  let inUse = $state(true);
  const ready = $derived(!!status.engine);
  const locked = $derived(!!status.engine && !status.engine.integrity_ok);

  /** @param {{ sections: { id: string, fields: Field[] }[] }} reply */
  async function take(reply) {
    const all = reply.sections.flatMap((s) => s.fields);
    fields = Object.fromEntries(all.filter((f) => f.id.startsWith('overlay.') || f.id === 'general.show_labels').map((f) => [f.id, f]));
    look = await invoke('overlay_look');
    loaded = true;
  }
  async function load() { try { await take(await invoke('settings_fields')); } catch (e) { problem = String(e); } }
  $effect(() => { if (ready && !loaded) load(); });

  async function set(/** @type {string} */ id, /** @type {any} */ value) {
    problem = '';
    try { await take(await invoke('set_setting', { id, value })); } catch (e) { problem = String(e); }
  }
  async function resetLook() {
    problem = '';
    try { await take(await invoke('reset_overlay_look_setting')); } catch (e) { problem = String(e); }
  }
  async function toggleMove() {
    try { moving = await invoke('move_overlay'); if (!moving) await load(); } catch (e) { problem = String(e); }
  }
  const v = (/** @type {string} */ id) => fields[id]?.value;

  // ---- arrangement: vertical (an edge) or horizontal (a row, centred)
  const dock = $derived(String(v('overlay.dock') ?? 'right'));
  const vertical = $derived(dock === 'right' || dock === 'left');
  /** Keeps the same side of the screen where it can: right ↔ bottom, left ↔ top. The voice pop-up follows the layout. */
  const arrange = async (/** @type {boolean} */ vert) => {
    if ((v('overlay.popup') === 'vertical') !== vert) await set('overlay.popup', vert ? 'vertical' : 'horizontal');
    if (vert !== vertical) set('overlay.dock', vert ? (dock === 'top' ? 'left' : 'right') : (dock === 'left' ? 'top' : 'bottom'));
  };
  const IDLE = [
    { value: 'faint', name: 'Faint', about: 'Small and light at the edge, like the old app. It comes into full colour when you press a button or talk. In a row it rests as one bar and spreads out to three.' },
    { value: 'hidden', name: 'Hidden', about: 'Out of sight until you press a button or talk, then it fades in.' },
    { value: 'always', name: 'Always full', about: 'Always shown in full colour.' },
  ];
  const idle = $derived(IDLE.find((i) => i.value === v('overlay.idle')) ?? IDLE[0]);

  // ---- the look
  const BAR_COLORS = ['overlay.indicator_color_1', 'overlay.indicator_color_2', 'overlay.indicator_color_3'];
  const BAR_NUMBERS = ['overlay.indicator_width', 'overlay.indicator_height', 'overlay.indicator_spacing', 'overlay.indicator_opacity'];
  const LABEL_COLORS = ['overlay.label_background', 'overlay.label_text', 'overlay.label_border'];
  const LABEL_NUMBERS = ['overlay.label_font_size', 'overlay.label_font_opacity', 'overlay.label_bubble_opacity', 'overlay.label_corner_radius'];
  const shown = (/** @type {Field} */ f) => (f.step && f.step < 1 ? Number(f.value).toFixed(2) : String(f.value));

  const SAMPLE = ['OPTION 1', 'OPTION 2', 'OPTION 3'];
  /** the preview at about half size */
  const k = 0.55;
</script>

{#snippet colors(/** @type {string[]} */ ids)}
  {#each ids as id}
    {#if fields[id]}
      <div class="srow"><Ico n="palette" /><div class="rtext"><div class="rname">{fields[id].label}</div></div>
        <span class="sw" role="radiogroup" aria-label={fields[id].label}>
          {#each fields[id].options as o}<button type="button" role="radio" aria-checked={o.value === v(id)} class:on={o.value === v(id)} title={o.label} aria-label={o.label} style="--c: {o.value}" onclick={() => set(id, o.value)}></button>{/each}
        </span></div>
    {/if}
  {/each}
{/snippet}

{#snippet sliders(/** @type {string[]} */ ids)}
  {#each ids as id}
    {#if fields[id]}
      <div class="srow"><Ico n="sliders" /><div class="rtext"><div class="rname">{fields[id].label}</div></div>
        <div class="ctl"><input type="range" aria-label={fields[id].label} min={fields[id].min} max={fields[id].max} step={fields[id].step} value={v(id)} onchange={(e) => set(id, Number(e.currentTarget.value))} /><span class="val mono">{shown(fields[id])}</span></div></div>
    {/if}
  {/each}
{/snippet}

<div class="owrap">
  {#if !ready}
    <p class="hint">Overlay settings appear here once OASIS is running.</p>
  {:else if loaded && look}
  <div class="ov" inert={locked}>
    <section class="group" aria-label="Arrangement">
      <Head title="Arrangement" text="Where the three bars sit on your screen.">
        <span class="acts">
          {#if look.moved && !moving}<button class="pillbtn" onclick={() => set('overlay.dock', dock)} title="Put it back on its edge">Snap back to edge</button>{/if}
          <button class="pillbtn" class:dark={moving} onclick={toggleMove}>{moving ? 'Done moving' : 'Move overlay'}</button>
        </span>
      </Head>
      <div class="srow">
        <Ico n="layout" />
        <div class="rtext"><div class="rname">Layout</div></div>
        <div class="seg" role="radiogroup" aria-label="Layout">
          <button type="button" role="radio" aria-checked={vertical} class:on={vertical} onclick={() => arrange(true)}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="5" y="4" width="2.5" height="4" rx="1.2" /><rect x="5" y="10" width="2.5" height="4" rx="1.2" /><rect x="5" y="16" width="2.5" height="4" rx="1.2" /><rect x="10" y="4.5" width="10" height="3" rx="1.5" /><rect x="10" y="10.5" width="10" height="3" rx="1.5" /><rect x="10" y="16.5" width="10" height="3" rx="1.5" /></svg>Vertical</button>
          <button type="button" role="radio" aria-checked={!vertical} class:on={!vertical} onclick={() => arrange(false)}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="2" y="8" width="6" height="4" rx="2" /><rect x="9" y="8" width="6" height="4" rx="2" /><rect x="16" y="8" width="6" height="4" rx="2" /><rect x="10" y="15" width="4" height="1.6" rx=".8" /></svg>Horizontal</button>
        </div>
      </div>
      <div class="srow">
        <Ico n="dock" />
        <div class="rtext"><div class="rname">{vertical ? 'Side of the screen' : 'Edge, centred'}</div></div>
        <div class="seg" role="radiogroup" aria-label="Where on the screen">
          {#each vertical ? [['left', 'Left'], ['right', 'Right']] : [['top', 'Top'], ['bottom', 'Bottom']] as [d, t]}
            <button type="button" role="radio" aria-checked={dock === d} class:on={dock === d && !look.moved} onclick={() => set('overlay.dock', d)}>{t}</button>
          {/each}
        </div>
      </div>
      <div class="srow">
        <Ico n="eye" />
        <div class="rtext"><div class="rname">When not in use</div><div class="hint">{idle.about}{moving ? ' Drag the overlay on your screen now, then press Done moving.' : ''}</div></div>
        <div class="seg" role="radiogroup" aria-label="When not in use">
          {#each IDLE as i}<button type="button" role="radio" aria-checked={idle.value === i.value} class:on={idle.value === i.value} onclick={() => set('overlay.idle', i.value)}>{i.name}</button>{/each}
        </div>
      </div>

      <!-- a small screen showing the overlay where it will sit -->
      <div class="pvrow">
        <div class="preview" class:rest={!inUse} data-dock={dock} data-idle={idle.value} role="img" aria-label="Preview of the overlay"
          style="--p1: {look.indicator_colors[0]}; --p2: {look.indicator_colors[1]}; --p3: {look.indicator_colors[2]};
            --pw: {look.indicator_width * k}px; --ph: {look.indicator_height * k}px; --gap: {look.indicator_spacing * k}px; --po: {look.indicator_opacity};
            --bb: {look.label_background}; --bt: {look.label_text}; --bd: {look.label_border}; --bo: {Math.round(look.label_bubble_opacity * 100)}%; --to: {Math.round(look.label_font_opacity * 100)}%;
            --fs: {Math.max(8, look.label_font_size * k)}px; --r: {look.label_corner_radius * k}px; --lg: {look.label_spacing * k}px">
          <div class="stack">
            {#each SAMPLE as s, i}
              <div class="row" class:press={inUse && i === 1}><span class="bubble">{s}</span><span class="bar"></span></div>
            {/each}
          </div>
          {#if look.moved}<span class="note mono">Dragged to its own spot · shown on its edge here</span>{/if}
          <div class="viewseg" role="radiogroup" aria-label="Preview">
            <button type="button" role="radio" aria-checked={inUse} class:on={inUse} onclick={() => (inUse = true)}>In use</button>
            <button type="button" role="radio" aria-checked={!inUse} class:on={!inUse} onclick={() => (inUse = false)}>At rest</button>
          </div>
        </div>
      </div>
    </section>

    <section class="group" aria-label="Bars">
      <Head title="Bars" text="Colour and size of each bar."><button class="pillbtn" onclick={resetLook} title="Colours and sizes back to the defaults; where it sits is kept">Reset look</button></Head>
      {@render colors(BAR_COLORS)}
      {@render sliders(BAR_NUMBERS)}
    </section>

    <section class="group" aria-label="Labels">
      <Head title="Labels" text="The names shown beside the bars." />
      {#if fields['general.show_labels']}
        <div class="srow"><Ico n="tag" /><div class="rtext"><div class="rname">Show labels on the first tap</div><div class="hint">The second tap runs the button.</div></div>
          <Switch checked={!!v('general.show_labels')} label="Show labels on the first tap" onchange={(on) => set('general.show_labels', on)} /></div>
      {/if}
      {@render colors(LABEL_COLORS)}
      {@render sliders(LABEL_NUMBERS)}
    </section>
    {#if problem}<span class="problem" role="alert">{problem}</span>{/if}
  </div>
  {/if}
</div>

<style>
  .ov { display: flex; flex-direction: column; gap: 14px; max-width: 860px; }
  .mono { font-family: var(--mono); }
  .hint { margin: 0; font-size: 12px; line-height: 1.45; color: var(--muted); }
  .acts { display: flex; gap: 8px; flex-wrap: wrap; }
  .problem { font-size: 12px; color: var(--bad); }
  .seg svg { width: 16px; height: 16px; fill: currentColor; }

  /* ---- the preview: a pale screen with the overlay where it will sit */
  .pvrow { padding: 12px 0 14px; border-top: 1px solid var(--hair); }
  .preview { position: relative; height: 240px; border-radius: 10px; overflow: hidden; background-color: var(--surface);
    background-image: linear-gradient(90deg, color-mix(in srgb, var(--ink) 6%, transparent) 1px, transparent 1px), linear-gradient(color-mix(in srgb, var(--ink) 6%, transparent) 1px, transparent 1px);
    background-size: 32px 32px; box-shadow: inset 0 -1px 6.1px rgb(0 0 0 / .25), inset 0 2px 4px rgb(0 0 0 / .25); }
  .stack { position: absolute; display: flex; gap: var(--gap); transition: opacity .25s var(--ease); }
  .row { display: flex; align-items: center; gap: var(--lg); }
  .bar { flex: none; width: var(--pw); height: var(--ph); border-radius: 999px; opacity: var(--po); transition: transform .25s var(--ease), opacity .25s var(--ease); }
  .row:nth-child(1) .bar { background: var(--p1); }
  .row:nth-child(2) .bar { background: var(--p2); }
  .row:nth-child(3) .bar { background: var(--p3); }
  .row.press .bar { background: var(--signal); }
  .bubble { padding: 3px 10px; border-radius: var(--r); white-space: nowrap; font: 600 var(--fs)/1.3 var(--mono); letter-spacing: .04em;
    background: color-mix(in srgb, var(--bb) var(--bo), transparent); color: color-mix(in srgb, var(--bt) var(--to), transparent);
    border: 1px solid color-mix(in srgb, var(--bd) var(--bo), transparent); transition: opacity .2s var(--ease); }
  .preview[data-dock='right'] .stack { right: 14px; top: 50%; transform: translateY(-50%); flex-direction: column; align-items: flex-end; }
  .preview[data-dock='left'] .stack { left: 14px; top: 50%; transform: translateY(-50%); flex-direction: column; align-items: flex-start; }
  .preview[data-dock='left'] .row { flex-direction: row-reverse; }
  .preview[data-dock='bottom'] .stack, .preview[data-dock='top'] .stack { left: 50%; transform: translateX(-50%); flex-direction: row; }
  .preview[data-dock='bottom'] .stack { bottom: 14px; align-items: flex-end; }
  .preview[data-dock='top'] .stack { top: 14px; align-items: flex-start; }
  .preview[data-dock='bottom'] .row { flex-direction: column; }
  .preview[data-dock='top'] .row { flex-direction: column-reverse; }
  .preview[data-dock='bottom'] .bar, .preview[data-dock='top'] .bar { width: var(--ph); height: var(--pw); }
  /* in a row: one bar at rest, spreading out to three when in use */
  .preview[data-dock='bottom'] .row, .preview[data-dock='top'] .row { width: 110px; transition: transform .38s var(--ease); }
  .preview.rest[data-dock='bottom'] .row:nth-child(1), .preview.rest[data-dock='top'] .row:nth-child(1) { transform: translateX(calc(110px + var(--gap))); }
  .preview.rest[data-dock='bottom'] .row:nth-child(3), .preview.rest[data-dock='top'] .row:nth-child(3) { transform: translateX(calc(-110px - var(--gap))); }
  .preview.rest[data-dock='bottom'] .row:not(:nth-child(2)) .bar, .preview.rest[data-dock='top'] .row:not(:nth-child(2)) .bar { opacity: 0; }
  /* at rest: labels away; faint bars shrink and fade, hidden ones go */
  .preview.rest .bubble { opacity: 0; }
  .preview.rest[data-idle='faint'] .bar { transform: scale(.6); opacity: calc(var(--po) * .3); }
  .preview.rest[data-idle='hidden'] .stack { opacity: 0; }
  .note { position: absolute; left: 12px; top: 10px; font-size: 10px; color: var(--muted); text-transform: uppercase; }
  .viewseg { position: absolute; left: 12px; bottom: 12px; display: flex; gap: 2px; padding: 3px; border-radius: var(--r-pill); background: color-mix(in srgb, var(--surface) 85%, transparent); box-shadow: 0 1px 4px var(--shadow); }
  .preview[data-dock='bottom'] .viewseg { bottom: auto; top: 12px; }
  .viewseg button { height: 24px; padding: 0 12px; border: 0; border-radius: var(--r-pill); background: transparent; color: var(--muted); font-size: 11px; font-weight: 600; }
  .viewseg button.on { background: var(--ink); color: var(--inv); }

  /* ---- colours and sizes */
  .val { min-width: 3em; font-size: 12px; color: var(--muted); text-align: right; }
  .sw { display: flex; gap: 7px; }
  .sw button { width: 24px; height: 24px; padding: 0; border-radius: 50%; background: var(--c); border: 1px solid color-mix(in srgb, var(--ink) 20%, transparent); }
  .sw button.on { box-shadow: 0 0 0 2px var(--surface), 0 0 0 4px var(--signal); }
  input[type='range'] { width: 190px; accent-color: var(--signal); }
</style>
