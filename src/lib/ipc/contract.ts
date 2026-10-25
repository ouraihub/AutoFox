/**
 * Parsers for every shape the Rust backend returns.
 *
 * One parser per `docs/03-后端契约.md` type. They exist because `invoke<T>()`
 * asserts rather than checks: `ErrorCode.NotFound` once reached the frontend as
 * a string no TypeScript union listed, and nothing failed until a component
 * rendered `undefined`.
 *
 * Field names here are the camelCase ones `serde(rename_all)` emits, so a
 * missed `rename_all` on a new Rust variant fails loudly at the boundary.
 */

import type {
  BackendStatus,
  DeliveryPlan,
  DownloadOption,
  DownloadResult,
  LogEntry,
  MediaFormat,
  MediaManifest,
  MuxResult,
  Settings,
  SizeHint,
  TaskProgress,
  TaskSummary,
} from './client';
import { array, bool, literal, num, object, optional, str, stringRecord } from './validate';

const LOG_LEVELS = ['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'] as const;
const TASK_STATES = ['downloading', 'paused', 'done', 'failed', 'cancelled'] as const;
const EXTRACTOR_POLICIES = ['auto', 'ytdlpOnly', 'nativeOnly'] as const;

export function parseBackendStatus(value: unknown, path = 'backendStatus'): BackendStatus {
  const row = object(value, path);
  return {
    ytdlpReady: bool(row.ytdlpReady, `${path}.ytdlpReady`),
    ffmpegReady: bool(row.ffmpegReady, `${path}.ffmpegReady`),
    ffprobeReady: bool(row.ffprobeReady, `${path}.ffprobeReady`),
    version: optional(row.version, `${path}.version`, str),
    ytdlpPath: str(row.ytdlpPath, `${path}.ytdlpPath`),
    ffmpegPath: str(row.ffmpegPath, `${path}.ffmpegPath`),
  };
}

/**
 * `SizeHint` is an externally tagged enum: `{ kind: 'exact', bytes }` or
 * `{ kind: 'unknown' }`. The `bytes` field is absent in the unknown case, so
 * reading it unconditionally is what produced `NaN` byte totals.
 */
function parseSizeHint(value: unknown, path: string): SizeHint {
  const row = object(value, path);
  const kind = literal(row.kind, `${path}.kind`, ['exact', 'approx', 'unknown'] as const);
  if (kind === 'unknown') return { kind };
  return { kind, bytes: num(row.bytes, `${path}.bytes`) };
}

function parseFormat(value: unknown, path: string): MediaFormat {
  const row = object(value, path);
  const delivery = object(row.delivery, `${path}.delivery`);
  const kind = literal(delivery.kind, `${path}.delivery.kind`, [
    'direct',
    'hls',
    'dash',
  ] as const);

  return {
    formatId: str(row.formatId, `${path}.formatId`),
    delivery:
      kind === 'direct'
        ? { kind, url: str(delivery.url, `${path}.delivery.url`) }
        : kind === 'hls'
          ? { kind, playlistUrl: str(delivery.playlistUrl, `${path}.delivery.playlistUrl`) }
          : { kind, manifestUrl: str(delivery.manifestUrl, `${path}.delivery.manifestUrl`) },
    container: str(row.container, `${path}.container`),
    video: optional(row.video, `${path}.video`, (raw, at) => {
      const spec = object(raw, at);
      return {
        codec: str(spec.codec, `${at}.codec`),
        width: optional(spec.width, `${at}.width`, num),
        height: optional(spec.height, `${at}.height`, num),
        fps: optional(spec.fps, `${at}.fps`, num),
        bitrateKbps: optional(spec.bitrateKbps, `${at}.bitrateKbps`, num),
      };
    }),
    audio: optional(row.audio, `${path}.audio`, (raw, at) => {
      const spec = object(raw, at);
      return {
        codec: str(spec.codec, `${at}.codec`),
        channels: optional(spec.channels, `${at}.channels`, num),
        sampleRate: optional(spec.sampleRate, `${at}.sampleRate`, num),
        bitrateKbps: optional(spec.bitrateKbps, `${at}.bitrateKbps`, num),
      };
    }),
    size: parseSizeHint(row.size, `${path}.size`),
    httpHeaders: stringRecord(row.httpHeaders, `${path}.httpHeaders`),
  };
}

function parsePlan(value: unknown, path: string): DeliveryPlan {
  const row = object(value, path);
  const kind = literal(row.kind, `${path}.kind`, ['progressive', 'merge', 'manifest'] as const);
  if (kind === 'merge') {
    return {
      kind,
      videoUrl: str(row.videoUrl, `${path}.videoUrl`),
      audioUrl: str(row.audioUrl, `${path}.audioUrl`),
      videoFormatId: str(row.videoFormatId, `${path}.videoFormatId`),
      audioFormatId: str(row.audioFormatId, `${path}.audioFormatId`),
    };
  }
  return {
    kind,
    formatId: str(row.formatId, `${path}.formatId`),
    url: str(row.url, `${path}.url`),
  };
}

