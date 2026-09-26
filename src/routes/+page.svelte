<script lang="ts">
  import { onMount } from 'svelte';
  import { Download, FolderOpen, Activity } from '@lucide/svelte';
  import { Button, Input, Badge, Progress, Empty, Tabs, LogBar, type TabItem } from '$lib/components/ui';
  import { downloadDir } from '@tauri-apps/api/path';
  import { listen } from '@tauri-apps/api/event';
  import { cancelTask, downloadDirect, downloadStream, getBackendStatus, getSettings, isDesktop, listTasks, probeUrl, saveSettings, type BackendStatus, type MediaFormat, type MediaManifest, type Settings, type TaskSummary } from '$lib/ipc/client';

  let url = $state('');
  let backend = $state<BackendStatus | null>(null);
  let manifest = $state<MediaManifest | null>(null);
  let probing = $state(false);
  let downloading = $state(false);
  let selectedFormat = $state<MediaFormat | null>(null);
  let progress = $state<{ bytesWritten: number; totalBytes?: number; bytesPerSecond: number } | null>(null);
  let notice = $state('');
  let tasks = $state<TaskSummary[]>([]);
  let settings = $state<Settings>({ schemaVersion: 1, extractorPolicy: 'auto', allowPathLookup: false, logLevel: 'INFO' });
  let activeTab = $state('download');
  let logExpanded = $state(false);

  const tabs: TabItem[] = [
    { id: 'download', label: '下载' },
    { id: 'tasks', label: '任务' },
    { id: 'backend', label: '后端' },
    { id: 'settings', label: '设置' },
    { id: 'about', label: '关于' },
  ];

  onMount(() => {
    // `listen` reaches into the Tauri IPC bridge and throws outside the
    // desktop runtime. Unguarded it aborts this whole mount callback, so the
    // browser preview loses the task list and every later effect in the flush.
    if (!isDesktop) return;
    let unlisten: (() => void) | undefined;
    listen<{ bytesWritten: number; totalBytes?: number; bytesPerSecond: number }>('task://progress', (event) => {
      progress = event.payload;
    }).then((dispose) => { unlisten = dispose; });
    listTasks().then((items) => { tasks = items; }).catch(() => { /* backend not ready */ });
    // Settings now live in a tab rather than behind a dialog, so they load with
    // the page instead of on a click.
    getSettings().then((value) => { settings = value; }).catch(() => { /* backend not ready */ });
    return () => unlisten?.();
  });

  async function checkBackend() {
    probing = true;
    notice = '';
    try {
      backend = await getBackendStatus();
      notice = backend.ytdlpReady ? 'yt-dlp 已就绪，可以开始解析。' : '未找到随包 yt-dlp.exe，请检查安装目录。';
    } catch {
      notice = '桌面后端尚未连接。浏览器预览模式下仅展示界面。';
    } finally {
      probing = false;
    }
  }

  async function startProbe() {
    const value = url.trim();
    if (!value) { notice = '请先粘贴一个 http(s) 视频链接。'; return; }
    probing = true;
    manifest = null;
    selectedFormat = null;
    notice = '正在调用 yt-dlp 解析格式…';
    // A probe started from another tab has its result on the download tab, so
    // move there rather than leaving the user watching an unrelated panel.
    activeTab = 'download';
    try {
      manifest = await probeUrl(value);
      notice = `解析完成：${manifest.formats.length} 个格式，后端 ${manifest.backendVersion}`;
    } catch (error) {
      notice = error instanceof Error ? error.message : '解析失败，请检查链接或后端状态。';
    } finally { probing = false; }
  }

  async function startDownload(format: MediaFormat) {
    downloading = true;
    progress = null;
    notice = format.delivery.kind === 'direct' ? '正在下载…' : '正在用 ffmpeg 处理流媒体…';
    try {
      const directory = await downloadDir();
      const result = format.delivery.kind === 'direct'
        ? await downloadDirect({ url: format.delivery.url, formatId: format.formatId, outputDirectory: directory, fileName: manifest?.title, httpHeaders: format.httpHeaders })
        : await downloadStream({ url: format.delivery.kind === 'hls' ? format.delivery.playlistUrl : format.delivery.manifestUrl, outputDirectory: directory, fileName: manifest?.title ?? 'autofox-download' });
      notice = `下载完成：${result.outputPath}`;
    } catch (error) {
      notice = error instanceof Error ? error.message : '下载失败，请重试。';
    } finally { downloading = false; }
  }

  async function refreshTasks() {
    try { tasks = await listTasks(); } catch { /* browser preview */ }
  }

  async function cancelCurrentTask() {
    const current = tasks.find((task) => task.state === 'downloading');
    if (!current) return;
    await cancelTask(current.taskId);
    await refreshTasks();
  }

  function formatSize(size: MediaManifest['formats'][number]['size']) {
    if (size.kind === 'unknown') return '未知';
    const units = ['B', 'KB', 'MB', 'GB']; let value = size.bytes; let unit = 0;
    while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
    return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
  }

  function formatSpeed(bytes: number) { return `${formatSize({ kind: 'exact', bytes })}/s`; }

  async function persistSettings() {
    try { await saveSettings(settings); notice = '设置已保存。'; } catch (error) { notice = error instanceof Error ? error.message : '设置保存失败。'; }
  }
