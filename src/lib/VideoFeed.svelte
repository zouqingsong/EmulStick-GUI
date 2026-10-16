<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { commands } from "./ipc";
  import { videoPoint, PointerQueue } from "./absolutePointer";

  // `onpick` fires when the operator clicks the screen (to grab input).
  // `devices` / `selectedId` are bindable so the parent can render the capture
  // source picker inside its toolbar (avoids an overlay that collides with the
  // bar). VideoFeed still owns the actual MediaStream lifecycle.
  let {
    onpick,
    locked = false,
    absolute = false,
    onerror,
    devices = $bindable([]),
    selectedId = $bindable(""),
  }: {
    onpick?: () => void;
    locked?: boolean;
    absolute?: boolean;
    onerror?: (message: string) => void;
    devices?: MediaDeviceInfo[];
    selectedId?: string;
  } = $props();

  let videoEl: HTMLVideoElement | undefined = $state();
  let error = $state<string | null>(null);
  let stream: MediaStream | null = null;
  // deviceId currently streaming — guards the $effect against redundant restarts.
  let activeId = "";
  let contact = false;
  let position = { x: 0, y: 0 };
  let pointerFlags = 1;
  const pointerQueue = new PointerQueue(
    (report) => commands.absolutePointer(report.x, report.y, report.flags, report.wheel),
    (failure) => {
      releasePointer();
      onerror?.(String(failure));
    },
  );

  function releasePointer() {
    if (!contact) return;
    contact = false;
    pointerFlags = 1;
    pointerQueue.release({ ...position, flags: 0, wheel: 0 });
  }

  function sendPointer(event: PointerEvent | WheelEvent, motion = false, wheel = 0) {
    if (!absolute || locked || !videoEl || error) return;
    const point = videoPoint(event.clientX, event.clientY, videoEl.getBoundingClientRect(),
      videoEl.videoWidth, videoEl.videoHeight);
    if (!point) {
      releasePointer();
      return;
    }
    position = point;
    contact = true;
    pointerFlags = 1 | ((event.buttons & 1) ? 2 : 0) | ((event.buttons & 2) ? 4 : 0);
    pointerQueue.enqueue({ ...point, flags: pointerFlags, wheel }, motion);
    event.preventDefault();
  }

  function scrollPointer(event: WheelEvent) {
    if (event.deltaY) sendPointer(event, false, event.deltaY < 0 ? 1 : -1);
  }

  $effect(() => {
    if (!absolute || locked) releasePointer();
  });

  async function refreshDevices() {
    const all = await navigator.mediaDevices.enumerateDevices();
    devices = all.filter((d) => d.kind === "videoinput");
  }

  function getStream(deviceId: string): Promise<MediaStream> {
    return navigator.mediaDevices.getUserMedia({
      audio: false,
      video: deviceId
        ? { deviceId: { exact: deviceId }, width: { ideal: 1920 }, height: { ideal: 1080 } }
        : { width: { ideal: 1920 }, height: { ideal: 1080 } },
    });
  }

  async function startStream() {
    stopStream();
    error = null;
    const want = selectedId;
    let s: MediaStream;
    try {
      s = await getStream(want);
    } catch (e) {
      // A persisted/selected device that's no longer present throws here. Fall
      // back to the default camera once rather than stranding the operator on an
      // error (e.g. a remembered source that was since unplugged).
      if (!want) {
        error = e instanceof Error ? e.message : String(e);
        return;
      }
      selectedId = "";
      try {
        s = await getStream("");
      } catch (e2) {
        error = e2 instanceof Error ? e2.message : String(e2);
        return;
      }
    }
    stream = s;
    if (videoEl) videoEl.srcObject = stream;
    // Surface an unplugged capture device (the track ends) instead of freezing
    // on the last frame; clearing the selection lets Retry — or a re-plug via
    // `devicechange` — pick up an available device.
    const track = stream.getVideoTracks()[0];
    if (track)
      track.onended = () => {
        error = "Capture device disconnected.";
        stopStream();
        activeId = "";
        selectedId = "";
        refreshDevices();
      };
    // Labels are only populated once capture permission is granted.
    await refreshDevices();
    const actual = track?.getSettings().deviceId ?? devices[0]?.deviceId ?? "";
    activeId = actual;
    if (selectedId !== actual) selectedId = actual;
  }

  function stopStream() {
    releasePointer();
    stream?.getTracks().forEach((t) => t.stop());
    stream = null;
    if (videoEl) videoEl.srcObject = null;
  }

  // Keep the picker in sync with hot-plugged hardware, and auto-recover the
  // feed: restart if we lost the stream entirely, or if the device we were
  // streaming just disappeared from the list (its `onended` isn't guaranteed to
  // fire, so don't rely on it alone).
  async function onDeviceChange() {
    await refreshDevices();
    if (activeId && !devices.some((d) => d.deviceId === activeId)) {
      stopStream();
      activeId = "";
      selectedId = "";
      startStream();
      return;
    }
    if (!stream) startStream();
  }

  // Restart when the parent's source picker selects a different device.
  $effect(() => {
    if (selectedId && selectedId !== activeId) startStream();
  });

  onMount(() => {
    startStream();
    navigator.mediaDevices.addEventListener("devicechange", onDeviceChange);
  });
  onDestroy(() => {
    stopStream();
    navigator.mediaDevices.removeEventListener("devicechange", onDeviceChange);
  });
</script>

<svelte:window onblur={releasePointer} />

<div class="video-wrap" class:locked
  onclick={() => { if (!absolute) onpick?.(); }}
  onpointermove={(event) => sendPointer(event, true)}
  onpointerdown={(event) => sendPointer(event)}
  onpointerup={(event) => sendPointer(event)}
  onpointerleave={releasePointer} onpointercancel={releasePointer}
  oncontextmenu={(event) => { if (absolute) event.preventDefault(); }}
  onwheel={scrollPointer} role="presentation">
  <!-- svelte-ignore a11y_media_has_caption -->
  <video bind:this={videoEl} autoplay playsinline muted></video>

  {#if error}
    <div class="video-msg">
      <p>Couldn't start capture.</p>
      <p class="mono">{error}</p>
      <button onclick={(e) => (e.stopPropagation(), startStream())}>Retry</button>
    </div>
  {:else if devices.length === 0}
    <div class="video-msg muted">Waiting for a capture device…</div>
  {/if}
</div>

<style>
  .video-wrap {
    position: absolute;
    inset: 0;
    background: #000;
    overflow: hidden;
    cursor: pointer;
  }
  /* While grabbed the OS cursor is frozen at screen-centre over the video; hide
     it (and any child's cursor) so the operator sees only the remote pointer. */
  .video-wrap.locked,
  .video-wrap.locked :global(*) {
    cursor: none;
  }
  video {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .video-msg {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    text-align: center;
    color: #e6e8ec;
    padding: 24px;
  }
</style>
