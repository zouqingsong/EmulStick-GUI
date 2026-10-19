import { test } from 'node:test';
import assert from 'node:assert/strict';
import ts from 'typescript';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/lib/panelKeyboard.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.ESNext } }).outputText;
const { keyUsage, heldReport, KeyboardQueue } = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);

test('physical keys map to US HID usages', () => {
  for (const [code, usage] of [['KeyA', 4], ['KeyZ', 29], ['Digit1', 30], ['Digit0', 39],
    ['F1', 58], ['F12', 69], ['Numpad9', 97], ['NumpadEnter', 88], ['ArrowLeft', 80], ['Slash', 56]]) {
    assert.equal(keyUsage(code), usage);
  }
  assert.equal(keyUsage('Unidentified'), undefined);
});

test('held reports combine modifiers and release removed keys', () => {
  const held = new Set(['ControlLeft', 'ShiftRight', 'KeyC']);
  assert.deepEqual(heldReport(held), { modifiers: 33, keys: [6] });
  held.delete('KeyC');
  assert.deepEqual(heldReport(held), { modifiers: 33, keys: [] });
  assert.deepEqual(heldReport([]), { modifiers: 0, keys: [] });
  assert.throws(() => heldReport(['KeyA', 'KeyB', 'KeyC', 'KeyD', 'KeyE', 'KeyF', 'KeyG']));
});

test('keyboard operations are serialized', async () => {
  const order = [];
  let finish;
  const queue = new KeyboardQueue(async () => {});
  const first = queue.submit(async () => {
    order.push('down');
    await new Promise(resolve => { finish = resolve; });
  });
  const second = queue.submit(async () => { order.push('up'); });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(order, ['down']);
  finish();
  await Promise.all([first, second]);
  assert.deepEqual(order, ['down', 'up']);
});

test('cancellation discards queued keys and waits for release before new input', async () => {
  const order = [];
  let finish;
  const queue = new KeyboardQueue(async () => { order.push('release'); });
  const first = queue.submit(async () => {
    order.push('in-flight');
    await new Promise(resolve => { finish = resolve; });
  });
  const stale = queue.submit(async () => { order.push('stale'); });
  await new Promise(resolve => setImmediate(resolve));
  const cancel = queue.cancel();
  const next = queue.submit(async () => { order.push('next'); });
  finish();
  await Promise.all([first, stale, cancel, next]);
  assert.deepEqual(order, ['in-flight', 'release', 'release', 'next']);
});

test('cancellation ends with release even when IPC starts late', async () => {
  const order = [];
  let finish;
  const queue = new KeyboardQueue(async () => { order.push('release'); });
  const first = queue.submit(async () => {
    await new Promise(resolve => { finish = resolve; });
    order.push('late key down');
  });
  await new Promise(resolve => setImmediate(resolve));
  const cancel = queue.cancel();
  finish();
  await Promise.all([first, cancel]);
  assert.deepEqual(order, ['release', 'late key down', 'release']);
});

test('a failed write does not wedge subsequent operations', async () => {
  const queue = new KeyboardQueue(async () => {});
  await assert.rejects(queue.submit(async () => { throw new Error('failed'); }));
  let sent = false;
  await queue.submit(async () => { sent = true; });
  assert.equal(sent, true);
});