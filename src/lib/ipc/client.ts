import { invoke, isTauri } from '@tauri-apps/api/core';

export type BackendStatus = {
  ytdlpReady: boolean;
  ffmpegReady: boolean;
  ffprobeReady: boolean;
  version?: string;
  ffmpegPath: string;
};

export type ProbeRequest = { url: string };

export type SizeHint = { kind: 'exact' | 'approx'; bytes: number } | { kind: 'unknown' };
export type MediaFormat = {
  formatId: string;
  delivery: { kind: 'direct'; url: string } | { kind: 'hls'; playlistUrl: string } | { kind: 'dash'; manifestUrl: string };
  container: string;
  video?: { codec: string; width?: number; height?: number; fps?: number; bitrateKbps?: number };
  audio?: { codec: string; channels?: number; sampleRate?: number; bitrateKbps?: number };
  size: SizeHint;
  httpHeaders: Record<string, string>;
};
export type DeliveryPlan =
  | { kind: 'progressive'; formatId: string; url: string }
  | { kind: 'merge'; videoUrl: string; audioUrl: string; videoFormatId: string; audioFormatId: string }
  | { kind: 'manifest'; formatId: string; url: string };
export type DownloadOption = {
  optionId: string;
  label: string;
  container: string;
  detail: string;
  size: SizeHint;
  height?: number;
  plan: DeliveryPlan;
};
export type MediaManifest = {
  sourceId: string;
  title: string;
  durationSecs?: number;
  formats: MediaFormat[];
  downloadOptions: DownloadOption[];
  producedBy: string;
  backendVersion: string;
};


export async function probeUrl(url: string): Promise<MediaManifest> {
  if (!isDesktop) {
    throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  }
  return invoke<MediaManifest>('probe_url', { request: { url } });
}

export type DownloadRequest = {
  url: string;
  formatId: string;
  outputDirectory: string;
  fileName?: string;
  httpHeaders: Record<string, string>;
};

export type DownloadResult = { taskId: string; outputPath: string; bytesWritten: number };

export async function downloadDirect(request: DownloadRequest): Promise<DownloadResult> {
  if (!isDesktop) {
    throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  }
  return invoke<DownloadResult>('download_direct', { request });
}
export type StreamRequest = { url: string; outputDirectory: string; fileName: string; durationSecs?: number };
export async function downloadStream(request: StreamRequest): Promise<MuxResult> {
  if (!isDesktop) throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  return invoke<MuxResult>('download_stream', { request });
}

export type MuxRequest = { videoUrl: string; audioUrl: string; outputDirectory: string; fileName: string; container: string; httpHeaders: Record<string, string>; durationSecs?: number };
export type MuxResult = { outputPath: string };
export async function muxStreams(request: MuxRequest): Promise<MuxResult> {
  if (!isDesktop) throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  return invoke<MuxResult>('mux_streams', { request });
}

export type LogLevel = 'ERROR' | 'WARN' | 'INFO' | 'DEBUG' | 'TRACE';
export type LogEntry = { seq: number; time: string; level: string; target: string; message: string };
export async function readLogs(limit = 300): Promise<LogEntry[]> { if (!isDesktop) return []; return invoke<LogEntry[]>('read_logs', { limit }); }
export async function clearLogs(): Promise<void> { if (!isDesktop) return; await invoke('clear_logs'); }

export type Settings = { schemaVersion: number; downloadDirectory?: string; extractorPolicy: 'auto' | 'ytdlpOnly' | 'nativeOnly'; allowPathLookup: boolean; ytdlpPath?: string; ffmpegPath?: string; logLevel: LogLevel };
export async function getSettings(): Promise<Settings> {
  if (!isDesktop) throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  return invoke<Settings>('get_settings');
}
export async function saveSettings(settings: Settings): Promise<void> {
  if (!isDesktop) throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  await invoke('save_settings', { settings });
}

export type TaskState = 'downloading' | 'paused' | 'done' | 'failed' | 'cancelled';
// Payload of `task://progress`, emitted by both the direct download loop and
// the ffmpeg muxer. `percent` is the only field both can fill meaningfully:
// the muxer knows elapsed media time, not the final byte total.
export type TaskProgress = { taskId: string; bytesWritten: number; totalBytes?: number; bytesPerSecond: number; percent?: number };
export type TaskSummary = { taskId: string; kind: string; state: TaskState; outputPath?: string; errorKey?: string };

export async function listTasks(): Promise<TaskSummary[]> {
  if (!isDesktop) throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  return invoke<TaskSummary[]>('list_tasks');
}

export async function cancelTask(taskId: string): Promise<void> {
  if (!isDesktop) throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  await invoke('cancel_task', { taskId });
}

export async function pauseTask(taskId: string): Promise<void> {
  if (!isDesktop) throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  await invoke('pause_task', { taskId });
}

export async function resumeTask(taskId: string): Promise<void> {
  if (!isDesktop) throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  await invoke('resume_task', { taskId });
}

export async function removeTask(taskId: string): Promise<void> {
  if (!isDesktop) throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  await invoke('remove_task', { taskId });
}

 export type ErrorCode =
  | 'Validation' | 'NoBackendAvailable' | 'BackendUnavailable' | 'BackendProtocol'
  | 'Unsupported' | 'ExtractionFailed' | 'AuthRequired' | 'GeoRestricted'
  | 'FormatUnavailable' | 'Network' | 'TimedOut' | 'Cancelled'
  | 'DiskFull' | 'AtomicWrite' | 'MuxFailed' | 'MuxerMissing' | 'Busy' | 'Internal';

export type IpcError = {
  code: ErrorCode;
  messageKey: string;
  field?: string;
  retryable: boolean;
  detail: string;
};

/** Tauri rejects with the serialized `IpcError`, which is a plain object and
 * fails `instanceof Error`. Without this every backend diagnosis collapsed to
 * a generic fallback string and `detail` never reached the user. */
export function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && error !== null && 'detail' in error) {
    const { detail } = error as IpcError;
    if (typeof detail === 'string' && detail.length > 0) return detail;
  }
  if (typeof error === 'string' && error.length > 0) return error;
  return fallback;
}

export const isDesktop = isTauri();

export async function getBackendStatus(): Promise<BackendStatus> {
  if (!isDesktop) {
    throw new Error('AutoFox IPC is available only in the Tauri desktop runtime');
  }
  return invoke<BackendStatus>('get_backend_status');
}
