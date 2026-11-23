<script>
  import Head from './Head.svelte';
  import Ico from './Ico.svelte';
  import { onMount } from 'svelte';

  const tauri = window.__TAURI__;
  /** @type {{ current: string, available: string | null, notes: string | null, checked: number | null, error: string | null, checking: boolean, progress: number | null }} */
  let info = $state({ current: '', available: null, notes: null, checked: null, error: null, checking: false, progress: null });

  onMount(() => {
    if (!tauri) return;
    tauri.core.invoke('update_status').then((/** @type {any} */ i) => (info = i));
    const off = tauri.event.listen('update-status', (/** @type {any} */ e) => (info = e.payload));
    return () => off.then((/** @type {() => void} */ f) => f());
  });

  const check = () => tauri?.core.invoke('check_for_update');
  // On success the installer closes OASIS; a failure comes back in `info.error`.
  const install = () => tauri?.core.invoke('install_update').catch(() => {});

  const when = $derived(info.checked ? new Date(info.checked * 1000).toLocaleString([], { dateStyle: 'medium', timeStyle: 'short' }) : null);
  const line = $derived(
    info.progress !== null ? `Downloading OASIS ${info.available}… ${info.progress}%. OASIS restarts when it is installed.`
    : info.checking ? 'Checking for updates…'
    : info.error ? `The last check failed: ${info.error}`
    : info.available ? `OASIS ${info.available} is ready to install. It takes a minute and OASIS restarts on its own.`
    : when ? `You have the latest version. Last checked ${when}.`
    : 'OASIS checks for updates when it starts and twice a day.'
  );
</script>

<section class="group" aria-label="Updates">
  <Head title="Updates"><span class="chip">Version {info.current}</span></Head>
  <div class="srow">
    <Ico n="download" />
    <div class="rtext">
      <div class="rname">{info.available ? `OASIS ${info.available} is available` : info.error ? 'Could not check for updates' : info.checking ? 'Checking for updates…' : 'OASIS is up to date'}</div>
      <div class="hint" role="status">{line}</div>
      {#if info.available && info.notes && info.progress === null}<div class="hint notes">{info.notes}</div>{/if}
      {#if info.progress !== null}<div class="meter" role="progressbar" aria-label="Update download" aria-valuemin="0" aria-valuemax="100" aria-valuenow={info.progress}><span style="width:{info.progress}%"></span></div>{/if}
    </div>
    {#if info.available}
      <button class="pillbtn dark" onclick={install} disabled={info.progress !== null}>Install {info.available}</button>
    {:else}
      <button class="pillbtn" onclick={check} disabled={info.checking}>Check now</button>
    {/if}
  </div>
</section>

<style>
  .notes { white-space: pre-line; margin-top: 6px; }
  .meter { height: 6px; margin-top: 10px; border-radius: 3px; background: var(--hair); overflow: hidden; }
  .meter span { display: block; height: 100%; background: var(--signal); transition: width .25s var(--ease); }
</style>
