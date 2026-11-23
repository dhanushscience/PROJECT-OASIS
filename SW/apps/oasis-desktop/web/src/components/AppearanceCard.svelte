<script>
  import Head from './Head.svelte';
  import Ico from './Ico.svelte';
  import { themeChoice, chooseTheme } from '../theme.js';
  import { view, setLayout } from '../pad.svelte.js';

  let choice = $state(themeChoice());
  const THEMES = [['system', 'System'], ['light', 'Light'], ['dark', 'Dark']];
  const LAYOUTS = [['auto', 'Automatic'], ['minimal', 'Minimal'], ['full', 'Full']];
  /** @param {'system' | 'light' | 'dark'} c */
  const pick = (c) => { choice = c; chooseTheme(c); };
</script>

<section class="group" aria-label="Appearance">
  <Head title="Theme and keyboard" text="How OASIS itself looks." />

  <div class="srow">
    <Ico n="sun" />
    <div class="rtext"><div class="rname">Colour theme</div><div class="hint">System follows Windows.</div></div>
    <div class="seg" role="radiogroup" aria-label="Colour theme">
      {#each THEMES as [id, name]}
        <button type="button" role="radio" aria-checked={choice === id} class:on={choice === id} onclick={() => pick(/** @type {any} */ (id))}>{name}</button>
      {/each}
    </div>
  </div>

  <div class="srow">
    <Ico n="keyboard" />
    <div class="rtext"><div class="rname">Keyboard view</div><div class="hint">How Profiles draws the keyboard. Minimal shows only the keys you use.</div></div>
    <div class="seg" role="radiogroup" aria-label="Keyboard view">
      {#each LAYOUTS as [id, name]}
        <button type="button" role="radio" aria-checked={view.layout === id} class:on={view.layout === id} onclick={() => setLayout(/** @type {any} */ (id))}>{name}</button>
      {/each}
    </div>
  </div>
</section>
