/**
 * Frontend pre-check for backup exclude patterns.
 *
 * Exclude patterns are compiled by the backend with the Rust `regex` crate,
 * whose syntax set differs from JavaScript's `RegExp`:
 *
 * - Rust rejects look-around (`(?=`, `(?!`, `(?<=`, `(?<!)`), backreferences
 *   (`\1`, `\k<name>`), subroutine calls (`(?P=`, `(?&`, `\g<...>`) and
 *   `\Q...\E` literal quoting — all of which `new RegExp` happily accepts, so
 *   a JS-side `new RegExp` check used to wave these through and the user only
 *   found out when the whole backup-settings save was rejected.
 * - Conversely Rust accepts possessive quantifiers (`a++`), `(?i)` inline
 *   flags and `(?P<name>...)` groups that `new RegExp` throws on, so the JS
 *   check also blocked valid patterns.
 *
 * This scanner therefore mirrors the Rust syntax set instead of delegating to
 * `new RegExp`. It is deliberately a *pre*-check: anything it passes can still
 * be rejected by the backend compiler (e.g. `*a`, `[a-\d]`); the backend error
 * names the entry and the pattern, so structural typos surface at save time.
 *
 * The accepted escape sets were probed against regex 1.12 (see the companion
 * test for the full case list).
 */

/** Bare `\x` escapes the Rust regex crate accepts. */
const RUST_BARE_ESCAPES = new Set('aAbBdDfnrsStvwWz'.split(''));

/**
 * Escapes that take a parameter: `p`/`P` (`\p{L}`, `\pL`), `x` (`\x41`,
 * `\x{...}`), `u`/`U` (`z`, `\u{...}`). Bare `\p` without a parameter is
 * rejected by Rust but the parameterized forms are fine.
 */
const RUST_PARAMETERIZED_ESCAPES = new Set(['p', 'P', 'x', 'u', 'U']);

const HEX_DIGITS = new Set('0123456789abcdefABCDEF'.split(''));

export type RustRegexIssue =
  /** `(?=a)`, `(?!a)`, `(?<=a)`, `(?<!a)` */
  | 'lookAround'
  /** `\1`..`\9`, `\0` */
  | 'backreference'
  /** `\k<name>`, `\k'name'` */
  | 'namedBackreference'
  /** `(?P=name)`, `(?P>name)`, `(?&name)`, `\g<1>`, `\g{1}` */
  | 'subroutineCall'
  /** `\Q...\E` literal quoting */
  | 'literalQuote'
  /** Any `\x` escape Rust does not know but JavaScript reads as a literal */
  | 'unsupportedEscape';

/**
 * Scan `pattern` for constructs the Rust regex crate refuses. Returns the
 * first issue found, or `null` when nothing known-bad is present.
 */
export function findRustRegexIncompatibility(pattern: string): RustRegexIssue | null {
  for (let index = 0; index < pattern.length; index += 1) {
    const char = pattern[index];

    if (char === '\\') {
      const next = pattern[index + 1];
      if (next === undefined) {
        // A trailing lone backslash is invalid everywhere; the backend
        // compiler reports it with the entry name attached.
        return null;
      }
      if (next === '\\') {
        // Escaped backslash: a literal `\`, not an escape sequence.
        index += 1;
        continue;
      }
      if (/[0-9]/.test(next)) {
        return 'backreference';
      }
      if (next === 'k') {
        return 'namedBackreference';
      }
      if (next === 'g') {
        return 'subroutineCall';
      }
      if (next === 'Q' || next === 'E') {
        return 'literalQuote';
      }
      if (/[a-zA-Z]/.test(next)) {
        if (RUST_BARE_ESCAPES.has(next)) {
          index += 1;
          continue;
        }
        if (RUST_PARAMETERIZED_ESCAPES.has(next)) {
          const parameter = pattern[index + 2];
          const parameterized =
            parameter !== undefined &&
            (parameter === '{' ||
              HEX_DIGITS.has(parameter) ||
              // `\pL` / `\PL`: one-letter Unicode class shorthand.
              ((next === 'p' || next === 'P') && /[a-zA-Z]/.test(parameter)));
          if (parameterized) {
            index += 1;
            continue;
          }
          return 'unsupportedEscape';
        }
        return 'unsupportedEscape';
      }
      // `\.` / `\(` / `\{` … — punctuation escapes are the same in both.
      index += 1;
      continue;
    }

    if (char === '(' && pattern[index + 1] === '?') {
      const after = pattern[index + 2];
      if (after === '=' || after === '!') {
        return 'lookAround';
      }
      if (after === '<') {
        const lookBehind = pattern[index + 3];
        if (lookBehind === '=' || lookBehind === '!') {
          return 'lookAround';
        }
        // `(?<name>...)` is a valid named group in both engines.
        continue;
      }
      if (after === 'P') {
        const kind = pattern[index + 3];
        if (kind === '=' || kind === '>') {
          return 'subroutineCall';
        }
        // `(?P<name>...)` is a valid named group in Rust.
        continue;
      }
      if (after === '&') {
        return 'subroutineCall';
      }
      // `(?i)`, `(?:`, `(?R)`, `(?x: …)` — valid Rust group syntax.
    }
  }
  return null;
}
