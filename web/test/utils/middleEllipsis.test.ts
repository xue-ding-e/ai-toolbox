import assert from 'node:assert/strict';
import test from 'node:test';
import {
  KEY_HEAD_CHARS,
  KEY_TAIL_CHARS,
  middleEllipsis,
} from '../../utils/middleEllipsis.ts';

const buildKey = (length: number) => 'k'.repeat(length);

test('a value that already fits comes back untouched', () => {
  const fits = buildKey(KEY_HEAD_CHARS + KEY_TAIL_CHARS + 1);
  assert.equal(middleEllipsis(fits), fits);
});

test('one character over the budget is elided, not left to wrap', () => {
  const overflow = buildKey(KEY_HEAD_CHARS + KEY_TAIL_CHARS + 2);
  assert.equal(middleEllipsis(overflow), `${buildKey(KEY_HEAD_CHARS)}…${buildKey(KEY_TAIL_CHARS)}`);
});

test('both ends survive, because both identify the key', () => {
  const key = 'sk-ant-fixture-0123456789abcdefghijklmnopqrstuvwxyz-TAIL';
  const shortened = middleEllipsis(key);
  assert.ok(shortened.startsWith('sk-ant-fixture-0123456789'));
  assert.ok(shortened.endsWith('-TAIL'));
  assert.ok(shortened.includes('…'));
  assert.ok(shortened.length < key.length);
});

test('the shortened line is shorter than the budget it was given', () => {
  assert.equal(middleEllipsis(buildKey(400)).length, KEY_HEAD_CHARS + KEY_TAIL_CHARS + 1);
});

test('an empty key stays empty rather than becoming an ellipsis', () => {
  assert.equal(middleEllipsis(''), '');
});