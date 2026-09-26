<script lang="ts">
  let { value = 0, max = 100, label = '' }: { value?: number; max?: number; label?: string } = $props();
</script>

<div class="ui-progress" role="progressbar" aria-valuenow={value} aria-valuemin="0" aria-valuemax={max} aria-label={label}>
  <span style={`width: ${max ? Math.min(100, value / max * 100) : 0}%`}></span>
</div>
