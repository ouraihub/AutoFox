<script lang="ts">
  import type { Snippet } from 'svelte';

  export type TabItem = { id: string; label: string; disabled?: boolean };

  let {
    items,
    value = $bindable(),
    panel,
  }: { items: TabItem[]; value: string; panel: Snippet<[string]> } = $props();

  let list = $state<HTMLDivElement | null>(null);

  const selectable = $derived(items.filter((item) => !item.disabled));

  /// Moves selection by `step` across enabled tabs, wrapping at both ends, and
  /// carries focus with it. The WAI-ARIA tabs pattern ties arrow keys to
  /// selection for automatic-activation tablists, so the panel follows focus.
  function step(delta: number) {
    if (selectable.length === 0) return;
    const current = selectable.findIndex((item) => item.id === value);
    const next = selectable[(current + delta + selectable.length) % selectable.length];
    if (next) select(next.id);
  }

  function select(id: string) {
    value = id;
    // The tablist uses roving tabindex, so the newly selected trigger is the
    // only tab stop. Focus has to follow or the next Tab press leaves the list
    // from a control that no longer exists in the tab order.
    queueMicrotask(() => list?.querySelector<HTMLButtonElement>(`[data-tab="${id}"]`)?.focus());
  }

  function onkeydown(event: KeyboardEvent) {
    const handlers: Record<string, () => void> = {
      ArrowRight: () => step(1),
      ArrowLeft: () => step(-1),
      Home: () => selectable[0] && select(selectable[0].id),
      End: () => selectable.at(-1) && select(selectable[selectable.length - 1]!.id),
    };
    const handler = handlers[event.key];
    if (!handler) return;
    event.preventDefault();
    handler();
  }
</script>

<div class="ui-tabs">
  <div class="ui-tabs-list" role="tablist" bind:this={list}>
    {#each items as item (item.id)}
      <button
        type="button"
        role="tab"
        class="ui-tabs-trigger"
        data-tab={item.id}
        id={`tab-${item.id}`}
        aria-selected={value === item.id}
        aria-controls={`panel-${item.id}`}
        disabled={item.disabled}
        aria-disabled={item.disabled}
        tabindex={value === item.id ? 0 : -1}
        onclick={() => select(item.id)}
        {onkeydown}
      >
        {item.label}
      </button>
    {/each}
  </div>
  {#each items as item (item.id)}
    {#if value === item.id}
      <!-- `tabindex="0"` so the panel itself is reachable: with the roving
           tabindex above, a panel holding no focusable control would otherwise
           be skipped entirely by keyboard users. -->
      <div
        class="ui-tabs-panel"
        role="tabpanel"
        id={`panel-${item.id}`}
        aria-labelledby={`tab-${item.id}`}
        tabindex="0"
      >
        {@render panel(item.id)}
      </div>
    {/if}
  {/each}
</div>
