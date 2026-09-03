// The ambient "frequency line" — the node's heartbeat, drawn calm.
//
// Not the miner's explosive power core: this is the chill sibling. A single
// soft line breathes across the canvas out of layered slow sine waves; when
// the chain advances (a new block arrives) one gentle pulse travels through
// the line — the network "playing a note". That's all it ever does.
//
// Deliberately low-power: ~24 fps cap, one stroke path per frame, no
// offscreen buffers, and it fully stops when the window is hidden. The calm
// is an engineering property, not just a look.

type AmbientMode = "ready" | "syncing" | "stopped" | "error";

interface Pulse {
  /** 0..1 position of the traveling pulse along the line. */
  x: number;
  /** Remaining strength 0..1 (fades as it travels). */
  power: number;
}

const FPS_CAP = 24;
const FRAME_MS = 1000 / FPS_CAP;

export class AmbientLine {
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;
  private mode: AmbientMode = "stopped";
  private pulses: Pulse[] = [];
  private t = 0;
  private lastFrame = 0;
  private raf = 0;
  private running = false;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    this.ctx = canvas.getContext("2d")!;
    // Pause completely when the window is hidden (tray-only / minimized):
    // an invisible animation is pure battery drain.
    document.addEventListener("visibilitychange", () => {
      if (document.hidden) this.stop();
      else this.start();
    });
  }

  setMode(mode: AmbientMode) {
    this.mode = mode;
  }

  /** A new block (or peer event) — send one gentle note through the line. */
  pulse() {
    // Keep it chill: never more than 3 concurrent notes.
    if (this.pulses.length < 3) this.pulses.push({ x: -0.1, power: 1 });
  }

  start() {
    if (this.running) return;
    this.running = true;
    this.lastFrame = 0;
    const loop = (ts: number) => {
      if (!this.running) return;
      this.raf = requestAnimationFrame(loop);
      if (ts - this.lastFrame < FRAME_MS) return; // fps cap
      this.lastFrame = ts;
      this.renderFrame();
    };
    this.raf = requestAnimationFrame(loop);
  }

  stop() {
    this.running = false;
    cancelAnimationFrame(this.raf);
  }

  /** Resolve the current theme accent from CSS so themes restyle the line. */
  private colors(): { line: string; glow: string } {
    const css = getComputedStyle(document.documentElement);
    const accent = css.getPropertyValue("--color-accent").trim() || "#f7931a";
    const green = css.getPropertyValue("--color-green").trim() || "#4ade80";
    const muted = css.getPropertyValue("--color-muted").trim() || "#9b9790";
    const danger = css.getPropertyValue("--color-danger").trim() || "#f87171";
    switch (this.mode) {
      case "ready":
        return { line: green, glow: green };
      case "syncing":
        return { line: accent, glow: accent };
      case "error":
        return { line: danger, glow: danger };
      default:
        return { line: muted, glow: "transparent" };
    }
  }

  /** Draw one frame. Public so QA harnesses can step frames without rAF
   *  (headless browsers suspend requestAnimationFrame entirely). */
  renderFrame() {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = this.canvas.clientWidth;
    const h = this.canvas.clientHeight;
    if (w === 0 || h === 0) return;
    if (this.canvas.width !== w * dpr || this.canvas.height !== h * dpr) {
      this.canvas.width = w * dpr;
      this.canvas.height = h * dpr;
    }
    const ctx = this.ctx;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    this.t += 1 / FPS_CAP;
    const { line, glow } = this.colors();
    const mid = h / 2;
    // Base amplitude breathes slowly; a stopped node lies almost flat.
    const alive = this.mode === "ready" || this.mode === "syncing";
    const breathe = alive ? 1 + 0.35 * Math.sin(this.t * 0.9) : 0.25;
    const baseAmp = (h / 7) * breathe;

    // Advance pulses (one full crossing ≈ 3 s).
    for (const p of this.pulses) {
      p.x += 1 / (3 * FPS_CAP);
      p.power = Math.max(0, 1 - p.x);
    }
    this.pulses = this.pulses.filter((p) => p.x < 1.15);

    ctx.beginPath();
    const STEPS = 96; // straight segments approximate the curve finely enough
    for (let i = 0; i <= STEPS; i++) {
      const u = i / STEPS;
      const x = u * w;
      // Layered slow sines — irregular enough to feel organic, cheap to compute.
      let y =
        Math.sin(u * 6.3 + this.t * 1.1) * 0.55 +
        Math.sin(u * 11.7 - this.t * 0.7) * 0.3 +
        Math.sin(u * 23.1 + this.t * 0.35) * 0.15;
      // Edge fade so the line dissolves into the background at both ends.
      const edge = Math.sin(u * Math.PI);
      y *= baseAmp * edge;
      // Traveling notes: a gaussian bump riding through.
      for (const p of this.pulses) {
        const d = u - p.x;
        y += Math.exp(-(d * d) / 0.002) * p.power * (h / 4) * Math.sin(this.t * 8 + u * 40) * edge;
      }
      if (i === 0) ctx.moveTo(x, mid + y);
      else ctx.lineTo(x, mid + y);
    }
    // Glow via a second, wide low-alpha stroke of the SAME path — two cheap
    // strokes instead of canvas shadowBlur, which forces a full Gaussian pass
    // over the stroke's bounding box every frame (the single most expensive
    // thing this "deliberately low-power" renderer could do).
    if (alive && glow !== "transparent") {
      ctx.strokeStyle = glow;
      ctx.globalAlpha = 0.16;
      ctx.lineWidth = 7;
      ctx.stroke();
    }
    ctx.strokeStyle = line;
    ctx.globalAlpha = alive ? 0.9 : 0.5;
    ctx.lineWidth = 1.6;
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
}
