import { describe, expect, it } from 'vitest';

import {
  parseBackendStatus,
  parseDownloadResult,
  parseLogEntries,
  parseManifest,
  parseMuxResult,
  parseSettings,
  parseTaskProgress,
  parseTaskSummaries,
} from './contract';
import { ContractError } from './validate';
// Written by `ipc_fixture_matches_rust_serialization` in `src-tauri/src/ipc.rs`
// from the real serde types, so these are the bytes the backend actually sends.
import fixture from './fixtures/ipc-payloads.json';

/** Minimal payload the Rust side actually emits for a progressive-only video. */
function manifestPayload() {
  return {
    sourceId: 'abc',
    title: 'Example',
    durationSecs: 12.5,
    formats: [
      {
        formatId: '18',
        delivery: { kind: 'direct', url: 'https://cdn.example/v.mp4' },
        container: 'mp4',
        video: { codec: 'h264', width: 640, height: 360 },
        audio: { codec: 'aac' },
        size: { kind: 'exact', bytes: 1024 },
        httpHeaders: { 'User-Agent': 'AutoFox' },
      },
    ],
    downloadOptions: [
      {
        optionId: 'o1',
        label: '360p',
        container: 'mp4',
        detail: 'h264 + aac',
        size: { kind: 'exact', bytes: 1024 },
        plan: { kind: 'progressive', formatId: '18', url: 'https://cdn.example/v.mp4' },
      },
    ],
    entries: [],
    producedBy: 'builtin-ytdlp',
    backendVersion: '2025.01.01',
  };
}

describe('parseManifest', () => {
  it('accepts the payload shape the backend emits', () => {
    const manifest = parseManifest(manifestPayload());
    expect(manifest.title).toBe('Example');
    expect(manifest.formats[0]?.delivery).toEqual({
      kind: 'direct',
      url: 'https://cdn.example/v.mp4',
    });
    expect(manifest.downloadOptions[0]?.plan.kind).toBe('progressive');
  });

  /// The drift that motivated this layer: a renamed Rust field used to surface
  /// as `undefined` inside a component instead of an error at the boundary.
  it('names the exact path of a missing required field', () => {
    const payload = manifestPayload();
    delete (payload.downloadOptions[0] as Record<string, unknown>).label;

    expect(() => parseManifest(payload)).toThrow(ContractError);
    expect(() => parseManifest(payload)).toThrow(
      'manifest.downloadOptions[0].label: expected string, got undefined',
    );
  });

  it('rejects a snake_case delivery key that a missed rename_all would emit', () => {
    const payload = manifestPayload();
    payload.formats[0]!.delivery = {
      kind: 'hls',
      playlist_url: 'https://cdn.example/p.m3u8',
    } as never;

    expect(() => parseManifest(payload)).toThrow(
      'manifest.formats[0].delivery.playlistUrl: expected string, got undefined',
    );
  });

  /// `SizeHint::Unknown` carries no `bytes`. Reading it unconditionally is what
  /// produced `NaN` totals, so the unknown arm must not require the field.
  it('accepts an unknown size without a bytes field', () => {
    const payload = manifestPayload();
    payload.formats[0]!.size = { kind: 'unknown' } as never;
    expect(parseManifest(payload).formats[0]?.size).toEqual({ kind: 'unknown' });
  });

  it('rejects a non-finite number rather than passing NaN to the UI', () => {
    const payload = manifestPayload();
    payload.formats[0]!.size = { kind: 'exact', bytes: Number.NaN };
    expect(() => parseManifest(payload)).toThrow('expected finite number');
  });
});

