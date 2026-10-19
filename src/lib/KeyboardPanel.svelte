<script lang="ts">
  import { onDestroy } from 'svelte';
  import { commands } from './ipc';
  import { heldReport, keyUsage, KeyboardQueue } from './panelKeyboard';

  let { enabled, expanded = $bindable(false), onerror }: {
    enabled: boolean; expanded?: boolean; onerror: (message: string) => void;
  } = $props();
  let text = $state('');
  let busy = $state(false);
  let live = $state(false);
  let status = $state('');
  let selectedModifiers = $state<number[]>([]);
  let liveButton: HTMLButtonElement | undefined = $state();
  const held = new Set<string>();
  let touched = false;
  const queue = new KeyboardQueue(commands.releasePanelKeyboard);
  const modifiers = [{ name: 'Ctrl', mask: 1 }, { name: 'Shift', mask: 2 },
    { name: 'Alt', mask: 4 }, { name: 'Win / Super', mask: 8 }];
  const keys = [
    ['Esc', 'Escape'], ['Tab', 'Tab'], ['Enter', 'Enter'], ['Backspace', 'Backspace'],
    ['Delete', 'Delete'], ['Space', 'Space'], ['Home', 'Home'], ['End', 'End'],
    ['PgUp', 'PageUp'], ['PgDn', 'PageDown'], ['Insert', 'Insert'], ['PrtSc', 'PrintScreen'],
    ['Left', 'ArrowLeft'], ['Up', 'ArrowUp'], ['Down', 'ArrowDown'], ['Right', 'ArrowRight'],
    ...Array.from({ length: 12 }, (_, index) => [`F${index + 1}`, `F${index + 1}`]),
  ];
  const shortcuts = [
    { name: 'Ctrl+C', modifiers: 1, usage: 6 },
    { name: 'Ctrl+V', modifiers: 1, usage: 25 },
    { name: 'Ctrl+Z', modifiers: 1, usage: 29 },
    { name: 'Ctrl+A', modifiers: 1, usage: 4 },
    { name: 'Ctrl+Shift+V', modifiers: 3, usage: 25 },
    { name: 'Alt+Tab', modifiers: 4, usage: 43 },
  ];

  function failure(error: unknown) {
    if (!String(error).includes('Keyboard input cancelled')) onerror(String(error));
  }

  function stop() {
    held.clear();
    live = false;
    selectedModifiers = [];
    if (!touched) return;
    touched = false;
    void queue.cancel().catch(failure);
  }

  function blurLive() {
    if (held.size || live) stop();
  }

  async function tap(usage: number, modifiers = selectedModifiers.reduce((mask, value) => mask | value, 0)) {
    if (!enabled || busy) return;
    selectedModifiers = [];
    touched = true;
    busy = true;
    try {
      await queue.submit(() => commands.panelKeyboard(modifiers, [usage], true));
      status = 'Key sent';
    } catch (error) { failure(error); }
    finally { busy = false; }
  }

  async function sendText(enter: boolean) {
    if (!enabled || busy || (!text && !enter)) return;
    const value = text;
    touched = true;
    busy = true;
    status = 'Sending...';
    try {
      await queue.submit(() => commands.panelKeyboard(0, [], false, value, enter));
      status = `Sent ${value.length} characters${enter ? ' + Enter' : ''}`;
    } catch (error) { status = ''; failure(error); }
    finally { busy = false; }
  }

  function liveKey(event: KeyboardEvent, pressed: boolean) {
    event.preventDefault();
    event.stopPropagation();
    if (!enabled || busy || event.isComposing || event.repeat) return;
    if (pressed) held.add(event.code);
    else held.delete(event.code);
    try {
      const report = heldReport(held);
      if ((report.modifiers & 17) && (report.modifiers & 4)) {
        stop();
        liveButton?.blur();
        return;
      }
      touched = true;
      void queue.submit(() => commands.panelKeyboard(report.modifiers, report.keys)).catch((error) => {
        stop();
        failure(error);
      });
    } catch (error) {
      held.delete(event.code);
      failure(error);
    }
  }

  $effect(() => {
    if (!enabled || !expanded) stop();
  });
  onDestroy(stop);
</script>

<svelte:window onblur={blurLive} />

<details class="keyboard-panel" bind:open={expanded}>
  <summary>Keyboard</summary>
  <div class="keyboard-body">
    <fieldset disabled={!enabled || busy}>
      <div class="text-send">
        <textarea aria-label="Text to send" title="US keyboard layout, up to 4096 bytes. Enter is sent only when explicitly requested or included in the text."
          bind:value={text} rows="2" placeholder="Text to send" maxlength="4096"></textarea>
        <div class="send-actions">
          <button onclick={() => sendText(false)} disabled={!text}>Send</button>
          <button onclick={() => sendText(true)}>Send + Enter</button>
        </div>
      </div>
      <div class="modifiers">
        {#each modifiers as modifier}
          <label><input type="checkbox" bind:group={selectedModifiers} value={modifier.mask} />{modifier.name}</label>
        {/each}
      </div>
      <div class="key-grid">
        {#each keys as [label, code]}
          <button title={code} onclick={() => tap(keyUsage(code)!)}>{label}</button>
        {/each}
      </div>
      <div class="shortcuts">
        {#each shortcuts as shortcut}
          <button onclick={() => tap(shortcut.usage, shortcut.modifiers)}>{shortcut.name}</button>
        {/each}
      </div>
      <button class="live-keyboard" class:active={live} bind:this={liveButton}
        title="Focus here to forward physical keys. Ctrl+Left Alt or clicking elsewhere releases keys. The target uses a US keyboard layout."
        onfocus={() => (live = true)} onblur={blurLive}
        onkeydown={(event) => liveKey(event, true)} onkeyup={(event) => liveKey(event, false)}>
        {live ? 'Forwarding keys' : 'Live keyboard'}
      </button>
    </fieldset>
    <span class="keyboard-status" role="status">{!enabled ? 'Enable Keyboard and unlock capture' : busy ? 'Sending...' : status}</span>
  </div>
</details>

<style>
  .keyboard-panel { flex: none; border-top: 1px solid var(--border); background: var(--panel); max-height: 46vh; overflow: auto; }
  summary { padding: 10px 14px; cursor: pointer; font-weight: 600; }
  .keyboard-body { padding: 0 14px 10px; }
  fieldset { border: 0; padding: 0; margin: 0; min-width: 0; }
  fieldset:disabled { opacity: 0.55; }
  .text-send { display: flex; gap: 8px; }
  textarea { flex: 1; min-width: 0; resize: vertical; border: 1px solid var(--border); border-radius: 4px; background: var(--bg); color: var(--text); padding: 8px; font: inherit; }
  .send-actions { display: flex; flex-direction: column; gap: 4px; }
  .modifiers { display: flex; flex-wrap: wrap; gap: 14px; margin: 8px 0; }
  .modifiers label { display: inline-flex; align-items: center; gap: 4px; }
  .key-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(78px, 1fr)); gap: 4px; }
  .key-grid button { min-height: 32px; padding: 5px; }
  .shortcuts { display: flex; flex-wrap: wrap; gap: 4px; margin: 8px 0; }
  .live-keyboard { width: 100%; min-height: 34px; }
  .live-keyboard.active { border-color: var(--green); }
  .keyboard-status { display: block; font-size: 12px; color: var(--muted); min-height: 18px; margin-top: 4px; }
</style>