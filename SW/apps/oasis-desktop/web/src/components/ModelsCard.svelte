<script>
  import Head from './Head.svelte';
  import Ico from './Ico.svelte';
  import { models, modelsCall, etaText, setName, upgradeText } from '../models.svelte.js';
  import { sure } from '../confirm.svelte.js';

  const info = $derived(models.info);
  const percent = $derived(info && info.total_mb ? Math.min(100, Math.round((info.done_mb / info.total_mb) * 100)) : 0);
  const busy = $derived(info?.phase === 'downloading');

  const line = $derived.by(() => {
    if (!info) return 'Checking this PC…';
    const set = setName(info.target);
    switch (info.phase) {
      case 'checking': return 'Checking this PC…';
      case 'downloading': return `Downloading ${set}: ${info.done_mb} of ${info.total_mb} MB${info.eta_s !== null ? `, ${etaText(info.eta_s)}` : ''}.`;
      case 'paused': return `${set} is paused at ${info.done_mb} of ${info.total_mb} MB.`;
      case 'metered': return `${set} waits: this is a metered connection. It downloads on an ordinary one, or now if you choose.`;
      case 'offline': return `${set} waits for a connection${info.error ? ` (${info.error})` : ''}. OASIS tries again every few minutes.`;
      case 'failed': return info.error ?? 'The download failed.';
      case 'cleaning': return 'Clearing old voice models before the update…';
      case 'testing': return `Testing ${set} on this PC (about 20 seconds)…`;
      case 'ready': return `${set} is in use.`;
      default: return info.choice === 'none' ? 'No voice models are downloaded. Choose a set to download one.' : 'No set fits this PC. Choose one below, or use Groq (online) in Voice settings.';
    }
  });

  /** What each set does to your voice, for the pictures: words wrong per 100 (VOICE_BENCH.md §3: 4.6% and 0.9%),
      where it runs, and whether S1-mini polishes the text. Balanced and Quality share the speech model. */
  const PROFILE = /** @type {Record<string, { model: string, wrong: number, runs: string, polish: boolean }>} */ ({
    light: { model: 'Small model', wrong: 5, runs: 'Processor', polish: false },
    balanced: { model: 'Accurate model', wrong: 1, runs: 'Processor', polish: false },
    quality: { model: 'Accurate model', wrong: 1, runs: 'Processor + graphics card', polish: true },
  });
  const cur = $derived(info?.target ? PROFILE[info.target] : undefined);
  const curSet = $derived(info?.sets.find((x) => x.id === info?.target));

  /** @param {string} choice */
  function choose(choice) {
    if (info && choice !== info.choice) modelsCall('models_choose', { choice });
  }
  async function remove() {
    if (await sure({ title: 'Remove downloaded models?', body: 'Voice stops until a set is downloaded again.', yes: 'Remove', keep: 'Keep them' })) modelsCall('models_remove');
  }
</script>

