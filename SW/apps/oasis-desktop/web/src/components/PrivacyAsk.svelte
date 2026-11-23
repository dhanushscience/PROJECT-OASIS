<script>
  import { onMount, tick } from 'svelte';
  import { guide } from '../guide/onboarding.svelte.js';
  import PrivacyNotice from './PrivacyNotice.svelte';

  // Asked once after Getting Started (and again if the wording changes). Both answers end the question.
  const tauri = /** @type {any} */ (window).__TAURI__;
  let answered = $state(true); // true until we know, so it never flashes
  let open = $state(false);
  let adult = $state(false);
  let problem = $state('');
  /** @type {HTMLButtonElement} */
  let noBtn;

  onMount(async () => {
    try { answered = (await tauri.core.invoke('privacy_status')).answered; } catch { /* stays hidden until the settings can be read */ }
  });

  $effect(() => {
    if (open || answered || !guide.loaded || (guide.status !== 'dismissed' && guide.status !== 'complete')) return;
    open = true;
    tick().then(() => noBtn?.focus());
  });

  async function choose(/** @type {boolean} */ usage) {
    try {
      await tauri.core.invoke('privacy_choose', { usage, adult });
      answered = true;
      open = false;
      problem = '';
    } catch (e) { problem = String(e); } // asked again next time if it could not be saved
  }
</script>

<svelte:window onkeydowncapture={(e) => { if (open && e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); choose(false); } }} />

<div class="scrim" class:open inert={!open} role="presentation">
  <div class="box" role="dialog" aria-modal="true" aria-labelledby="pa-title">
    <h2 id="pa-title">Help me improve OASIS?</h2>
    <PrivacyNotice bind:adult />
    {#if problem}<p class="bad" role="status">{problem}</p>{/if}
    <div class="btns">
      <button bind:this={noBtn} class="later" onclick={() => choose(false)}>No thanks</button>
      <button class="go" onclick={() => choose(true)} disabled={!adult}>Yes, send these three numbers</button>
    </div>
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: 195; display: flex; align-items: center; justify-content: center; padding: 24px; background: rgb(20 20 24 / .45);
    opacity: 0; visibility: hidden; transition: opacity .18s var(--ease), visibility 0s linear .18s; }
  .scrim.open { opacity: 1; visibility: visible; transition-delay: 0s; }
  .box { width: min(780px, 100%); max-height: 100%; overflow: auto; padding: 26px 30px; background: var(--surface); border: 1px solid var(--line); border-radius: var(--r-panel); box-shadow: 0 24px 60px var(--shadow); }
  h2 { margin: 0 0 14px; font-size: 12px; font-weight: 650; letter-spacing: .06em; text-transform: uppercase; color: var(--muted); }
  .bad { margin: 12px 0 0; font-size: 13px; color: #b3261e; }
  .btns { display: flex; justify-content: flex-end; gap: 10px; margin-top: 20px; }
  .later { border: 1px solid var(--line); background: transparent; color: var(--ink); }
  .go { border: 0; background: var(--signal); color: var(--on-signal); }
  button { height: 38px; padding: 0 18px; border-radius: var(--r-pill); font-size: 13px; font-weight: 600; }
  button:disabled { opacity: .45; cursor: not-allowed; }
</style>
