<script>
  // A settings group's heading: a numbered title and one short line on the left, the group's own switch or
  // buttons on the right. legacy.js builds the same markup (headNode).
  /** @type {{ title: string, text?: string, id?: string, children?: import('svelte').Snippet }} */
  let { title, text = '', id, children } = $props();
</script>

<div class="ghead hero">
  <div class="htext"><span class="lab" {id}>{title}</span>{#if text}<p class="gh">{text}</p>{/if}</div>
  {#if children}<div class="hctl">{@render children()}</div>{/if}
</div>