<!-- what is in use first, then the choice, then upkeep; the PC facts behind the choice are not shown -->
<section class="group" id="voice-models" aria-labelledby="models-head">
  <Head id="models-head" title="In use">
    {#if busy}
      <button class="pillbtn" onclick={() => modelsCall('models_pause')}>Pause</button>
    {:else if info?.phase === 'paused' || info?.phase === 'offline' || info?.phase === 'failed'}
      <button class="pillbtn dark" onclick={() => modelsCall('models_resume')}>{info.phase === 'paused' ? 'Resume' : 'Try again'}</button>
    {:else if info?.phase === 'metered'}
      <button class="pillbtn dark" onclick={() => modelsCall('models_download_anyway')}>Download anyway</button>
    {/if}
  </Head>
  <div class="inuse">
    <div class="who">
      <div class="big">{setName(info?.target) || 'None'}</div>
      <div class="hint" role="status">{line}</div>
      {#if curSet}<p class="what">{curSet.what}</p>{/if}
    </div>
    {#if cur}
      <div class="how">
        <!-- what happens to a recording with this set: the steps it goes through, left to right -->
        <ol class="flow" aria-label="What happens to your voice">
          <li><Ico n="mic" /><span>You speak</span></li>
          <li class="lit"><Ico n="waves" /><span>{cur.model}</span></li>
          <li class:lit={cur.polish} class:off={!cur.polish}><Ico n="sparkle" /><span>{cur.polish ? 'Polished' : 'No polish'}</span></li>
          <li><Ico n="text" /><span>Typed</span></li>
        </ol>
        <div class="miss">
          <span class="dots" aria-hidden="true">{#each { length: 100 } as _, i}<i class:bad={i < cur.wrong}></i>{/each}</span>
          <span class="hint">About {cur.wrong} in 100 words wrong</span>
        </div>
        <div class="facts"><span class="chip"><Ico n="chip" />{cur.runs}</span>{#if curSet}<span class="chip">{curSet.size_mb} MB</span>{/if}</div>
      </div>
    {/if}
  </div>
  {#if busy || info?.phase === 'paused'}
    <div class="meter" role="progressbar" aria-label="Voice model download" aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent}><span style="width:{percent}%"></span></div>
  {/if}
  {#if info?.upgrade}
    <!-- Basic in use on a PC that can run Balanced: one click switches, the same way as choosing Balanced below -->
    <div class="srow">
      <Ico n="up" />
      <div class="rtext"><div class="rname">Basic makes more mistakes</div>
        <div class="hint">This PC can run Balanced, which gets about five times fewer words wrong.</div></div>
      <button class="pillbtn dark" onclick={() => choose('balanced')}>{upgradeText(info)}</button>
    </div>
  {/if}
</section>

{#if info}
  <section class="group" aria-label="Model set">
    <Head title="Change model" text="Faster or more accurate." />
    <div class="sets" role="radiogroup" aria-label="Model set">
      <label class="set" class:on={info.choice === 'auto'}>
        <input type="radio" name="model-set" value="auto" checked={info.choice === 'auto'} onchange={() => choose('auto')} />
        <span class="radio" aria-hidden="true"></span>
        <span class="body">
          <span class="name">Automatic{info.recommended ? ` · ${setName(info.recommended)}` : ''}<span class="tag">Recommended</span></span>
          <span class="desc">The set this PC suits, from a speed check.</span>
        </span>
      </label>
      {#each info.sets as set (set.id)}
        <label class="set" class:on={info.choice === set.id}>
          <input type="radio" name="model-set" value={set.id} checked={info.choice === set.id} onchange={() => choose(set.id)} />
          <span class="radio" aria-hidden="true"></span>
          <span class="body">
            <span class="name">{set.label}<span class="size">{set.size_mb} MB</span>{#if set.installed}<span class="tag dl">Downloaded</span>{/if}</span>
            <span class="desc">{set.what}</span>
            {#if PROFILE[set.id]}
              {@const f = PROFILE[set.id]}
              <span class="mini"><span class="chip">≈{f.wrong} in 100 wrong</span><span class="chip">{f.runs}</span>{#if f.polish}<span class="chip">Polish</span>{/if}</span>
            {/if}
            {#if !set.suits}<span class="warn">This PC is below what this set needs. It still works, but may be slow.</span>{/if}
          </span>
        </label>
      {/each}
    </div>
  </section>

  <section class="group" aria-label="Manage models">
    <Head title="Manage" />
    <div class="srow">
      <Ico n="gauge" />
      <div class="rtext"><div class="rname">Check this PC again</div><div class="hint">After a driver or hardware change.</div></div>
      <button class="pillbtn" onclick={() => modelsCall('models_check_again')}>Check again</button>
    </div>
    <div class="srow">
      <Ico n="trash" />
      <div class="rtext"><div class="rname">Remove downloaded models</div>
        <div class="hint">Frees disk space. Download again any time.</div></div>
      <button class="pillbtn" onclick={remove} disabled={!info.sets.some((s) => s.installed)}>Remove…</button>
    </div>
  </section>
{/if}

<style>
  /* in use: the name and what it is on the left, how it treats your voice on the right */
  .inuse { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1.25fr); gap: 20px 28px; align-items: start; padding: 4px 0 16px; }
  .big { font-size: 24px; font-weight: 700; letter-spacing: -.02em; color: var(--ink); }
  .what { margin: 10px 0 0; font-size: 13px; line-height: 1.5; color: var(--muted); }
  .how { display: flex; flex-direction: column; gap: 14px; padding: 14px 16px; border-radius: var(--r-row); background: var(--sunk); }
  .flow { margin: 0; padding: 0; list-style: none; display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 6px; }
  .flow li { position: relative; display: flex; flex-direction: column; align-items: center; gap: 6px; text-align: center; font-size: 11px; line-height: 1.25; color: var(--muted); }
  .flow li + li::before { content: ''; position: absolute; top: 16px; right: calc(50% + 22px); width: calc(100% - 38px); border-top: 1.5px dashed var(--line); }
  .flow :global(.ico) { background: var(--surface); box-shadow: 0 2px 0 var(--line); }
  .flow li.lit { color: var(--ink); font-weight: 600; }
  .flow li.lit :global(.ico) { background: color-mix(in srgb, var(--signal) 14%, var(--surface)); color: var(--signal-ink); box-shadow: 0 2px 0 color-mix(in srgb, var(--signal) 55%, var(--line)); }
  .flow li.off { opacity: .45; }
  .flow li.off span { text-decoration: line-through; }
  .miss { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; }
  .dots { display: grid; grid-template-columns: repeat(25, 5px); gap: 3px; }
  .dots i { width: 5px; height: 5px; border-radius: 50%; background: var(--line); }
  .dots i.bad { background: var(--signal); }
  .facts { display: flex; gap: 6px; flex-wrap: wrap; }
  .facts .chip { gap: 6px; }
  .facts :global(.ico) { width: 16px; height: 16px; background: none; }
  .facts :global(.ico svg) { width: 14px; height: 14px; }
  .mini { display: flex; gap: 6px; flex-wrap: wrap; margin-top: 4px; }
  .mini .chip { height: 22px; font-size: 11px; }
  @media (max-width: 760px) { .inuse { grid-template-columns: minmax(0, 1fr); } }
  .meter { height: 6px; margin: 4px 0 12px; border-radius: 3px; background: var(--hair); overflow: hidden; }
  .meter span { display: block; height: 100%; background: var(--signal); transition: width .25s var(--ease); }

  /* the sets: a list; the chosen one is lifted with a dark ring, and its radio fills in orange */
  .sets { display: flex; flex-direction: column; margin: 0 -8px 8px; }
  .set { position: relative; display: flex; gap: 14px; align-items: flex-start; padding: 13px 12px; border-radius: var(--r-row); border: 1.5px solid transparent; cursor: pointer; transition: border-color .2s var(--ease), background-color .2s var(--ease); }
  .set + .set { border-top-color: var(--hair); }
  .set:hover { background: var(--sunk); }
  .set.on { border-color: var(--ink); background: var(--sunk); }
  .set + .set.on, .set.on + .set { border-top-color: transparent; }
  .set input { position: absolute; opacity: 0; inset: 0; margin: 0; cursor: pointer; }
  .radio { flex: none; width: 18px; height: 18px; margin-top: 2px; border-radius: 50%; border: 1.5px solid var(--line); background: var(--surface); transition: border-color .2s var(--ease), box-shadow .2s var(--ease); }
  .set.on .radio { border-color: var(--ink); box-shadow: inset 0 0 0 4px var(--surface), inset 0 0 0 9px var(--signal); }
  .set:has(input:focus-visible) { outline: 2px solid var(--signal); outline-offset: 2px; }
  .body { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .name { font-size: 14px; font-weight: 600; display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .size { font: 500 11px/1 var(--mono); color: var(--muted); }
  .tag { font: 600 10px/1 var(--mono); letter-spacing: .06em; text-transform: uppercase; padding: 4px 7px; border-radius: var(--r-pill); background: var(--hair); color: var(--ink); }
  .tag.dl { background: var(--ink); color: var(--inv); }
  .desc { font-size: 13px; color: var(--muted); line-height: 1.45; }
  .warn { font-size: 12px; color: var(--signal-ink); line-height: 1.4; }
  @media (prefers-reduced-motion: reduce) { .meter span, .set, .radio { transition: none; } }
</style>
