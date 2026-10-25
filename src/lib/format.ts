/** Human-readable byte and rate formatting for the download UI. */

import type { SizeHint } from './ipc/client';

const UNITS = ['B', 'KB', 'MB', 'GB'] as const;

/**
 * Renders a [`SizeHint`] for display.
 *
 * Whole bytes print without a decimal because "512.0 B" reads as false
 * precision; every larger unit keeps one so a 1.4 GB file is distinguishable
 * from a 1.9 GB one. `SizeHint::Unknown` has no byte count at all, which is a
 * normal answer from an extractor rather than an error.
 */
export function formatSize(size: SizeHint): string {
  if (size.kind === 'unknown') return '未知';

  let value = size.bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit === 0 ? 0 : 1)} ${UNITS[unit]}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  return `${formatSize({ kind: 'exact', bytes: bytesPerSecond })}/s`;
}
