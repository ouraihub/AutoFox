/**
 * Runtime validation for values crossing the Tauri IPC boundary.
 *
 * `invoke<T>()` is an unchecked assertion: the Rust side can rename a field and
 * TypeScript keeps claiming the old shape, so the failure surfaces as
 * `undefined` deep inside a component instead of at the boundary. These guards
 * turn that into one actionable error naming the offending path.
 *
 * Hand-written rather than a schema library: the contract is ~10 shapes, and a
 * dependency that ships its own parser would outweigh the code it replaces.
 */

/** Raised when a backend payload does not match the declared contract. */
export class ContractError extends Error {
  constructor(
    readonly path: string,
    readonly expected: string,
    readonly actual: unknown,
  ) {
    super(`${path}: expected ${expected}, got ${describe(actual)}`);
    this.name = 'ContractError';
  }
}

function describe(value: unknown): string {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  return typeof value;
}

/** Asserts `value` is a plain object and returns it for field access. */
export function object(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new ContractError(path, 'object', value);
  }
  return value as Record<string, unknown>;
}

export function str(value: unknown, path: string): string {
  if (typeof value !== 'string') throw new ContractError(path, 'string', value);
  return value;
}

/**
 * Rejects `NaN` and `Infinity` as well as non-numbers: both survive JSON and
 * both render as broken progress bars rather than as an error.
 */
export function num(value: unknown, path: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new ContractError(path, 'finite number', value);
  }
  return value;
}

export function bool(value: unknown, path: string): boolean {
  if (typeof value !== 'boolean') throw new ContractError(path, 'boolean', value);
  return value;
}

/**
 * Reads an optional field.
 *
 * `null` is treated as absent: `serde` skips `None` fields, but a hand-rolled
 * payload or a future `serialize_with` can emit an explicit null, and both mean
 * "no value" to every caller here.
 */
export function optional<T>(
  value: unknown,
  path: string,
  parse: (value: unknown, path: string) => T,
): T | undefined {
  if (value === undefined || value === null) return undefined;
  return parse(value, path);
}

export function array<T>(
  value: unknown,
  path: string,
  parse: (value: unknown, path: string) => T,
): T[] {
  if (!Array.isArray(value)) throw new ContractError(path, 'array', value);
  return value.map((item, index) => parse(item, `${path}[${index}]`));
}

/** Asserts `value` is one of `allowed`, which keeps string unions honest. */
export function literal<T extends string>(
  value: unknown,
  path: string,
  allowed: readonly T[],
): T {
  const text = str(value, path);
  if (!(allowed as readonly string[]).includes(text)) {
    throw new ContractError(path, `one of ${allowed.join(' | ')}`, text);
  }
  return text as T;
}

/**
 * Validates a `Record<string, string>`, used for the HTTP headers a format
 * carries. Header values reach an ffmpeg command line, so a non-string here is
 * worth rejecting at the boundary rather than stringifying silently.
 */
export function stringRecord(value: unknown, path: string): Record<string, string> {
  const source = object(value, path);
  const result: Record<string, string> = {};
  for (const [key, item] of Object.entries(source)) {
    result[key] = str(item, `${path}.${key}`);
  }
  return result;
}
