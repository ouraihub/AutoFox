<script lang="ts">
  import { onMount } from 'svelte';
  import { ChevronDown, ChevronRight } from '@lucide/svelte';
  import Button from './button.svelte';
  import Badge from './badge.svelte';
  import { clearLogs, isDesktop, readLogs, type LogEntry } from '$lib/ipc/client';

  let { expanded = $bindable(false) }: { expanded?: boolean } = $props();

  let entries = $state<LogEntry[]>([]);
  let onlyProblems = $state(false);
  let loading = $state(false);
  let failure = $state('');
  let scroller = $state<HTMLUListElement | null>(null);
  let pinned = $state(true);

  const visible = $derived(
    onlyProblems ? entries.filter((entry) => entry.level === 'ERROR' || entry.level === 'WARN') : entries,
  );
  const warnCount = $derived(entries.filter((entry) => entry.level === 'WARN').length);
  const errorCount = $derived(entries.filter((entry) => entry.level === 'ERROR').length);
  // `read_logs` returns newest first, so the collapsed summary line is index 0.
  const latest = $derived(entries[0]);

  /// Reads the tail. `showPending` stays false for the poll so a 1s refresh
  /// never flashes a spinner for an action the user did not start.
  async function refresh(showPending = false) {
    let timer: ReturnType<typeof setTimeout> | undefined;
    // Delay-show: a read that resolves in 20ms must not paint a spinner at all.
    if (showPending) timer = setTimeout(() => { loading = true; }, 150);
    try {
      entries = await readLogs(300);
      failure = '';
    } catch (error) {
      failure = error instanceof Error ? error.message : '读取日志失败。';
    } finally {
      if (timer) clearTimeout(timer);
      loading = false;
    }
  }

  onMount(() => {
    void refresh(true);
    if (!isDesktop) return;
    // Polling beats an event stream here: `read_logs` reads a capped in-memory
    // ring buffer, so a 1s tick costs one lock and one clone of ≤300 rows.
    const timer = setInterval(() => void refresh(), 1000);
    return () => clearInterval(timer);
  });

  // Stick to the newest row unless the user scrolled up to read history.
  $effect(() => {
    if (!expanded || !pinned || visible.length === 0) return;
    const node = scroller;
    if (node) node.scrollTop = 0;
  });

  function onscroll() {
    // The list is `column-reverse`, so "newest visible" is scrollTop ≈ 0.
    if (scroller) pinned = scroller.scrollTop > -8;
  }

  function shortTarget(target: string) {
    return target.split('::').at(-1) ?? target;
  }

  function levelClass(level: string) {
    if (level === 'ERROR') return 'log-error';
    if (level === 'WARN') return 'log-warn';
    return level === 'DEBUG' || level === 'TRACE' ? 'log-muted' : 'log-info';
  }
</script>

