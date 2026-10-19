const namedKeys: Record<string, number> = {
  Enter: 40, Escape: 41, Backspace: 42, Tab: 43, Space: 44,
  Minus: 45, Equal: 46, BracketLeft: 47, BracketRight: 48, Backslash: 49,
  Semicolon: 51, Quote: 52, Backquote: 53, Comma: 54, Period: 55, Slash: 56,
  CapsLock: 57, PrintScreen: 70, ScrollLock: 71, Pause: 72, Insert: 73,
  Home: 74, PageUp: 75, Delete: 76, End: 77, PageDown: 78,
  ArrowRight: 79, ArrowLeft: 80, ArrowDown: 81, ArrowUp: 82,
  NumLock: 83, NumpadDivide: 84, NumpadMultiply: 85, NumpadSubtract: 86,
  NumpadAdd: 87, NumpadEnter: 88, Numpad0: 98, NumpadDecimal: 99,
  ContextMenu: 101,
};

export const modifierCodes: Record<string, number> = {
  ControlLeft: 1, ShiftLeft: 2, AltLeft: 4, MetaLeft: 8,
  ControlRight: 16, ShiftRight: 32, AltRight: 64, MetaRight: 128,
};

export function keyUsage(code: string): number | undefined {
  if (/^Key[A-Z]$/.test(code)) return code.charCodeAt(3) - 65 + 4;
  if (/^Digit[0-9]$/.test(code)) return code === 'Digit0' ? 39 : Number(code[5]) + 29;
  if (/^F([1-9]|1[0-2])$/.test(code)) return Number(code.slice(1)) + 57;
  if (/^Numpad[1-9]$/.test(code)) return Number(code[6]) + 88;
  return namedKeys[code];
}

export function heldReport(codes: Iterable<string>): { modifiers: number; keys: number[] } {
  let modifiers = 0;
  const keys: number[] = [];
  for (const code of codes) {
    if (modifierCodes[code]) modifiers |= modifierCodes[code];
    else {
      const usage = keyUsage(code);
      if (usage !== undefined && !keys.includes(usage)) keys.push(usage);
    }
  }
  if (keys.length > 6) throw new Error('At most six simultaneous keys');
  return { modifiers, keys };
}

export class KeyboardQueue {
  private tail: Promise<unknown> = Promise.resolve();
  private generation = 0;

  constructor(private releaseKeys: () => Promise<void>) {}

  submit(operation: () => Promise<void>): Promise<void> {
    const generation = this.generation;
    const result = this.tail.catch(() => {}).then(async () => {
      if (generation === this.generation) await operation();
    });
    this.tail = result.catch(() => {});
    return result;
  }

  cancel(): Promise<void> {
    this.generation++;
    const release = this.releaseKeys();
    const result = Promise.allSettled([this.tail, release]).then(() => this.releaseKeys());
    this.tail = result.catch(() => {});
    return result;
  }
}