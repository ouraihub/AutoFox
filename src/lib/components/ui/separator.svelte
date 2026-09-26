<script lang="ts">
  let { orientation = 'horizontal' }: { orientation?: 'horizontal' | 'vertical' } = $props();
</script>

<div class={`ui-separator ui-separator-${orientation}`} role="separator" aria-orientation={orientation}></div>
