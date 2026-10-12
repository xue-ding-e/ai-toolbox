/**
 * Shortening a long fixed-format string for a one-line cell.
 *
 * Its own module rather than a helper inside the detail panel that uses it,
 * because that is the shape this repo keeps pure text logic in: the unit test
 * imports this file directly and never drags React or antd into a node test
 * (`CodexMemoriesPanel/memoriesSource.ts` is the precedent).
 */

/**
 * How much of a key is kept at each end when it does not fit on one line.
 *
 * A budget in characters rather than a measurement: the detail panel's value
 * column is a fixed 294px of monospaced text, which holds about 36 characters,
 * and measuring per render would cost a layout pass for a string whose shape
 * never changes. The full value stays reachable two ways — the `title`
 * attribute on hover, and the copy button beside it.
 */
export const KEY_HEAD_CHARS = 25;
export const KEY_TAIL_CHARS = 10;

/**
 * A key on one line, with the elision in the **middle**.
 *
 * Both ends of a key identify it: the prefix says who issued it, the tail is
 * what the provider's console shows. An end-elided `text-overflow` would hide
 * exactly the half a reader is looking for, so a long key is shortened here
 * instead of being left to the browser.
 *
 * A value that already fits comes back untouched — including one that is
 * exactly one character over, where an elision would replace as much as it
 * saved.
 */
export function middleEllipsis(
  value: string,
  head: number = KEY_HEAD_CHARS,
  tail: number = KEY_TAIL_CHARS,
): string {
  if (value.length <= head + tail + 1) {
    return value;
  }
  return `${value.slice(0, head)}…${value.slice(-tail)}`;
}