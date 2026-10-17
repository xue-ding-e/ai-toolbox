import test from 'node:test';
import assert from 'node:assert/strict';

import { findRustRegexIncompatibility } from '../../../../features/settings/utils/rustRegexCompat.ts';

test('patterns both engines accept pass the scan', () => {
  for (const pattern of [
    'node_modules',
    '\\.git',
    '(?i)readme',
    '(?P<name>a)b',
    '(?<name>a)b',
    '(?:ab)+',
    'a++',
    '.*+',
    'a{1,2}+',
    '\\d+',
    '\\p{L}+',
    '\\pL',
    '\\PL',
    '\\x41',
    '\\x{263A}',
    '\\u{263A}',
    '\\U0001F600',
    '[\\w\\-]+',
    '\\bword\\b',
    '\\b{start}',
    '(?i:abc)',
    '(?R)',
    '(?x) a b',
    '\\\\d', // escaped backslash then literal d — not a class escape
    'a\\\\1', // literal backslash before 1 — not a backreference
  ]) {
    assert.equal(findRustRegexIncompatibility(pattern), null, pattern);
  }
});

test('look-around is rejected', () => {
  for (const pattern of ['(?=a)', '(?!a)', '(?<=a)', '(?<!a)', 'x(?=y)z']) {
    assert.equal(findRustRegexIncompatibility(pattern), 'lookAround', pattern);
  }
});

test('backreferences are rejected', () => {
  for (const pattern of ['(a)\\1', '\\0', '(a)(b)\\12']) {
    assert.equal(findRustRegexIncompatibility(pattern), 'backreference', pattern);
  }
});

test('named backreferences are rejected', () => {
  for (const pattern of ['(?<n>a)\\k<n>', "(?'n'a)\\k'n'"]) {
    assert.equal(findRustRegexIncompatibility(pattern), 'namedBackreference', pattern);
  }
});

test('subroutine calls are rejected', () => {
  for (const pattern of ['(?P=n)', '(?P>n)', '(?&n)', '\\g<1>', '\\g{1}', '\\g1']) {
    assert.equal(findRustRegexIncompatibility(pattern), 'subroutineCall', pattern);
  }
});

test('literal quoting is rejected', () => {
  for (const pattern of ['\\Qa.b\\E', 'x\\Qy\\E']) {
    assert.equal(findRustRegexIncompatibility(pattern), 'literalQuote', pattern);
  }
});

test('escapes JavaScript reads as literals but Rust rejects are reported', () => {
  for (const pattern of ['\\h', '\\H', '\\R', '\\X', '\\G', '\\Z', '\\N', '\\V', '\\e', '\\q', '\\cA']) {
    assert.equal(findRustRegexIncompatibility(pattern), 'unsupportedEscape', pattern);
  }
});

test('parameterized escapes without their parameter are reported', () => {
  // `\p` alone is rejected by Rust; `\p{L}` / `\pL` are fine (covered above).
  for (const pattern of ['\\p', '\\x', '\\u', '\\U', '\\xzz']) {
    assert.equal(findRustRegexIncompatibility(pattern), 'unsupportedEscape', pattern);
  }
});

test('structural typos are left to the backend compiler', () => {
  // The scanner is a Rust-syntax pre-check, not a full parser: patterns that
  // are invalid in every engine pass through here and fail at save time with
  // the backend error naming the entry and the pattern.
  for (const pattern of ['*a', '(?', '[a-\\d]', 'a**']) {
    assert.equal(findRustRegexIncompatibility(pattern), null, pattern);
  }
});
