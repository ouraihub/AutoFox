<script lang="ts">
  import { Badge, Button, Empty, Progress } from '$lib/components/ui';
  import { formatSize, formatSpeed } from '$lib/format';
  import type { DownloadOption, MediaManifest, TaskProgress } from '$lib/ipc/client';

  type Props = {
    manifest: MediaManifest | null;
    progress: TaskProgress | null;
    busy: boolean;
    activeOptionId: string | undefined;
    onDownload: (option: DownloadOption) => void;
  };

  const { manifest, progress, busy, activeOptionId, onDownload }: Props = $props();

  // The muxer reports `percent` without a byte total and the download path
  // reports both, so a single derived value keeps the bar on one code path.
  const percent = $derived(
    progress?.percent ??
      (progress?.totalBytes ? progress.bytesWritten / progress.totalBytes : null),
  );
</script>

{#if manifest}
  <div class="panel-heading">
    <h2>{manifest.title}</h2>
    <span class="meta">
      {manifest.producedBy} · {manifest.backendVersion} · {manifest.downloadOptions.length} 个可选画质
    </span>
  </div>

  {#if progress}
    <div class="progress-card">
      <div class="progress-line">
        <span>
          {percent === null
            ? formatSize({ kind: 'exact', bytes: progress.bytesWritten })
            : `${Math.round(percent * 100)}%`}
        </span>
        <span>{formatSpeed(progress.bytesPerSecond)}</span>
      </div>
      <!-- With no percent the total is unknown, so the bar would read
           value == max and look finished. Keep it at zero and let the
           byte counter carry the signal instead. -->
      <Progress value={percent ?? 0} max={1} label="下载进度" />
    </div>
  {/if}

  <div class="rows">
    {#each manifest.downloadOptions as option (option.optionId)}
      <article class="row">
        <div><strong>{option.label}</strong><small>{option.detail}</small></div>
        <div class="row-meta">
          <Badge variant="outline">{option.container.toUpperCase()}</Badge>
          <span class="size">{formatSize(option.size)}</span>
          <Button onclick={() => onDownload(option)} disabled={busy}>
            {busy && activeOptionId === option.optionId ? '下载中…' : '下载'}
          </Button>
        </div>
      </article>
    {/each}
  </div>

  <details class="advanced">
    <summary>全部原始格式（{manifest.formats.length}）</summary>
    <div class="rows">
      {#each manifest.formats as format (format.formatId)}
        <article class="row">
          <div>
            <strong>{format.formatId}</strong>
            <small>
              {format.container} · {format.delivery.kind}{format.video?.width
                ? ` · ${format.video.width}×${format.video.height}`
                : ''}
            </small>
          </div>
          <div class="row-meta">
            <Badge variant="outline">{format.video?.codec ?? format.audio?.codec ?? '无媒体轨'}</Badge>
            <span class="size">{formatSize(format.size)}</span>
          </div>
        </article>
      {/each}
    </div>
  </details>
{:else}
  <Empty title="还没有解析结果" description="在上方粘贴链接并点击开始解析，可用格式会列在这里。" />
{/if}

<style>
  /* Layout primitives (`.panel-heading`, `.rows`, `.row`, `.row-meta`, `.size`,
     `.meta`) are global in `src/app.css`; only this panel's own parts live here. */
  .advanced { margin-top: var(--space-md); border-top: 1px solid var(--border); }
  .advanced summary { padding: .7rem 0; color: var(--muted-foreground); cursor: pointer; font-size: .72rem; }
  .advanced summary:hover { color: var(--foreground); }
  .advanced summary:focus-visible { outline: 2px solid var(--ring); outline-offset: 2px; }

  .progress-card { padding: var(--space-sm) 0 var(--space-md); }
  .progress-line { display: flex; justify-content: space-between; margin-bottom: .4rem; font-family: var(--font-mono); font-size: .72rem; font-variant-numeric: tabular-nums; }
</style>
