<script>
  import Head from './Head.svelte';
  import Ico from './Ico.svelte';
  import PrivacyNotice from './PrivacyNotice.svelte';
  import { settingsUi } from '../settings.svelte.js';

  const invoke = (/** @type {string} */ c, /** @type {any} */ a = undefined) => window.__TAURI__.core.invoke(c, a);
  let usage = $state(false);
  let adult = $state(false);
  let problem = $state('');

  async function load() {
    try { usage = (await invoke('privacy_status')).usage; } catch (e) { problem = String(e); }
  }
  async function choose(/** @type {boolean} */ on) {
    try { usage = (await invoke('privacy_choose', { usage: on, adult })).usage; adult = false; problem = ''; } catch (e) { problem = String(e); }
  }
  const policy = () => invoke('open_privacy_policy').catch(() => {});
  // Settings is built at start, before the first-run question: read the answer again on every open.
  $effect(() => { if (settingsUi.open) load(); });
</script>

<section class="card">
  <Head title="Help improve OASIS" text="Optional. Off until you turn it on." />
  {#if usage}
    <div class="srow">
      <Ico n="chart" />
      <div class="rtext">
        <div class="rname">On: sending three numbers a day</div>
        <div class="hint">Words dictated, dictations made, and dictations you corrected by hand. Nothing else. Turning it off stops the counting at once and deletes the counts kept on this PC.</div>
      </div>
      <div class="ctl"><button class="action quiet" onclick={() => choose(false)}>Turn off</button></div>
    </div>
    <button class="link" type="button" onclick={policy}>Read the full privacy policy</button>
  {:else}
    <PrivacyNotice bind:adult />
    <div class="turn"><button class="action" onclick={() => choose(true)} disabled={!adult}>Turn on</button></div>
  {/if}
  <div class="msg bad" role="status">{problem}</div>
</section>

<style>
  .card :global(.label) { margin-bottom: 20px; }
  .turn { margin-top: 24px; }
  .msg:empty { display: none; }
  .link { margin-top: 14px; padding: 0; border: 0; background: none; color: var(--signal); text-decoration: underline; text-underline-offset: 3px; font: inherit; font-size: 13.5px; cursor: pointer; }
</style>
