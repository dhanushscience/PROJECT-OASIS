<script>
  import { onMount } from 'svelte';
  import PadCard from './components/PadCard.svelte';
  import ProfilesScreen from './components/ProfilesScreen.svelte';
  import SideMenu from './components/SideMenu.svelte';
  import TopBar from './components/TopBar.svelte';
  import Banners from './components/Banners.svelte';
  import SettingsDialog from './components/SettingsDialog.svelte';
  import ConfirmDialog from './components/ConfirmDialog.svelte';
  import UpdatePrompt from './components/UpdatePrompt.svelte';
  import PrivacyAsk from './components/PrivacyAsk.svelte';
  import { settingsUi, openSettings } from './settings.svelte.js';
  import ModelsStrip from './components/ModelsStrip.svelte';
  import Guide from './components/Guide.svelte';
  import { startModels } from './models.svelte.js';
  import { jump } from './profiles.svelte.js';
  import { startRestart } from './restart.svelte.js';
  import QuickLaunchScreen from './components/QuickLaunchScreen.svelte';
  import VoiceScreen from './components/VoiceScreen.svelte';
  import InsightsScreen from './components/InsightsScreen.svelte';
  import { startStatus } from './status.svelte.js';
  import { startPad } from './pad.svelte.js';

  // Order is by use; Settings sits apart (DESIGN.md section 0) and opens as a pop-up over the window.
  const sections = [
    { id: 'profiles', name: 'Profiles' },
    { id: 'quick-launch', name: 'Quick Launch' },
    { id: 'voice', name: 'Voice' },
    { id: 'insights', name: 'Insights' },
    { id: '' },
    { id: 'settings', name: 'Settings' },
  ];

  // In a narrow window (half a screen) the navigation closes: the menu button in the top bar opens it over the screen.
  let width = $state(1400);
  const narrow = $derived(width < 1100);
  let menu = $state(false);
  $effect(() => { if (!narrow) menu = false; });

  let current = $state('profiles');
  const ORDER = sections.filter((s) => s.id && s.id !== 'settings').map((s) => s.id);
  /** Where a section sits relative to the open one: the others wait above or below it and drift in or out that way. */
  const pos = (/** @type {string} */ id) => (id === current ? 'on' : ORDER.indexOf(id) < ORDER.indexOf(current) ? 'before' : 'after');
  /** @type {((name: string) => void) | null} */
  let openLegacy = null;

  /** @param {string} id */
  function pick(id) {
    menu = false;
    // Settings is a pop-up, not a section; Input is one of its parts (the pad card opens it there).
    if (id === 'settings' || id === 'input') return openSettings(id === 'input' ? 'input' : undefined);
    current = id;
    openLegacy?.(id);
  }

  /** Settings › Voice models, from the top strip's Details or the pop-up's "Choose a different set". */
  const openModels = () => openSettings('models');

  // The old script fills its settings fields when the pop-up opens.
  $effect(() => { if (settingsUi.open) openLegacy?.('settings'); });

  onMount(async () => {
    startStatus();
    startModels();
    startRestart();
    startPad();
    // The old script finds its elements by id when it loads, so it loads once they exist.
    if (window.__TAURI__) {
      const legacy = await import('./legacy/legacy.js');
      openLegacy = legacy.openSection;
      openLegacy(current);
    }
  });
</script>

<svelte:window bind:innerWidth={width} onkeydown={(e) => e.key === 'Escape' && (menu = false)} />

