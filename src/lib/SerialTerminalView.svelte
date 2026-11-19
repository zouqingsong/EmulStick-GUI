<script lang="ts">
  import { onMount } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import '@xterm/xterm/css/xterm.css';
  import { commands } from './ipc';
  import { onRawData, rawHistory } from './serial.svelte';

  let { enabled, onerror, handle = $bindable() }: {
    enabled: boolean;
    onerror: (message: string) => void;
    handle?: { clear: () => void; copy: () => string; focus: () => void };
  } = $props();

  let host: HTMLDivElement;
  let term: Terminal | undefined;
  const encoder = new TextEncoder();

  // Tauri commands may run concurrently, so serialize writes and coalesce
  // keystrokes typed while one is in flight to keep bytes in order.
  let pending: number[] = [];
  let writing = false;
  async function flush() {
    if (writing) return;
    writing = true;
    try {
      while (pending.length) {
        const chunk = pending;
        pending = [];
        await commands.serialWrite(chunk);
      }
    } catch (error) {
      pending = [];
      onerror(String(error).replace(/^Error: /, ''));
    } finally {
      writing = false;
    }
  }

  function send(data: string) {
    if (!enabled) return;
    pending.push(...encoder.encode(data));
    flush();
  }

  async function paste() {
    try {
      const text = await navigator.clipboard.readText();
      if (text) term?.paste(text);
    } catch {
      /* clipboard unavailable; Ctrl+V falls back to the native paste event */
    }
  }

  onMount(() => {
    term = new Terminal({
      convertEol: false,
      cursorBlink: true,
      scrollback: 10000,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, monospace',
      fontSize: 13,
      theme: { background: '#0a0c10', foreground: '#d4d4d4', cursor: '#d4d4d4', selectionBackground: '#264f78' },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    for (const chunk of rawHistory()) term.write(chunk);

    // Copy/paste the way serial terminals do: Ctrl+C copies only when text is
    // selected (otherwise it's sent as ^C); Ctrl+Shift+C/V always copy/paste.
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown') return true;
      const ctrl = event.ctrlKey || event.metaKey;
      const key = event.key.toLowerCase();
      if (ctrl && key === 'c' && (event.shiftKey || term!.hasSelection())) {
        navigator.clipboard.writeText(term!.getSelection()).catch(() => {});
        term!.clearSelection();
        return false;
      }
      if (ctrl && key === 'v') {
        event.preventDefault();
        paste();
        return false;
      }
      return true;
    });
    const dataSub = term.onData(send);
    const unlisten = onRawData((bytes) => term?.write(bytes));

    const refit = () => {
      if (host.clientWidth && host.clientHeight) {
        try { fit.fit(); } catch { /* not laid out yet */ }
      }
    };
    const ro = new ResizeObserver(refit);
    ro.observe(host);
    refit();

    handle = {
      clear: () => term?.clear(),
      copy: () => {
        if (!term) return '';
        if (term.hasSelection()) return term.getSelection();
        const buf = term.buffer.active;
        const out: string[] = [];
        for (let i = 0; i < buf.length; i++) out.push(buf.getLine(i)?.translateToString(true) ?? '');
        return out.join('\n').replace(/\n+$/, '');
      },
      focus: () => term?.focus(),
    };

    return () => {
      ro.disconnect();
      unlisten();
      dataSub.dispose();
      term?.dispose();
      term = undefined;
    };
  });

  $effect(() => {
    if (term) term.options.disableStdin = !enabled;
  });
</script>

<div class="xterm-frame"><div class="xterm-host" bind:this={host} title={enabled ? '' : 'Open a port to type'}></div></div>

<style>
  .xterm-frame {
    height: 100%; min-height: 0; box-sizing: border-box; overflow: hidden;
    background: #0a0c10; border: 1px solid var(--border); border-radius: 4px; padding: 4px 0 2px 6px;
  }
  .xterm-host { height: 100%; width: 100%; overflow: hidden; }
  .xterm-host :global(.xterm) { height: 100%; }
  .xterm-host :global(.xterm-viewport) { background: #0a0c10 !important; }
</style>