describe('parseSettings', () => {
  it('rejects a policy value outside the union', () => {
    expect(() =>
      parseSettings({
        schemaVersion: 1,
        extractorPolicy: 'someFuturePolicy',
        allowPathLookup: false,
        logLevel: 'INFO',
      }),
    ).toThrow('settings.extractorPolicy: expected one of auto | ytdlpOnly | nativeOnly');
  });

  it('treats an absent optional path as undefined', () => {
    const settings = parseSettings({
      schemaVersion: 1,
      extractorPolicy: 'auto',
      allowPathLookup: true,
      logLevel: 'DEBUG',
    });
    expect(settings.ytdlpPath).toBeUndefined();
    expect(settings.downloadDirectory).toBeUndefined();
  });

  /// `serde` omits `None`, but an explicit null must mean the same thing.
  it('treats an explicit null optional as undefined', () => {
    const settings = parseSettings({
      schemaVersion: 1,
      downloadDirectory: null,
      extractorPolicy: 'auto',
      allowPathLookup: true,
      logLevel: 'DEBUG',
    });
    expect(settings.downloadDirectory).toBeUndefined();
  });
});

describe('parseTaskSummaries', () => {
  it('rejects a task state outside the union', () => {
    expect(() =>
      parseTaskSummaries([{ taskId: 't1', kind: 'direct', state: 'queued' }]),
    ).toThrow('tasks[0].state: expected one of downloading | paused | done | failed | cancelled');
  });

  it('reports the index of the offending row', () => {
    expect(() =>
      parseTaskSummaries([
        { taskId: 't1', kind: 'direct', state: 'done' },
        { taskId: 't2', kind: 'direct' },
      ]),
    ).toThrow('tasks[1].state');
  });
});

describe('parseTaskProgress', () => {
  it('accepts a muxer payload that knows percent but not totals', () => {
    const progress = parseTaskProgress({
      taskId: 't1',
      bytesWritten: 0,
      bytesPerSecond: 0,
      percent: 42,
    });
    expect(progress.totalBytes).toBeUndefined();
    expect(progress.percent).toBe(42);
  });
});

describe('parseBackendStatus', () => {
  it('requires both executable paths the settings panel displays', () => {
    expect(() =>
      parseBackendStatus({
        ytdlpReady: true,
        ffmpegReady: true,
        ffprobeReady: true,
        ffmpegPath: 'C:/lib/ffmpeg.exe',
      }),
    ).toThrow('backendStatus.ytdlpPath: expected string, got undefined');
  });
});

/// The hand-written payloads above prove the parsers' own behaviour; this
/// proves them against Rust. Every variant serde emits (all options set, all
/// `None`, empty collections, each tagged enum arm) must parse unchanged.
describe('Rust-serialized fixture', () => {
  const cases: [string, unknown[], (value: unknown) => unknown][] = [
    ['manifest', fixture.manifest, parseManifest],
    ['backendStatus', fixture.backendStatus, parseBackendStatus],
    ['settings', fixture.settings, parseSettings],
    ['tasks', fixture.tasks, parseTaskSummaries],
    ['progress', fixture.progress, parseTaskProgress],
    ['downloadResult', fixture.downloadResult, parseDownloadResult],
    ['muxResult', fixture.muxResult, parseMuxResult],
    ['logs', fixture.logs, parseLogEntries],
  ];
  for (const [name, payloads, parse] of cases) {
    it.each(payloads.map((payload, index) => [index, payload]))(`parses ${name}[%i]`, (_, payload) => {
      expect(() => parse(payload)).not.toThrow();
    });
  }

  it('reads explicit nulls as absent, which is what serde sends for None', () => {
    const bare = parseManifest(fixture.manifest[1]);
    expect(fixture.manifest[1]).toHaveProperty('durationSecs', null);
    expect(bare.durationSecs).toBeUndefined();
    expect(bare.entries).toEqual([]);
  });

  it('keeps every delivery and plan arm the backend can emit', () => {
    const full = parseManifest(fixture.manifest[0]);
    expect(full.formats.map((format) => format.delivery.kind)).toEqual(['direct', 'hls', 'dash']);
    expect(full.downloadOptions.map((option) => option.plan.kind)).toEqual([
      'progressive',
      'merge',
      'manifest',
    ]);
  });
});