<div class="window" class:narrow inert={settingsUi.open}>
  {#if narrow && menu}<button class="scrim" aria-label="Close the menu" onclick={() => (menu = false)}></button>{/if}
  <div class="left" class:open={menu} inert={narrow && !menu}>
    <SideMenu {sections} current={settingsUi.open ? 'settings' : current} onpick={pick} />
    <div class="fill"></div>
    <PadCard onpick={pick} />
  </div>

  <div class="right">
    <TopBar {narrow} onmenu={() => (menu = !menu)} {menu} onprofile={(i) => { pick('profiles'); jump(i); }} />
    <Banners />
    <ModelsStrip ondetails={openModels} />

    <main>
      <section class="panel screen {pos('profiles')}" aria-label="Profiles" inert={current !== 'profiles'}>
        <ProfilesScreen active={current === 'profiles'} />
      </section>

      <section class="panel tight {pos('quick-launch')}" aria-label="Quick Launch" inert={current !== 'quick-launch'}>
        <div class="lab">Section</div>
        <div class="titlerow">
          <h1>Quick Launch</h1>
          <p>Open several apps at once, each on the screen and in the part of it you choose. Pick the apps yourself, or arrange your windows and press Capture. Say a setup’s name, like “open project mode”, to launch it by voice.</p>
        </div>
        <QuickLaunchScreen />
      </section>

      <section class="panel tight {pos('voice')}" aria-label="Voice" inert={current !== 'voice'}>
        <div class="lab">Section</div>
        <div class="titlerow">
          <h1>Voice</h1>
          <p>Hold your talk button and speak: it is typed where the cursor is, or runs as a command. Teach it the words it gets wrong below.</p>
          <div class="voicetalk"><VoiceScreen part="talk" /></div>
        </div>
        <!-- the old status script still writes here; the new screen shows the same state itself -->
        <span hidden><span id="voiceText"></span><span id="voiceHint"></span></span>
        <VoiceScreen />
      </section>

      <section class="panel tight {pos('insights')}" aria-label="Insights" inert={current !== 'insights'}>
        <div class="lab">Section</div>
        <div class="titlerow">
          <h1>Insights</h1>
          <p>Everything you have dictated, newest first. Hover a line to play what you said or copy the text.</p>
        </div>
        <InsightsScreen active={current === 'insights'} />
      </section>
    </main>
  </div>
</div>
<Guide />
<SettingsDialog />
<ConfirmDialog />
<UpdatePrompt />
<PrivacyAsk />

<style>
  .window { width: 100vw; height: 100dvh; padding: 12px 28px 28px 12px; display: flex; gap: 12px; }
  /* narrow: the navigation is a drawer over the screen, closed until the menu button opens it */
  .narrow .left { position: fixed; z-index: 30; left: 0; top: 0; bottom: 0; width: 248px; padding: 56px 20px 18px 12px; background: var(--bg); box-shadow: 8px 0 28px var(--shadow); transform: translateX(-110%); transition: transform .26s var(--ease); }
  .narrow .left.open { transform: none; }
  /* the current section still runs out to the drawer's edge, as it runs into the screen in the wide layout */
  .narrow .left :global(nav button) { margin-right: -20px; }
  .scrim { position: fixed; inset: 0; z-index: 20; border: 0; background: rgb(0 0 0 / .28); }
  .narrow { padding-left: 12px; }
  .left { width: 184px; flex: none; display: flex; flex-direction: column; padding-top: 48px; }
  .fill { flex: 1; }  .right { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }

  /* One stage; the sections are stacked in it and the current one fades in (transform and opacity only). */
  main { position: relative; flex: 1; min-height: 0; background-color: var(--surface);
    border-radius: var(--r-stage); display: grid; grid-template-columns: minmax(0, 1fr); overflow: hidden; }
  /* The frame is drawn ON TOP of the sections: the inner shadow, and a short fade where content scrolls out at the
     top and bottom. Drawn behind them, the cards covered it and the edge looked cut off. */
  main::after { content: ''; position: absolute; inset: 0; z-index: 5; pointer-events: none; border-radius: inherit;
    box-shadow: inset 0 3px 8px var(--shadow), inset 0 0 8px var(--shadow), inset 0 -2px 5px var(--shadow);
    background: linear-gradient(var(--surface), transparent) top / 100% 18px no-repeat, linear-gradient(transparent, var(--surface)) bottom / 100% 18px no-repeat; }
  .panel { grid-area: 1 / 1; min-height: 0; overflow: auto; padding: 28px; display: flex; flex-direction: column; gap: 14px;
    opacity: 0; transform: translateY(12px); visibility: hidden;
    transition: opacity .16s var(--ease), transform .16s var(--ease), visibility 0s linear .16s; }
  .panel.before { transform: translateY(-12px); }
  .panel.on { opacity: 1; transform: none; visibility: visible; transition-delay: 0s; }
  .screen { padding: 18px 18px 18px 28px; flex-direction: row; }
  .lab { font: 500 11px/1.3 var(--mono); letter-spacing: .12em; text-transform: uppercase; color: var(--signal-ink); }
  /* the title with its description beside it, level with the title's baseline; wraps below it when narrow */
  .panel.tight { gap: 6px; }
  /* the description takes the room beside the title (two lines, not four) and rests on the title's baseline */
  .titlerow { display: flex; align-items: flex-end; flex-wrap: wrap; gap: 6px 40px; }
  /* top right beside the title; in a narrow window it drops under the description, left-aligned, with no gap */
  .voicetalk { margin-left: auto; align-self: flex-start; padding-top: 6px; min-width: 0; }
  @media (max-width: 1250px) { .voicetalk { margin-left: 0; align-self: auto; padding-top: 0; flex-basis: 100%; } }
  .titlerow p { flex: 1 1 360px; max-width: 92ch; font-size: 14px; line-height: 1.5; padding-bottom: 8px; }
  h1 { font-size: 72px; font-weight: 700; letter-spacing: -.05em; line-height: .9; margin: 0; }
  p { margin: 0; font-size: 15px; color: var(--muted); max-width: 52ch; }
</style>