<section class="log-bar" class:log-bar-open={expanded} aria-label="后台日志">
  <div class="log-summary">
    <button
      type="button"
      class="log-toggle"
      aria-expanded={expanded}
      aria-controls="log-body"
      onclick={() => (expanded = !expanded)}
    >
      {#if expanded}<ChevronDown size={15} />{:else}<ChevronRight size={15} />{/if}
      日志
    </button>
    {#if failure}
      <p class="log-line log-error" role="alert">{failure}</p>
    {:else if loading && entries.length === 0}
      <p class="log-line log-muted">正在读取…</p>
    {:else if latest}
      <p class="log-line">
        <time>{latest.time}</time>
        <span class={levelClass(latest.level)}>{latest.level}</span>
        <code>{shortTarget(latest.target)}</code>
        <span class="log-message">{latest.message}</span>
      </p>
    {:else}
      <p class="log-line log-muted">暂无日志</p>
    {/if}
    <div class="log-counts">
      <Badge variant={warnCount > 0 ? 'outline' : 'secondary'}>{warnCount} 警告</Badge>
      <Badge variant={errorCount > 0 ? 'destructive' : 'secondary'}>{errorCount} 错误</Badge>
    </div>
  </div>

  {#if expanded}
    <div class="log-body" id="log-body">
      <div class="log-controls">
        <label class="log-filter"><input type="checkbox" bind:checked={onlyProblems} />只看警告和错误</label>
        <Button variant="outline" onclick={() => refresh(true)} disabled={loading}>
          {loading ? '读取中…' : '刷新'}
        </Button>
        <Button variant="ghost" onclick={async () => { await clearLogs(); entries = []; }}>清空</Button>
      </div>
      {#if visible.length === 0}
        <!-- The shared `Empty` block is sized for a full panel; in a docked bar
             it would spend ~200px on zero rows and steal that height from the
             active tab. A single line says the same thing. -->
        <p class="log-vacant">{onlyProblems ? '没有警告或错误。' : '解析、下载或检测后端后，日志会显示在这里。'}</p>
      {:else}
        <ul class="log-list" bind:this={scroller} {onscroll}>
          {#each visible as entry (entry.seq)}
            <li>
              <time>{entry.time}</time>
              <Badge variant={entry.level === 'ERROR' ? 'destructive' : entry.level === 'WARN' ? 'outline' : 'secondary'}>{entry.level}</Badge>
              <code>{shortTarget(entry.target)}</code>
              <span class={levelClass(entry.level)}>{entry.message}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}
</section>

<style>
  .log-bar {
    /* The bar owns its height: as a flex child of a fixed-height shell it must
       not be squeezed when the active panel is tall. */
    flex: 0 0 auto;
    border-top: 1px solid var(--border);
    background: color-mix(in oklab, var(--background) 92%, var(--foreground));
    font-variant-numeric: tabular-nums;
  }
  .log-summary { display: flex; align-items: center; gap: var(--space-sm); min-height: 2.75rem; padding: 0 var(--space-xl); }
  .log-toggle {
    display: inline-flex; align-items: center; gap: .35rem; flex: 0 0 auto;
    min-height: 2.25rem; padding: .3rem .5rem;
    border: 1px solid transparent; border-radius: .5rem;
    background: transparent; color: var(--foreground);
    font: inherit; font-size: .72rem; font-weight: 650;
    outline: 2px solid transparent; outline-offset: 2px;
  }
  .log-toggle:focus-visible { outline-color: var(--ring); }
  .log-toggle:active { transform: translateY(1px); }
  @media (hover: hover) {
    .log-toggle:hover { background: var(--secondary); }
  }
  .log-line {
    display: flex; align-items: baseline; gap: var(--space-sm);
    flex: 1; min-width: 0; margin: 0;
    font-family: var(--font-mono); font-size: .68rem;
  }
  .log-message { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .log-counts { display: flex; gap: .35rem; flex: 0 0 auto; }
  .log-body { padding: 0 var(--space-xl) var(--space-sm); }
  .log-controls { display: flex; align-items: center; justify-content: flex-end; gap: var(--space-sm); padding-bottom: var(--space-xs); }
  .log-filter { display: flex; align-items: center; gap: var(--space-xs); margin-right: auto; color: var(--muted-foreground); font-size: .72rem; }
  .log-filter input { accent-color: var(--primary); width: 1rem; height: 1rem; }
  .log-vacant {
    display: flex; align-items: center;
    min-height: 3rem; margin: 0;
    padding: 0 var(--space-xs);
    border: 1px solid var(--border); border-radius: .6rem;
    background: var(--background);
    color: var(--muted-foreground); font-size: .72rem;
  }
  .log-list {
    /* `column-reverse` renders newest-first data in newest-at-bottom order and
       makes the browser pin the scroll anchor at the newest row for free. */
    display: flex; flex-direction: column-reverse;
    /* `max-height`, not `height`: a two-row log should not reserve 14rem of
       empty box. `column-reverse` still pins newest-at-bottom once it fills. */
    max-height: 14rem; overflow: auto;
    margin: 0; padding: var(--space-xs);
    border: 1px solid var(--border); border-radius: .6rem;
    background: var(--background);
    list-style: none;
    font-family: var(--font-mono); font-size: .68rem;
  }
  .log-list li {
    display: grid; grid-template-columns: 5.5rem 4rem 8rem minmax(0, 1fr);
    align-items: baseline; gap: var(--space-sm);
    min-height: 1.75rem; padding: .2rem var(--space-xs);
  }
  @media (hover: hover) {
    .log-list li:hover { background: var(--muted); }
  }
  .log-list time, .log-list code, .log-line time, .log-line code { color: var(--muted-foreground); }
  .log-error { color: var(--destructive); }
  .log-warn { color: var(--feature-orange-ink); }
  .log-muted { color: var(--muted-foreground); }
  .log-info { color: var(--foreground); }
  @media (max-width: 680px) {
    .log-summary { padding: 0 var(--space-md); }
    .log-body { padding: 0 var(--space-md) var(--space-sm); }
    .log-counts { display: none; }
    .log-list li { grid-template-columns: 4.5rem minmax(0, 1fr); }
  }
</style>
