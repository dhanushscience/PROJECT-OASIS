<script>
  // A setting row's icon, in a small tile on the left of the row.
  import { ICONS } from '../settingsArt.js';
  let { n } = $props();
</script>

<span class="ico" aria-hidden="true"><svg viewBox="0 0 24 24"><path d={ICONS[/** @type {keyof typeof ICONS} */ (n)] ?? ''} /></svg></span>
