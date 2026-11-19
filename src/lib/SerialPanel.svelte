<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { commands } from './ipc';
  import { encodeInput, formatLine, formatTime } from './serialTerminal';
  import { initSerial, clearSession, log, saveSettings, serial, touchLog } from './serial.svelte';
  import SerialTerminalView from './SerialTerminalView.svelte';
  import type { SerialPortInfo } from './types';

  let { expanded = $bindable(false), fill = false, onerror }: {
    expanded?: boolean; fill?: boolean; onerror: (message: string) => void;
  } = $props();

  const BAUDS = [300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 74880, 115200, 230400, 460800, 921600, 1000000, 1500000, 2000000, 3000000];
  /** DOM is capped for responsiveness; Copy still takes the whole log. */
  const RENDER_LINES = 2000;

  let ports = $state<SerialPortInfo[]>([]);
  let busy = $state(false);
  let input = $state('');
  let historyIndex = -1;
  let follow = $state(true);
  let dtr = $state(true);
  let rts = $state(true);
  let logEl: HTMLDivElement | undefined = $state();
  let panelEl: HTMLDetailsElement | undefined = $state();
  let summaryEl: HTMLElement | undefined = $state();
  let bodyHeight = $state(0);
  let termHandle = $state<{ clear: () => void; copy: () => string; focus: () => void }>();
  const s = serial.settings;
  // Type straight into the output like PuTTY; Hex/Time switch to the
  // line log, which keeps the separate send box.
  const interactive = $derived(!s.hex && !s.timestamps);
  let baudSelection = $state(BAUDS.includes(s.baudRate) ? String(s.baudRate) : 'custom');

  const lines = $derived.by(() => {
    void serial.version;
    const all = log.view(s.hex);
    return all.length > RENDER_LINES ? all.slice(-RENDER_LINES) : all.slice();
  });

  onMount(() => {
    initSerial();
    refreshPorts();
  });

  // <details> content isn't a reliable flex item across webviews, so in fill
  // mode size the body explicitly from the panel's own height.
  $effect(() => {
    if (!fill || !expanded || !panelEl) {
      bodyHeight = 0;
      return;
    }
    const measure = () => {
      if (panelEl && summaryEl) bodyHeight = Math.max(0, panelEl.clientHeight - summaryEl.offsetHeight);
    };
    const ro = new ResizeObserver(measure);
    ro.observe(panelEl);
    measure();
    return () => ro.disconnect();
  });

  // Keep the newest output in view unless the operator scrolled up to read.
  $effect(() => {
    if (interactive) return;
    void lines;
    if (!follow || !logEl) return;
    tick().then(() => logEl && (logEl.scrollTop = logEl.scrollHeight));
  });

  async function refreshPorts() {
    try {
      ports = await commands.serialListPorts();
      if (!s.port || !ports.some((p) => p.name === s.port)) {
        // Keep the open port selected even if enumeration hiccups.
        s.port = serial.port ?? ports[0]?.name ?? '';
      }
    } catch (error) {
      onerror(String(error));
    }
  }

  async function toggleOpen() {
    busy = true;
    serial.error = null;
    try {
      if (serial.open) {
        await commands.serialClose();
      } else {
        if (!s.port) throw new Error('Select a serial port first');
        const baudRate = Number(s.baudRate);
        if (!Number.isInteger(baudRate) || baudRate <= 0) throw new Error('Enter a valid baud rate');
        s.baudRate = baudRate;
        saveSettings();
        await commands.serialOpen(s.port, {
          baudRate, dataBits: Number(s.dataBits) as 8, parity: s.parity,
          stopBits: Number(s.stopBits) as 1, flowControl: s.flowControl,
        });
        dtr = rts = true;
        await tick();
        termHandle?.focus();
      }
    } catch (error) {
      onerror(String(error).replace(/^Error: /, ''));
    } finally {
      busy = false;
    }
  }

  async function send() {
    if (!serial.open) return;
    try {
      const data = encodeInput(input, s.lineEnding, s.hexSend);
      if (!data.length) return;
      await commands.serialWrite(data);
      if (s.echo) {
        log.addSent(s.hexSend ? input.trim() : input);
        touchLog();
      }
      if (input && serial.history[serial.history.length - 1] !== input) serial.history.push(input);
      if (serial.history.length > 100) serial.history.shift();
      historyIndex = -1;
      input = '';
    } catch (error) {
      onerror(String(error).replace(/^Error: /, ''));
    }
  }

  function onInputKey(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.isComposing) {
      event.preventDefault();
      send();
    } else if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
      const h = serial.history;
      if (!h.length) return;
      event.preventDefault();
      if (event.key === 'ArrowUp') historyIndex = historyIndex < 0 ? h.length - 1 : Math.max(0, historyIndex - 1);
      else historyIndex = historyIndex < 0 ? -1 : historyIndex + 1;
      if (historyIndex >= h.length) historyIndex = -1;
      input = historyIndex < 0 ? '' : h[historyIndex];
    }
  }

  async function setSignal(name: 'dtr' | 'rts', value: boolean) {
    try {
      await commands.serialSetSignals({ [name]: value });
      if (name === 'dtr') dtr = value;
      else rts = value;
    } catch (error) {
      onerror(String(error));
    }
  }

  function onScroll() {
    if (!logEl) return;
    follow = logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 24;
  }

  function selectBaud(value: string) {
    baudSelection = value;
    if (value !== 'custom') {
      s.baudRate = Number(value);
      saveSettings();
    }
  }

  function clearLog() {
    clearSession();
    termHandle?.clear();
  }

  async function copyLog() {
    const text = interactive && termHandle
      ? termHandle.copy()
      : log.view(s.hex).map((l) => formatLine(l, s.timestamps)).join('\n');
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const area = Object.assign(document.createElement('textarea'), { value: text });
      document.body.append(area);
      area.select();
      document.execCommand('copy');
      area.remove();
    }
  }
