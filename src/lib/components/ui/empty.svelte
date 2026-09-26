<script lang="ts">
  import type { Snippet } from 'svelte';
  let { children, title, description }: { children?: Snippet; title: string; description: string } = $props();
</script>

<div class="ui-empty">
  <div class="ui-empty-mark">—</div>
  <strong>{title}</strong>
  <p>{description}</p>
  {@render children?.()}
</div>
