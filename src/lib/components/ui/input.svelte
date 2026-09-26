<script lang="ts">
  let { value = $bindable(''), placeholder = '', ariaLabel, disabled = false, invalid = false, type = 'text', ...rest }: import('svelte/elements').HTMLInputAttributes & { ariaLabel?: string; invalid?: boolean } = $props();
</script>

<input class="ui-input" bind:value {placeholder} aria-label={ariaLabel} aria-invalid={invalid} {disabled} {type} {...rest} />
