<script lang="ts">
  import type { Snippet } from 'svelte';
  let { children, variant = 'secondary', class: className = '' }: { children: Snippet; variant?: 'default' | 'secondary' | 'outline' | 'destructive'; class?: string } = $props();
</script>

<span class={`ui-badge ui-badge-${variant} ${className}`}>{@render children()}</span>
