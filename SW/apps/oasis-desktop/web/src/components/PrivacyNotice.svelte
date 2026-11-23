<script>
  // The one notice, shown before anything can be switched on: in the first-run question and in Settings.
  // Keep it in step with PRIVACY.md and privacy.html, and bump CONSENT_VERSION (oasis-settings/src/privacy.rs) when it changes.
  let { adult = $bindable(false), askAge = true } = $props();
  const policy = () => window.__TAURI__?.core.invoke('open_privacy_policy').catch(() => {});
  const SENT = [
    { name: 'Words dictated', note: 'How many words you spoke, as one number.', icon: 'M4 7h16M4 12h16M4 17h10' },
    { name: 'Dictations made', note: 'How many times you dictated.', icon: 'M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3zM6 11a6 6 0 0 0 12 0M12 17v4' },
    { name: 'Corrected by hand', note: 'How many of those you fixed afterwards.', icon: 'M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16v4zM13.5 6.5l4 4' },
  ];
  // Three on each side, so what is sent and what is not read as a matched pair.
  const NEVER = [
    { name: 'What you say or type', note: 'Not your words, not your audio.' },
    { name: 'Who you are', note: 'No name, email, ID or IP address.' },
    { name: 'What you use', note: 'No files, windows, programs or sites.' },
  ];
</script>

<div class="pn">
  <header>
    <h3>Three numbers a day. Nothing else.</h3>
    <p>Optional. OASIS works exactly the same if you say no.</p>
  </header>

  <div class="cols">
    <section>
      <p class="cap"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M8 12.5l3 3 5-6" /></svg>Sent</p>
      <ul class="rows">
        {#each SENT as s}
          <li>
            <span class="badge"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={s.icon} /></svg></span>
            <span class="txt"><span class="nm">{s.name}</span><span class="nt">{s.note}</span></span>
          </li>
        {/each}
      </ul>
      <p class="fine">With the date, your OASIS version, the send time and a random number new for each day. The policy lists every detail.</p>
    </section>

    <section>
      <p class="cap never"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M9 9l6 6M15 9l-6 6" /></svg>Never sent</p>
      <ul class="rows">
        {#each NEVER as n}
          <li>
            <span class="badge no"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M7 7l10 10M17 7L7 17" /></svg></span>
            <span class="txt"><span class="nm">{n.name}</span><span class="nt">{n.note}</span></span>
          </li>
        {/each}
      </ul>
    </section>
  </div>

  <div class="control">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="5" y="11" width="14" height="9" rx="2" /><path d="M8 11V8a4 4 0 0 1 8 0v3" /></svg>
    <p>Off until you say yes. Turn it off any time in Settings, Privacy: counting stops, the counts kept on this PC are deleted, and nothing more is sent.</p>
  </div>

  <div class="foot">
    <details>
      <summary>Why, where it goes, how long it is kept</summary>
      <p>To see how much OASIS is used and whether dictation is getting more accurate (fewer corrections). The numbers go to Aptabase (EU servers), are not linked to you, and are kept for up to 5 years, so individual numbers cannot be looked up or deleted later. Questions: oasis-privacy@protonmail.com</p>
    </details>
    <button class="link" type="button" onclick={policy}>Read the full privacy policy</button>
  </div>

  {#if askAge}
    <label class="age"><input type="checkbox" bind:checked={adult} /><span>I am 18 or older. (If you are under 18, please leave this off.)</span></label>
  {/if}
</div>

<style>
  .pn { container-type: inline-size; display: flex; flex-direction: column; gap: 22px; text-align: left; font-family: inherit; font-size: 14px; line-height: 1.5; color: var(--muted); }
  .pn :is(h3, p, ul, li, section, header) { margin: 0; padding: 0; font-family: inherit; }
  .pn li { list-style: none; }

  h3 { font-size: 19px; line-height: 1.3; font-weight: 650; letter-spacing: -.015em; color: var(--ink); }
  header p { margin-top: 4px; }

  /* Sent on the left, never sent on the right; one column when narrow. */
  .cols { display: grid; grid-template-columns: 1fr 1fr; gap: 0 36px; }
  @container (max-width: 520px) { .cols { grid-template-columns: 1fr; gap: 22px; } }

  .cap { display: flex; align-items: center; gap: 8px; font-size: 12px; font-weight: 650; letter-spacing: .06em; text-transform: uppercase; color: var(--ink); }
  .cap svg { width: 16px; height: 16px; color: #3a9d66; }
  .cap.never svg { color: #b4584f; }

  .rows { display: flex; flex-direction: column; margin-top: 2px; }
  .rows li { display: flex; align-items: center; gap: 12px; padding: 11px 0; border-bottom: 1px solid var(--hair); }
  .rows li:last-child { border-bottom: 0; }
  .badge { display: grid; place-items: center; width: 34px; height: 34px; flex: none; border-radius: 50%; background: color-mix(in srgb, var(--ink) 6%, transparent); color: var(--ink); }
  .badge.no { background: color-mix(in srgb, #b4584f 11%, transparent); color: #b4584f; }
  .badge svg { width: 16px; height: 16px; }
  .txt { display: flex; flex-direction: column; }
  .nm { font-size: 14.5px; font-weight: 600; color: var(--ink); }
  .nt { font-size: 13px; }
  .fine { margin-top: 2px; font-size: 12.5px; }

  .control { display: flex; gap: 12px; align-items: flex-start; font-size: 13.5px; }
  .control svg { width: 18px; height: 18px; flex: none; margin-top: 2px; color: var(--ink); opacity: .7; }

  .foot { display: flex; flex-wrap: wrap; align-items: baseline; gap: 6px 24px; }
  details summary { cursor: pointer; font-size: 13.5px; font-weight: 600; color: var(--ink); }
  details p { margin-top: 8px; font-size: 13px; }
  details[open] { flex: 1 1 100%; }
  .link { padding: 0; border: 0; background: none; color: var(--signal); text-decoration: underline; text-underline-offset: 3px; font: inherit; font-size: 13.5px; cursor: pointer; }

  .age { display: flex; align-items: flex-start; gap: 12px; color: var(--ink); cursor: pointer; }
  .age input { width: 18px; height: 18px; margin: 2px 0 0; flex: none; accent-color: var(--signal); }
</style>