</script>

<svelte:head>
  <title>AutoFox — 媒体下载器</title>
  <meta name="description" content="AutoFox desktop media downloader" />
</svelte:head>

<div class="shell">
  <header class="topbar">
    <div class="brand">
      <img src="/autofox.png" alt="" class="logo" />
      <div><h1>AutoFox</h1><span>桌面媒体下载器</span></div>
    </div>
    <div class="status-pill"><span class="status-dot"></span>本机模式</div>
  </header>

  <div class="url-bar">
    <div class="url-box">
      <Input bind:value={url} onkeydown={(event) => event.key === 'Enter' && startProbe()} placeholder="https://example.com/video" ariaLabel="视频链接" invalid={Boolean(notice && !url.trim())} />
      <Button onclick={startProbe} disabled={probing || downloading}>{probing ? '解析中…' : '开始解析'}<Download size={17} /></Button>
    </div>
    <p class="notice" aria-live="polite">{notice}</p>
  </div>

  <main class="workspace">
    <Tabs items={tabs} bind:value={activeTab}>
      {#snippet panel(id)}
        {#if id === 'download'}
          {#if manifest}
            <div class="panel-heading">
              <h2>{manifest.title}</h2>
              <span class="meta">{manifest.producedBy} · {manifest.backendVersion} · {manifest.formats.length} 个格式</span>
            </div>
            {#if progress}
              <div class="progress-card">
                <div class="progress-line">
                  <span>{progress.totalBytes ? `${Math.round(progress.bytesWritten / progress.totalBytes * 100)}%` : formatSize({ kind: 'exact', bytes: progress.bytesWritten })}</span>
                  <span>{formatSpeed(progress.bytesPerSecond)}</span>
                </div>
                <Progress value={progress.bytesWritten} max={progress.totalBytes ?? progress.bytesWritten} label="下载进度" />
              </div>
            {/if}
            <div class="rows">
              {#each manifest.formats as format (format.formatId)}
                <article class="row">
                  <div><strong>{format.formatId}</strong><small>{format.container} · {format.delivery.kind}{format.video?.width ? ` · ${format.video.width}×${format.video.height}` : ''}</small></div>
                  <div class="row-meta">
                    <Badge variant="outline">{format.video?.codec ?? 'audio'}{format.audio ? ` + ${format.audio.codec}` : ''}</Badge>
                    <span class="size">{formatSize(format.size)}</span>
                    <Button variant="outline" onclick={() => { selectedFormat = format; startDownload(format); }} disabled={probing || downloading}>{downloading && selectedFormat?.formatId === format.formatId ? '下载中…' : '下载'}</Button>
                  </div>
                </article>
              {/each}
            </div>
          {:else}
            <Empty title="还没有解析结果" description="在上方粘贴链接并点击开始解析，可用格式会列在这里。" />
          {/if}
        {:else if id === 'tasks'}
          <div class="panel-heading">
            <h2>任务</h2>
            <Button variant="outline" onclick={refreshTasks}>刷新</Button>
          </div>
          {#if tasks.length === 0}
            <Empty title="还没有下载任务" description="解析链接后选择一个格式开始下载。" />
          {:else}
            <div class="rows">
              {#each tasks as task (task.taskId)}
                <div class="row">
                  <div><strong>{task.kind}</strong><small>{task.state}{task.outputPath ? ` · ${task.outputPath}` : ''}</small></div>
                  {#if task.state === 'downloading'}<Button variant="destructive" onclick={cancelCurrentTask}>取消</Button>{/if}
                </div>
              {/each}
            </div>
          {/if}
        {:else if id === 'backend'}
          <div class="panel-heading">
            <h2>解析后端</h2>
            <Button variant="outline" onclick={checkBackend} disabled={probing}>{probing ? '检测中…' : '重新检测'}</Button>
          </div>
          <div class="rows">
            <div class="row">
              <div class="named"><span class:ready={backend?.ytdlpReady} class="backend-dot"></span><div><strong>yt-dlp</strong><small>随包 · 同目录 yt-dlp.exe</small></div></div>
              <Badge variant={backend?.ytdlpReady ? 'default' : 'outline'}>{backend?.ytdlpReady ? backend.version ?? '就绪' : '未检测'}</Badge>
            </div>
            <div class="row">
              <div class="named"><span class:ready={backend?.ffmpegReady} class="backend-dot"></span><div><strong>ffmpeg</strong><small>随包 · lib/ffmpeg.exe</small></div></div>
              <Badge variant={backend?.ffmpegReady ? 'default' : 'outline'}>{backend?.ffmpegReady ? '就绪' : '未检测'}</Badge>
            </div>
          </div>
        {:else if id === 'settings'}
          <div class="panel-heading"><h2>设置</h2></div>
          <div class="fields">
            <label>解析策略
              <select value={settings.extractorPolicy} onchange={(event) => (settings.extractorPolicy = (event.currentTarget as HTMLSelectElement).value as Settings['extractorPolicy'])}>
                <option value="auto">自动</option>
                <option value="ytdlpOnly">仅 yt-dlp</option>
                <option value="nativeOnly">仅 Rust</option>
              </select>
            </label>
            <label>日志级别
              <select value={settings.logLevel} onchange={(event) => (settings.logLevel = (event.currentTarget as HTMLSelectElement).value as Settings['logLevel'])}>
                <option value="ERROR">ERROR</option>
                <option value="WARN">WARN</option>
                <option value="INFO">INFO</option>
                <option value="DEBUG">DEBUG</option>
                <option value="TRACE">TRACE</option>
              </select>
            </label>
            <label>yt-dlp 路径
              <input value={settings.ytdlpPath ?? ''} onchange={(event) => (settings.ytdlpPath = (event.currentTarget as HTMLInputElement).value || undefined)} placeholder="默认使用同目录 yt-dlp.exe" />
            </label>
            <label>ffmpeg 路径
              <input value={settings.ffmpegPath ?? ''} onchange={(event) => (settings.ffmpegPath = (event.currentTarget as HTMLInputElement).value || undefined)} placeholder="默认使用 lib/ffmpeg.exe" />
            </label>
          </div>
          <Button onclick={persistSettings}>保存设置</Button>
        {:else}
          <div class="panel-heading"><h2>关于 AutoFox</h2></div>
          <p class="about-lead">统一接入 yt-dlp 与 Rust 解析后端，下载、断点续传和队列都在本机完成。</p>
          <div class="about-grid">
            <article><div class="about-icon orange"><Download size={18} /></div><h3>一个协议，多种后端</h3><p>随包 yt-dlp、用户自定义 CLI 与 Rust 后端都通过同一份契约接入。</p></article>
            <article><div class="about-icon blue"><Activity size={18} /></div><h3>本机优先</h3><p>链接、队列和设置留在你的设备上，不经过任何服务器。</p></article>
            <article><div class="about-icon green"><FolderOpen size={18} /></div><h3>可恢复下载</h3><p><code>.part</code> 临时文件、HTTP Range 续传与 ffmpeg 合流按任务状态推进。</p></article>
          </div>
          <div class="rows about-facts">
            <div class="row"><strong>版本</strong><span class="meta">0.1.0 · 阶段 2 下载</span></div>
            <div class="row"><strong>运行时</strong><span class="meta">Rust + Tauri 2 · Svelte 5</span></div>
            <div class="row"><strong>第三方声明</strong><span class="meta">仓库 THIRD_PARTY_NOTICES.md</span></div>
          </div>
        {/if}
      {/snippet}
    </Tabs>
  </main>

  <LogBar bind:expanded={logExpanded} />

  <footer><span>AutoFox 0.1.0</span><span>本地运行，无云端依赖</span></footer>
</div>

<style>
  /* Exactly one viewport, never more: the log bar is pinned above the footer,
     so growth has to be absorbed by the workspace scrolling rather than by
     pushing the footer and the log bar off-screen. */
  .shell { height: 100dvh; display: flex; flex-direction: column; overflow: hidden; }
  .topbar, .url-bar, footer { flex: 0 0 auto; }
  .topbar { min-height: 3.75rem; display: flex; align-items: center; justify-content: space-between; gap: var(--space-md); padding: 0 var(--space-xl); border-bottom: 1px solid var(--border); }
  .brand { display: flex; align-items: center; gap: var(--space-sm); }
  .brand h1 { margin: 0; font-family: var(--font-display); font-size: 1.05rem; font-weight: 700; letter-spacing: -.03em; }
  .brand span { display: block; margin-top: .1rem; color: var(--muted-foreground); font-size: .68rem; }
  .logo { width: 2.25rem; height: 2.25rem; object-fit: contain; border-radius: .5rem; }
  .status-pill { display: flex; align-items: center; gap: .4rem; min-height: 2.25rem; padding: .38rem .7rem; border: 1px solid var(--border); border-radius: 999px; color: var(--muted-foreground); font-size: .72rem; }
  .status-dot, .backend-dot { width: .45rem; height: .45rem; border-radius: 50%; background: var(--muted-foreground); }
  .status-dot { background: var(--success); }
  .backend-dot.ready { background: var(--success); }

  /* The URL bar is the app's primary action, so it sits above the tabs and
     stays reachable from every panel. */
  .url-bar { padding: var(--space-md) var(--space-xl) var(--space-xs); border-bottom: 1px solid var(--border); background: var(--card); }
  .url-box { display: flex; align-items: center; gap: var(--space-xs); }
  /* Reserved line so a notice appearing does not push the tabs down. */
  .notice { min-height: 1lh; margin: .3rem 0 0; color: var(--muted-foreground); font-size: .72rem; }

  .workspace { flex: 1; min-height: 0; overflow: auto; padding: var(--space-lg) var(--space-xl) var(--space-md); }
  .panel-heading { display: flex; align-items: center; justify-content: space-between; gap: var(--space-md); margin-bottom: var(--space-sm); }
  .panel-heading h2 { margin: 0; font-family: var(--font-display); font-size: 1rem; font-weight: 700; letter-spacing: -.02em; }
  .meta { color: var(--muted-foreground); font-size: .72rem; font-variant-numeric: tabular-nums; }

  .rows { display: grid; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: var(--space-md); min-height: 3.25rem; padding: .7rem 0; border-top: 1px solid var(--border); font-variant-numeric: tabular-nums; }
  .row strong { display: block; font-size: .82rem; }
  .row small { display: block; color: var(--muted-foreground); font-size: .7rem; }
  .row-meta { display: flex; align-items: center; gap: var(--space-sm); }
  .size { color: var(--muted-foreground); font-size: .72rem; font-variant-numeric: tabular-nums; }
  .named { display: flex; align-items: center; gap: .65rem; }

  .progress-card { padding: var(--space-sm) 0 var(--space-md); }
  .progress-line { display: flex; justify-content: space-between; margin-bottom: .4rem; font-family: var(--font-mono); font-size: .72rem; font-variant-numeric: tabular-nums; }

  .fields { display: grid; gap: var(--space-sm); max-width: 34rem; margin-bottom: var(--space-md); }
  .fields label { display: grid; gap: .3rem; font-size: .75rem; font-weight: 650; }
  .fields select, .fields input { width: 100%; min-height: 2.75rem; padding: 0 .7rem; border: 1px solid var(--input); border-radius: .6rem; background: var(--background); color: var(--foreground); font: inherit; font-weight: 400; outline: 2px solid transparent; outline-offset: 1px; }
  .fields select:focus-visible, .fields input:focus-visible { outline-color: var(--ring); }
  .fields input::placeholder { color: var(--muted-foreground); }
  @media (hover: hover) {
    .fields select:hover, .fields input:hover { background: var(--muted); }
  }

  .about-lead { max-width: 46ch; margin: 0 0 var(--space-lg); font-size: .95rem; line-height: 1.55; }
  .about-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr)); gap: var(--space-lg) var(--space-md); margin-bottom: var(--space-xl); }
  .about-grid h3 { margin: var(--space-xs) 0 .25rem; font-size: .85rem; font-weight: 700; }
  .about-grid p { margin: 0; max-width: 34ch; color: var(--muted-foreground); font-size: .75rem; line-height: 1.5; }
  .about-grid code { font-family: var(--font-mono); font-size: .72rem; }
  .about-icon { display: grid; place-items: center; width: 2.25rem; height: 2.25rem; border-radius: .55rem; }
  .orange { color: var(--feature-orange-ink); background: var(--feature-orange-bg); }
  .blue { color: var(--feature-blue-ink); background: var(--feature-blue-bg); }
  .green { color: var(--feature-green-ink); background: var(--feature-green-bg); }
  .about-facts { max-width: 34rem; }

  footer { display: flex; justify-content: space-between; gap: var(--space-md); padding: var(--space-xs) var(--space-xl); border-top: 1px solid var(--border); color: var(--muted-foreground); font-size: .65rem; }

  @media (max-width: 680px) {
    .topbar, .url-bar, footer { padding-left: var(--space-md); padding-right: var(--space-md); }
    .workspace { padding: var(--space-md) var(--space-md) var(--space-sm); }
    .url-box { flex-wrap: wrap; }
    .url-box :global(.ui-button) { width: 100%; }
    footer { flex-direction: column; gap: var(--space-2xs); }
  }
</style>
