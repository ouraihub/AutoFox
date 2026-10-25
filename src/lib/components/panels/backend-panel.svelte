<script lang="ts">
  import { Badge, Button } from '$lib/components/ui';
  import type { BackendStatus } from '$lib/ipc/client';

  type Props = {
    backend: BackendStatus | null;
    busy: boolean;
    onRecheck: () => void;
  };

  const { backend, busy, onRecheck }: Props = $props();
</script>

<div class="panel-heading">
  <h2>解析后端</h2>
  <Button variant="outline" onclick={onRecheck} disabled={busy}>
    {busy ? '检测中…' : '重新检测'}
  </Button>
</div>

<div class="rows">
  <div class="row">
    <div class="named">
      <span class:ready={backend?.ytdlpReady} class="backend-dot"></span>
      <div><strong>yt-dlp</strong><small>{backend?.ytdlpPath ?? '随包 · 同目录 yt-dlp.exe'}</small></div>
    </div>
    <Badge variant={backend?.ytdlpReady ? 'default' : 'outline'}>
      {backend?.ytdlpReady ? (backend.version ?? '就绪') : '未检测'}
    </Badge>
  </div>
  <div class="row">
    <div class="named">
      <span class:ready={backend?.ffmpegReady} class="backend-dot"></span>
      <div><strong>ffmpeg</strong><small>{backend?.ffmpegPath ?? '随包 · lib/ffmpeg.exe'}</small></div>
    </div>
    <Badge variant={backend?.ffmpegReady ? 'default' : 'outline'}>
      {backend?.ffmpegReady ? '就绪' : '未检测'}
    </Badge>
  </div>
</div>

<style>
  /* `.panel-heading`, `.rows`, `.row`, `.named` are global in `src/app.css`. */
  /* Status is never colour-only: the badge text says 就绪 or 未检测 next to the
     dot, so the state survives a monochrome display. */
  .backend-dot { width: .45rem; height: .45rem; border-radius: 50%; background: var(--muted-foreground); }
  .backend-dot.ready { background: var(--success); }
</style>
