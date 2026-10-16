import { test } from 'node:test';
import assert from 'node:assert/strict';
import ts from 'typescript';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/lib/absolutePointer.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.ESNext } }).outputText;
const { videoPoint, PointerQueue } = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);

test('video mapping excludes horizontal letterboxing and includes endpoints', () => {
  const rect = { left: 10, top: 20, width: 1000, height: 1000 };
  assert.equal(videoPoint(510, 100, rect, 1920, 1080), null);
  assert.deepEqual(videoPoint(510, 520, rect, 1920, 1080), { x: 1920, y: 1080 });
  assert.deepEqual(videoPoint(10, 238.75, rect, 1920, 1080), { x: 0, y: 0 });
  assert.deepEqual(videoPoint(1010, 801.25, rect, 1920, 1080), { x: 3840, y: 2160 });
});

test('pillarboxing, missing metadata, and resized video', () => {
  const rect = { left: 0, top: 0, width: 1920, height: 1080 };
  assert.equal(videoPoint(100, 540, rect, 4, 3), null);
  assert.deepEqual(videoPoint(960, 540, rect, 4, 3), { x: 1920, y: 1080 });
  assert.equal(videoPoint(960, 540, rect, 0, 0), null);
  assert.deepEqual(videoPoint(480, 270, { ...rect, width: 960, height: 540 }, 1920, 1080), { x: 1920, y: 1080 });
});

test('slow writes coalesce positions without losing button transitions', async () => {
  const sent = [];
  let finish;
  const queue = new PointerQueue(async (report) => {
    sent.push(report);
    if (sent.length === 1) await new Promise(resolve => { finish = resolve; });
  }, error => { throw error; });
  const report = { x: 1, y: 2, flags: 1, wheel: 0 };
  queue.enqueue(report, true);
  queue.enqueue({ ...report, x: 2 }, true);
  queue.enqueue({ ...report, x: 3 }, true);
  queue.enqueue({ ...report, flags: 3 });
  queue.enqueue({ ...report, flags: 1 });
  finish();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(sent.map(value => [value.x, value.flags]), [[1, 1], [3, 1], [1, 3], [1, 1]]);
});

test('release discards stale motion and follows any in-flight write', async () => {
  const sent = [];
  let finish;
  const queue = new PointerQueue(async (report) => {
    sent.push(report);
    if (sent.length === 1) await new Promise(resolve => { finish = resolve; });
  }, error => { throw error; });
  const report = { x: 1, y: 2, flags: 3, wheel: 0 };
  queue.enqueue(report);
  queue.enqueue({ ...report, x: 200 }, true);
  queue.release(report);
  finish();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(sent.map(value => [value.x, value.flags]), [[1, 3], [1, 0]]);
});

test('write failure still drains the emergency release', async () => {
  const sent = [];
  const report = { x: 1, y: 2, flags: 3, wheel: 0 };
  const queue = new PointerQueue(async (value) => {
    sent.push(value);
    if (sent.length === 1) throw new Error('write failed');
  }, () => queue.release(report));
  queue.enqueue(report);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(sent.map(value => value.flags), [3, 0]);
});