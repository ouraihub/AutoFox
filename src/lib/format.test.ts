import { describe, expect, it } from 'vitest';

import { formatSize, formatSpeed } from './format';

describe('formatSize', () => {
  it('prints whole bytes without false precision', () => {
    expect(formatSize({ kind: 'exact', bytes: 512 })).toBe('512 B');
  });

  it('keeps one decimal from KB upward', () => {
    expect(formatSize({ kind: 'exact', bytes: 1024 })).toBe('1.0 KB');
    expect(formatSize({ kind: 'exact', bytes: 1_572_864 })).toBe('1.5 MB');
  });

  /// The unit walk stops at GB, so a larger file must not fall off the end of
  /// the table and render `undefined`.
  it('saturates at GB rather than running past the unit table', () => {
    expect(formatSize({ kind: 'exact', bytes: 5 * 1024 ** 4 })).toBe('5120.0 GB');
  });

  it('reports an unknown size as text rather than a number', () => {
    expect(formatSize({ kind: 'unknown' })).toBe('未知');
  });

  it('formats zero as a byte count', () => {
    expect(formatSize({ kind: 'exact', bytes: 0 })).toBe('0 B');
  });
});

describe('formatSpeed', () => {
  it('appends a per-second suffix to the byte size', () => {
    expect(formatSpeed(1_048_576)).toBe('1.0 MB/s');
  });
});
