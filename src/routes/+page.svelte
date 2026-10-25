<script lang="ts">
  import { onMount } from 'svelte';
  import { Download, FolderOpen, Activity } from '@lucide/svelte';
  import { Button, Input, Tabs, LogBar, type TabItem } from '$lib/components/ui';
  import DownloadPanel from '$lib/components/panels/download-panel.svelte';
  import TaskPanel from '$lib/components/panels/task-panel.svelte';
  import BackendPanel from '$lib/components/panels/backend-panel.svelte';
  import { downloadDir } from '@tauri-apps/api/path';
  import { open } from '@tauri-apps/plugin-dialog';
  import { listen } from '@tauri-apps/api/event';
  import { cancelTask, downloadDirect, downloadStream, errorMessage, getBackendStatus, getSettings, isDesktop, listTasks, muxStreams, probeUrl, saveSettings, type BackendStatus, type DownloadOption, type MediaManifest, type Settings, type TaskProgress, type TaskSummary } from '$lib/ipc/client';

  let url = $state('');
  let backend = $state<BackendStatus | null>(null);
  let manifest = $state<MediaManifest | null>(null);
  let probing = $state(false);
  let downloading = $state(false);
  let selectedOption = $state<DownloadOption | null>(null);
  let progress = $state<TaskProgress | null>(null);
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
    // `listen` resolves asynchronously. Without the `disposed` latch an unmount
    // that beats the first resolve leaves the disposer unrecorded, the cleanup
    // no-ops, and the listener outlives the component.
    let disposed = false;
    const disposers: Array<() => void> = [];
    const track = (dispose: () => void) => {
      if (disposed) dispose();
      else disposers.push(dispose);
    };
    listen<TaskProgress>('task://progress', (event) => {
      progress = event.payload;
    })
      .then(track)
      .catch(() => { /* backend not ready */ });
    // The backend marks tasks terminal through `task://state`. Without this the
    // queue kept showing `downloading` until the user hit refresh by hand.
    listen('task://state', () => { void refreshTasks(); })
      .then(track)
      .catch(() => { /* backend not ready */ });
    listTasks().then((items) => { tasks = items; }).catch(() => { /* backend not ready */ });
    // Settings now live in a tab rather than behind a dialog, so they load with
    // the page instead of on a click.
    getSettings().then((value) => { settings = value; }).catch(() => { /* backend not ready */ });
    return () => {
      disposed = true;
      for (const dispose of disposers) dispose();
      disposers.length = 0;
    };
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
    selectedOption = null;
    notice = '正在调用 yt-dlp 解析格式…';
    // A probe started from another tab has its result on the download tab, so
    // move there rather than leaving the user watching an unrelated panel.
    activeTab = 'download';
    try {
      manifest = await probeUrl(value);
      notice = `解析完成：${manifest.downloadOptions.length} 个可选画质，后端 ${manifest.backendVersion}`;
    } catch (error) {
      notice = errorMessage(error, '解析失败，请检查链接或后端状态。');
    } finally { probing = false; }
  }

  // The configured directory wins; the system Downloads folder is only the
  // fallback for a fresh install that never opened the settings tab. Trimming
  // matters because the field is free text: a stray space would otherwise be
  // sent as a path component.
  async function resolveDownloadDirectory(): Promise<string> {
    const configured = settings.downloadDirectory?.trim();
    return configured || (await downloadDir());
  }

  async function startDownload(option: DownloadOption) {
    downloading = true;
    selectedOption = option;
    progress = null;
    notice = option.plan.kind === 'progressive' ? '正在下载…' : '正在用 ffmpeg 合流…';
    try {
      const directory = await resolveDownloadDirectory();
      const fileName = `${manifest?.title ?? 'autofox-download'} ${option.label}`;
      let outputPath: string;
      if (option.plan.kind === 'progressive') {
        const result = await downloadDirect({
          url: option.plan.url,
          formatId: option.plan.formatId,
          outputDirectory: directory,
          fileName,
          httpHeaders: headersFor(option.plan.formatId),
        });
        outputPath = result.outputPath;
      } else if (option.plan.kind === 'merge') {
        // Video-only and audio-only streams are useless on their own, so the
        // two URLs go straight to ffmpeg's stream-copy path.
        const result = await muxStreams({
          videoUrl: option.plan.videoUrl,
          audioUrl: option.plan.audioUrl,
          outputDirectory: directory,
          fileName,
          container: option.container,
          httpHeaders: headersFor(option.plan.videoFormatId),
          durationSecs: manifest?.durationSecs,
        });
        outputPath = result.outputPath;
      } else {
        const result = await downloadStream({ url: option.plan.url, outputDirectory: directory, fileName, durationSecs: manifest?.durationSecs });
        outputPath = result.outputPath;
      }
      notice = `下载完成：${outputPath}`;
    } catch (error) {
      notice = errorMessage(error, '下载失败，请重试。');
    } finally { downloading = false; }
  }

  // The plan carries only URLs, so headers come from the raw row it came from.
  function headersFor(formatId: string): Record<string, string> {
    return manifest?.formats.find((format) => format.formatId === formatId)?.httpHeaders ?? {};
  }

  async function refreshTasks() {
    try { tasks = await listTasks(); } catch { /* browser preview */ }
  }

  // Takes the row's own id. `cancelCurrentTask` used to cancel "the first
  // downloading task", which stopped the wrong one whenever two ran at once,
  // and the limit is four.
  async function cancelTaskById(taskId: string) {
    try {
      await cancelTask(taskId);
    } catch (error) {
      notice = errorMessage(error, '取消失败，请重试。');
    }
    await refreshTasks();
  }

  async function persistSettings() {
    try { await saveSettings(settings); notice = '设置已保存。'; } catch (error) { notice = errorMessage(error, '设置保存失败。'); }
  }

  // The picker is a convenience over the text field, not a replacement: the
  // field stays editable so a path can be pasted. `open` resolves to `null`
  // when the user cancels, which must leave the current value untouched.
  async function chooseDownloadDirectory() {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        defaultPath: settings.downloadDirectory ?? (await downloadDir()),
      });
      if (typeof selected === 'string') settings.downloadDirectory = selected;
    } catch (error) {
      notice = errorMessage(error, '无法打开目录选择器。');
    }
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
          <DownloadPanel
            {manifest}
            {progress}
            busy={probing || downloading}
            activeOptionId={selectedOption?.optionId}
            onDownload={startDownload}
          />
        {:else if id === 'tasks'}
          <TaskPanel {tasks} onRefresh={refreshTasks} onCancel={cancelTaskById} />
        {:else if id === 'backend'}
          <BackendPanel {backend} busy={probing} onRecheck={checkBackend} />
        {:else if id === 'settings'}
          <div class="panel-heading"><h2>设置</h2></div>
          <div class="fields">
            <div class="field-group">
              <label for="download-directory">下载目录</label>
              <div class="path-field">
                <input id="download-directory" value={settings.downloadDirectory ?? ''} onchange={(event) => (settings.downloadDirectory = (event.currentTarget as HTMLInputElement).value.trim() || undefined)} placeholder="默认使用系统下载目录" />
                <Button variant="outline" onclick={chooseDownloadDirectory}>浏览…</Button>
              </div>
            </div>
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
  .status-dot { width: .45rem; height: .45rem; border-radius: 50%; background: var(--success); }

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

  .fields { display: grid; gap: var(--space-sm); max-width: 34rem; margin-bottom: var(--space-md); }
  .fields label { display: grid; gap: .3rem; font-size: .75rem; font-weight: 650; }
  .fields select, .fields input { width: 100%; min-height: 2.75rem; padding: 0 .7rem; border: 1px solid var(--input); border-radius: .6rem; background: var(--background); color: var(--foreground); font: inherit; font-weight: 400; outline: 2px solid transparent; outline-offset: 1px; }
  .fields select:focus-visible, .fields input:focus-visible { outline-color: var(--ring); }
  .fields input::placeholder { color: var(--muted-foreground); }
  /* The directory row pairs free-text input with a picker button, so its label
     sits outside the control. This group restores the label-to-input spacing
     that `.fields label` gives the plain fields. */
  .field-group { display: grid; gap: .3rem; }
  .field-group label { font-size: .75rem; font-weight: 650; }
  .path-field { display: flex; gap: var(--space-xs); align-items: center; }
  .path-field input { flex: 1 1 auto; min-width: 0; }
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