function parseOption(value: unknown, path: string): DownloadOption {
  const row = object(value, path);
  return {
    optionId: str(row.optionId, `${path}.optionId`),
    label: str(row.label, `${path}.label`),
    container: str(row.container, `${path}.container`),
    detail: str(row.detail, `${path}.detail`),
    size: parseSizeHint(row.size, `${path}.size`),
    height: optional(row.height, `${path}.height`, num),
    plan: parsePlan(row.plan, `${path}.plan`),
  };
}

export function parseManifest(value: unknown, path = 'manifest'): MediaManifest {
  const row = object(value, path);
  return {
    sourceId: str(row.sourceId, `${path}.sourceId`),
    title: str(row.title, `${path}.title`),
    durationSecs: optional(row.durationSecs, `${path}.durationSecs`, num),
    formats: array(row.formats, `${path}.formats`, parseFormat),
    downloadOptions: array(row.downloadOptions, `${path}.downloadOptions`, parseOption),
    entries: array(row.entries, `${path}.entries`, (raw, at) => {
      const entry = object(raw, at);
      return {
        sourceId: str(entry.sourceId, `${at}.sourceId`),
        title: str(entry.title, `${at}.title`),
        url: optional(entry.url, `${at}.url`, str),
        durationSecs: optional(entry.durationSecs, `${at}.durationSecs`, num),
      };
    }),
    producedBy: str(row.producedBy, `${path}.producedBy`),
    backendVersion: str(row.backendVersion, `${path}.backendVersion`),
  };
}

export function parseDownloadResult(value: unknown, path = 'downloadResult'): DownloadResult {
  const row = object(value, path);
  return {
    taskId: str(row.taskId, `${path}.taskId`),
    outputPath: str(row.outputPath, `${path}.outputPath`),
    bytesWritten: num(row.bytesWritten, `${path}.bytesWritten`),
  };
}

export function parseMuxResult(value: unknown, path = 'muxResult'): MuxResult {
  const row = object(value, path);
  return { outputPath: str(row.outputPath, `${path}.outputPath`) };
}

export function parseLogEntry(value: unknown, path = 'logEntry'): LogEntry {
  const row = object(value, path);
  return {
    seq: num(row.seq, `${path}.seq`),
    time: str(row.time, `${path}.time`),
    level: str(row.level, `${path}.level`),
    target: str(row.target, `${path}.target`),
    message: str(row.message, `${path}.message`),
  };
}

export function parseLogEntries(value: unknown, path = 'logs'): LogEntry[] {
  return array(value, path, parseLogEntry);
}

export function parseSettings(value: unknown, path = 'settings'): Settings {
  const row = object(value, path);
  return {
    schemaVersion: num(row.schemaVersion, `${path}.schemaVersion`),
    downloadDirectory: optional(row.downloadDirectory, `${path}.downloadDirectory`, str),
    extractorPolicy: literal(row.extractorPolicy, `${path}.extractorPolicy`, EXTRACTOR_POLICIES),
    allowPathLookup: bool(row.allowPathLookup, `${path}.allowPathLookup`),
    ytdlpPath: optional(row.ytdlpPath, `${path}.ytdlpPath`, str),
    ffmpegPath: optional(row.ffmpegPath, `${path}.ffmpegPath`, str),
    logLevel: literal(row.logLevel, `${path}.logLevel`, LOG_LEVELS),
  };
}

export function parseTaskSummary(value: unknown, path = 'task'): TaskSummary {
  const row = object(value, path);
  return {
    taskId: str(row.taskId, `${path}.taskId`),
    kind: str(row.kind, `${path}.kind`),
    state: literal(row.state, `${path}.state`, TASK_STATES),
    outputPath: optional(row.outputPath, `${path}.outputPath`, str),
    errorKey: optional(row.errorKey, `${path}.errorKey`, str),
  };
}

export function parseTaskSummaries(value: unknown, path = 'tasks'): TaskSummary[] {
  return array(value, path, parseTaskSummary);
}

/**
 * Validates a `task://progress` event payload.
 *
 * Events bypass `invoke` entirely, so this is the only check standing between a
 * renamed Rust field and a progress bar stuck at zero.
 */
export function parseTaskProgress(value: unknown, path = 'progress'): TaskProgress {
  const row = object(value, path);
  return {
    taskId: str(row.taskId, `${path}.taskId`),
    bytesWritten: num(row.bytesWritten, `${path}.bytesWritten`),
    totalBytes: optional(row.totalBytes, `${path}.totalBytes`, num),
    bytesPerSecond: num(row.bytesPerSecond, `${path}.bytesPerSecond`),
    percent: optional(row.percent, `${path}.percent`, num),
  };
}
