<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLButtonAttributes } from 'svelte/elements';
  let { children, variant = 'default', disabled = false, type = 'button', class: className = '', ...rest }: HTMLButtonAttributes & { children: Snippet; variant?: 'default' | 'outline' | 'secondary' | 'ghost' | 'destructive'; class?: string } = $props();
</script>

<button class={`ui-button ui-button-${variant} ${className}`} {type} {disabled} {...rest}>
  {@render children()}
</button>