</script>

<details class="serial-panel" class:fill bind:open={expanded} bind:this={panelEl}>
  <summary bind:this={summaryEl}>
    Serial terminal
    {#if serial.open}<span class="serial-badge">● {serial.port?.replace(/^\/dev\//, '')} · {s.baudRate}</span>{/if}
  </summary>
  <div class="serial-body" style:height={fill && bodyHeight ? `${bodyHeight}px` : null}>
    <div class="serial-row">
      <select class="grow" aria-label="Serial port" bind:value={s.port} disabled={serial.open || busy} title={s.port}>
        {#if !ports.length}<option value="">No serial ports found</option>{/if}
        {#each ports as port (port.name)}
          <option value={port.name}>{port.name.replace(/^\/dev\//, '')}{port.description ? ` — ${port.description}` : ''}</option>
        {/each}
      </select>
      <button class="secondary icon" onclick={refreshPorts} disabled={serial.open || busy} title="Refresh port list" aria-label="Refresh port list">↻</button>
      <button onclick={toggleOpen} disabled={busy || (!serial.open && !s.port)} class:danger={serial.open}>
        {serial.open ? 'Close' : 'Open'}
      </button>
    </div>

    <div class="serial-row settings">
      <select class="baud" aria-label="Baud rate" value={baudSelection} onchange={(e) => selectBaud(e.currentTarget.value)} disabled={serial.open} title="Baud rate">
        {#each BAUDS as b}<option value={b}>{b}</option>{/each}
        <option value="custom">Custom…</option>
      </select>
      {#if baudSelection === 'custom'}
        <input class="baud custom-baud" aria-label="Custom baud rate" type="number" min="1" step="1" bind:value={s.baudRate} disabled={serial.open} inputmode="numeric" title="Enter a custom baud rate" />
      {/if}
      <select aria-label="Data bits" bind:value={s.dataBits} disabled={serial.open} title="Data bits">
        {#each [8, 7, 6, 5] as n}<option value={n}>{n}</option>{/each}
      </select>
      <select aria-label="Parity" bind:value={s.parity} disabled={serial.open} title="Parity">
        <option value="none">N</option><option value="even">E</option><option value="odd">O</option>
      </select>
      <select aria-label="Stop bits" bind:value={s.stopBits} disabled={serial.open} title="Stop bits">
        <option value={1}>1</option><option value={2}>2</option>
      </select>
      <select aria-label="Flow control" bind:value={s.flowControl} disabled={serial.open} title="Flow control">
        <option value="none">No flow</option><option value="hardware">RTS/CTS</option><option value="software">XON/XOFF</option>
      </select>
    </div>

    <div class="serial-row options">
      <label title="Prefix each line with its receive time"><input type="checkbox" bind:checked={s.timestamps} onchange={saveSettings} />Time</label>
      <label title="Show received bytes as hex"><input type="checkbox" bind:checked={s.hex} onchange={saveSettings} />Hex</label>
      {#if !interactive}
        <label title="Show sent lines in the log"><input type="checkbox" bind:checked={s.echo} onchange={saveSettings} />Echo</label>
      {/if}
      {#if serial.open}
        <label title="Data Terminal Ready (many dev boards use it for reset)"><input type="checkbox" checked={dtr} onchange={(e) => setSignal('dtr', e.currentTarget.checked)} />DTR</label>
        <label title="Request To Send"><input type="checkbox" checked={rts} onchange={(e) => setSignal('rts', e.currentTarget.checked)} />RTS</label>
      {/if}
      <span class="grow"></span>
      <button class="secondary small" onclick={copyLog}>Copy</button>
      <button class="secondary small" onclick={clearLog}>Clear</button>
    </div>

    {#if interactive}
      <div class="serial-term">
        <SerialTerminalView bind:handle={termHandle} enabled={serial.open} {onerror} />
      </div>
    {:else}
    <div class="serial-log" class:hex={s.hex} bind:this={logEl} onscroll={onScroll} role="log" aria-live="off" tabindex="-1">
      {#each lines as line}
        <div class:tx={line.dir === 'tx'}>{#if s.timestamps}<span class="ts">{formatTime(line.time)}</span>{/if}{#if line.dir === 'tx'}<span class="arrow">→</span>{/if}{line.text || '\u200b'}</div>
      {:else}
        <div class="placeholder">{serial.open ? 'Waiting for data…' : 'Open a port to see the device log.'}</div>
      {/each}
    </div>
    {#if !follow}
      <button class="secondary small jump" onclick={() => { follow = true; logEl && (logEl.scrollTop = logEl.scrollHeight); }}>↓ Jump to latest</button>
    {/if}
    {/if}

    {#if serial.error}<div class="serial-error" role="alert">{serial.error}</div>{/if}

    {#if !interactive}
    <div class="serial-row send">
      <input class="grow mono-input" aria-label="Data to send" bind:value={input} onkeydown={onInputKey} disabled={!serial.open}
        placeholder={s.hexSend ? 'Hex bytes, e.g. 0D 0A' : 'Type a command, Enter to send (↑/↓ history)'} autocomplete="off" spellcheck="false" />
      <select aria-label="Line ending" bind:value={s.lineEnding} onchange={saveSettings} title="Appended to each sent line">
        <option value="crlf">CR+LF</option><option value="lf">LF</option><option value="cr">CR</option><option value="none">None</option>
      </select>
      <label class="hexsend" title="Interpret input as hex bytes"><input type="checkbox" bind:checked={s.hexSend} onchange={saveSettings} />Hex</label>
      <button onclick={send} disabled={!serial.open}>Send</button>
    </div>
    {/if}
  </div>
</details>

<style>
  .serial-panel { flex: none; border-top: 1px solid var(--border); background: var(--panel); }
  /* Fill mode: the panel owns a full-height column; the log takes the slack. */
  .serial-panel.fill { min-height: 0; height: 100%; border-top: 0; overflow: hidden; }
  .serial-panel.fill .serial-body { box-sizing: border-box; min-height: 0; padding-top: 2px; }
  .serial-panel.fill .serial-log { flex: 1; min-height: 120px; height: auto; resize: none; font-size: 13px; }
  .serial-term { height: var(--serial-log-height, 260px); min-height: 0; }
  .serial-panel.fill .serial-term { flex: 1; min-height: 120px; height: auto; }
  .serial-panel.fill .jump { bottom: 56px; }
  summary { padding: 10px 14px; cursor: pointer; font-weight: 600; display: list-item; }
  .serial-badge { margin-left: 8px; font-weight: 500; font-size: 12px; color: var(--green); }
  .serial-body { padding: 0 14px 10px; display: flex; flex-direction: column; gap: 6px; position: relative; }
  .serial-row { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .serial-row.settings, .serial-row.options { flex-wrap: wrap; }
  .grow { flex: 1; min-width: 0; }
  select, input:not([type='checkbox']) {
    border: 1px solid var(--border); border-radius: 4px; background: var(--bg); color: var(--text);
    padding: 5px 6px; font: inherit; font-size: 12px;
  }
  select:disabled, input:disabled { opacity: 0.55; }
  select.baud { width: 112px; }
  .custom-baud { width: 90px; }
  label { display: inline-flex; align-items: center; gap: 3px; font-size: 12px; color: var(--muted); cursor: pointer; }
  button.icon { padding: 4px 8px; }
  button.small { padding: 3px 8px; font-size: 12px; }
  button.danger { background: var(--red); }
  .serial-log {
    height: var(--serial-log-height, 220px); overflow: auto; resize: vertical;
    background: #0a0c10; border: 1px solid var(--border); border-radius: 4px; padding: 6px 8px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; line-height: 1.4;
    white-space: pre-wrap; word-break: break-all; user-select: text; -webkit-user-select: text; cursor: text;
  }
  .serial-log.hex { white-space: pre; word-break: normal; }
  .serial-log .tx { color: #7dd3fc; }
  .ts { color: var(--muted); margin-right: 8px; }
  .arrow { margin-right: 6px; }
  .placeholder { color: var(--muted); font-family: inherit; }
  .jump { position: absolute; right: 24px; bottom: 52px; opacity: 0.9; }
  .serial-error { font-size: 12px; color: var(--red); }
  .mono-input { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
  .hexsend { flex: none; }
</style>
