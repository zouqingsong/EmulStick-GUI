export function videoPoint(
  clientX: number, clientY: number,
  rect: { left: number; top: number; width: number; height: number },
  videoWidth: number, videoHeight: number,
): { x: number; y: number } | null {
  if (videoWidth <= 0 || videoHeight <= 0 || rect.width <= 0 || rect.height <= 0) return null;
  const scale = Math.min(rect.width / videoWidth, rect.height / videoHeight);
  const width = videoWidth * scale;
  const height = videoHeight * scale;
  const x = clientX - rect.left - (rect.width - width) / 2;
  const y = clientY - rect.top - (rect.height - height) / 2;
  if (x < 0 || y < 0 || x > width || y > height) return null;
  return { x: Math.round(x / width * 3840), y: Math.round(y / height * 2160) };
}

export type PointerReport = { x: number; y: number; flags: number; wheel: number };

export class PointerQueue {
  private pending: { report: PointerReport; motion: boolean }[] = [];
  private busy = false;

  constructor(
    private send: (report: PointerReport) => Promise<void>,
    private onError: (error: unknown) => void,
  ) {}

  enqueue(report: PointerReport, motion = false) {
    const last = this.pending[this.pending.length - 1];
    if (motion && last?.motion) last.report = report;
    else this.pending.push({ report, motion });
    void this.drain();
  }

  release(report: PointerReport) {
    this.pending = [];
    this.enqueue({ ...report, flags: 0, wheel: 0 });
  }

  private async drain() {
    if (this.busy) return;
    this.busy = true;
    try {
      while (this.pending.length) await this.send(this.pending.shift()!.report);
    } catch (error) {
      this.pending = [];
      this.onError(error);
    } finally {
      this.busy = false;
      if (this.pending.length) void this.drain();
    }
  }
}