// Serial terminal text model: turns the raw UART byte stream into display
// lines, and operator input into bytes. Pure (no Tauri/Svelte) so it can be
// unit-tested in node — see tests/serial-terminal.test.mjs.

export type LineEnding = 'none' | 'lf' | 'cr' | 'crlf';
export type Direction = 'rx' | 'tx';

export interface LogLine {
  /** Epoch ms when the line started. */
  time: number;
  text: string;
  dir: Direction;
}

const ENDINGS: Record<LineEnding, number[]> = { none: [], lf: [10], cr: [13], crlf: [13, 10] };
const ESC = '\u001b';

/** Bounded line log fed by streaming UART chunks. */
export class TerminalLog {
  lines: LogLine[] = [];
  /** RX line still being received (no newline yet). */
  partial: LogLine | null = null;
  /** Raw RX bytes as hex-dump lines, for the hex view. */
  hexLines: LogLine[] = [];

  private decoder = new TextDecoder('utf-8');
  private pendingCr = false;
  /** Incomplete ANSI escape sequence carried over from the previous chunk. */
  private pendingEsc = '';

  constructor(private maxLines = 5000) {}

  append(bytes: ArrayLike<number>, now = Date.now()) {
    const data = Uint8Array.from(bytes as ArrayLike<number>);
    if (!data.length) return;
    for (let i = 0; i < data.length; i += 16) {
      const row = Array.from(data.subarray(i, i + 16), (b) => b.toString(16).padStart(2, '0'));
      this.hexLines.push({ time: now, text: row.join(' '), dir: 'rx' });
    }
    trim(this.hexLines, this.maxLines);

    let text = this.pendingEsc + this.decoder.decode(data, { stream: true });
    this.pendingEsc = '';
    // Hold back an escape sequence split across chunks until it completes.
    const lastEsc = text.lastIndexOf(ESC);
    if (lastEsc >= 0 && !/^\u001b(\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001b]*(\u0007|\u001b\\)|[@-Z\\-_])/.test(text.slice(lastEsc))) {
      if (text.length - lastEsc < 64) {
        this.pendingEsc = text.slice(lastEsc);
        text = text.slice(0, lastEsc);
      }
    }
    text = stripAnsi(text);

    for (const ch of text) {
      if (this.pendingCr) {
        this.pendingCr = false;
        if (ch === '\n') {
          this.newline(now);
          continue;
        }
        // Lone CR: return to line start (progress bars redraw in place).
        if (this.partial) this.partial.text = '';
      }
      if (ch === '\r') this.pendingCr = true;
      else if (ch === '\n') this.newline(now);
      else if (ch === '\b') {
        if (this.partial) this.partial.text = this.partial.text.slice(0, -1);
      } else if (ch === '\t' || ch >= ' ') {
        if (!this.partial) this.partial = { time: now, text: '', dir: 'rx' };
        this.partial.text += ch;
      }
      // Other control characters (bell, NUL, …) are dropped.
    }
  }

  /** Record locally-sent data (echo). */
  addSent(text: string, now = Date.now()) {
    this.lines.push({ time: now, text, dir: 'tx' });
    trim(this.lines, this.maxLines);
  }

  clear() {
    this.lines = [];
    this.hexLines = [];
    this.partial = null;
    this.pendingCr = false;
    this.pendingEsc = '';
  }

  /** Lines for display, including the in-progress one. */
  view(hex: boolean): LogLine[] {
    if (hex) return this.hexLines;
    return this.partial ? [...this.lines, this.partial] : this.lines;
  }

  private newline(now: number) {
    this.lines.push(this.partial ?? { time: now, text: '', dir: 'rx' });
    this.partial = null;
    trim(this.lines, this.maxLines);
  }
}

function trim(lines: LogLine[], max: number) {
  if (lines.length > max) lines.splice(0, lines.length - max);
}

/** Remove ANSI CSI/OSC/two-byte escape sequences (colours, cursor moves). */
export function stripAnsi(text: string): string {
  return text.replace(/\u001b(\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001b]*(\u0007|\u001b\\)|[@-Z\\-_])/g, '');
}

/** `HH:MM:SS.mmm` local time. */
export function formatTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number, w = 2) => String(n).padStart(w, '0');
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

export function formatLine(line: LogLine, timestamps: boolean): string {
  const prefix = (timestamps ? `[${formatTime(line.time)}] ` : '') + (line.dir === 'tx' ? '→ ' : '');
  return prefix + line.text;
}

/**
 * Bytes to send for one input line. In hex mode `text` is hex byte pairs
 * (`48 65 6c`, `0x48,0x65`, or `48656c`); the line ending is appended either way.
 */
export function encodeInput(text: string, ending: LineEnding, hex: boolean): number[] {
  let body: number[];
  if (hex) {
    const digits = text.replace(/0x/gi, '').replace(/[\s,]/g, '');
    if (!/^[0-9a-f]*$/i.test(digits) || digits.length % 2) {
      throw new Error('Hex input must be whole bytes, e.g. "0D 0A" or "0x48,0x69"');
    }
    body = (digits.match(/../g) ?? []).map((h) => parseInt(h, 16));
  } else {
    body = Array.from(new TextEncoder().encode(text));
  }
  return [...body, ...ENDINGS[ending]];
}
