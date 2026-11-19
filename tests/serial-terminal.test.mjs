import { test } from 'node:test';
import assert from 'node:assert/strict';
import ts from 'typescript';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../src/lib/serialTerminal.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.ESNext } }).outputText;
const { TerminalLog, encodeInput, stripAnsi } = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);

const bytes = (s) => Array.from(Buffer.from(s, 'utf8'));
const texts = (log) => log.view(false).map((l) => l.text);

test('CRLF, LF and a CR split across chunks each end exactly one line', () => {
  const log = new TerminalLog();
  log.append(bytes('boot ok\r'));
  log.append(bytes('\nnext\nthird'));
  assert.deepEqual(texts(log), ['boot ok', 'next', 'third']);
  assert.equal(log.lines.length, 2); // "third" is still in progress
});

test('lone CR redraws the line in place and backspace erases', () => {
  const log = new TerminalLog();
  log.append(bytes('10%\r50%\r100%\n'));
  log.append(bytes('ab\bc\n'));
  assert.deepEqual(texts(log), ['100%', 'ac']);
});

test('ANSI colour codes are stripped, even when split between chunks', () => {
  const log = new TerminalLog();
  log.append(bytes('\u001b[0;32mI (31) boot\u001b'));
  log.append(bytes('[0m done\n'));
  assert.deepEqual(texts(log), ['I (31) boot done']);
  assert.equal(stripAnsi('\u001b]0;title\u0007x\u001b[2K'), 'x');
});

test('multi-byte UTF-8 split across chunks decodes intact', () => {
  const log = new TerminalLog();
  const b = bytes('温度 25°C\n');
  log.append(b.slice(0, 2));
  log.append(b.slice(2));
  assert.deepEqual(texts(log), ['温度 25°C']);
});

test('log is bounded and hex view rows hold 16 bytes', () => {
  const log = new TerminalLog(3);
  log.append(bytes('1\n2\n3\n4\n5\n'));
  assert.deepEqual(texts(log), ['3', '4', '5']);
  const hex = new TerminalLog();
  hex.append(Array.from({ length: 18 }, (_, i) => i));
  assert.deepEqual(hex.view(true).map((l) => l.text), [
    '00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f', '10 11']);
});

test('input encoding: text + line endings, and hex forms', () => {
  assert.deepEqual(encodeInput('AT', 'crlf', false), [65, 84, 13, 10]);
  assert.deepEqual(encodeInput('é', 'none', false), [0xc3, 0xa9]);
  assert.deepEqual(encodeInput('0x48, 0x69', 'lf', true), [0x48, 0x69, 10]);
  assert.deepEqual(encodeInput('0d0A', 'none', true), [13, 10]);
  assert.throws(() => encodeInput('4', 'none', true));
  assert.throws(() => encodeInput('zz', 'none', true));
});
