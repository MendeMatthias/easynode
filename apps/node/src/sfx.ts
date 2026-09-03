// Tiny 16-bit sound effects, SYNTHESIZED with the WebAudio API — no audio files
// are bundled. Deliberate: the app's CSP has no media-src, so a bundled or data:
// <audio> would be blocked, while a WebAudio oscillator loads no resource at all.
// It ships zero binaries and gives exact volume control. Square/pulse waves are
// how 8/16-bit consoles made sound, so this is genuinely chiptune, not a sample.
// Ported from the BTX PQ wallet's sfx.js, plus a distinct "confirmed" chime.
//
// Volume is baked LOW here (MASTER = 0.35 of full scale) so the blips stay decent
// and never annoying. Every call is defensive: a missing or autoplay-suspended
// AudioContext can never throw into a caller on a money path.

let actx: AudioContext | null = null;

function ctx(): AudioContext | null {
  try {
    if (!actx) {
      const AC = window.AudioContext || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
      if (!AC) return null;
      actx = new AC();
    }
    // Browsers start the context "suspended" until a user gesture. resume() is
    // best-effort and .catch'd: on WKWebView it rejects when called outside a
    // gesture, and an un-awaited reject would surface as a global
    // unhandledrejection. primeAudio() (called from the wallet-open click) is
    // what actually unlocks it inside a real gesture.
    if (actx.state === "suspended") actx.resume().catch(() => {});
    return actx;
  } catch {
    return null;
  }
}

/** Create + resume the AudioContext NOW, while a user gesture is on the stack,
 *  so a later poll-driven chime isn't the first thing to touch a suspended
 *  context (WKWebView won't resume outside a gesture, dropping that first coin).
 *  Call from the wallet-open click. Safe to call repeatedly. */
export function primeAudio(): void {
  ctx();
}

const MASTER = 0.35; // ~35% of full scale: present but quiet, never full-volume

// One short note with a fast attack + exponential decay (a classic chiptune blip).
function blip(c: AudioContext, at: number, freq: number, dur: number, vol: number, type: OscillatorType = "square"): void {
  const o = c.createOscillator();
  const g = c.createGain();
  o.type = type;
  o.frequency.setValueAtTime(freq, at);
  g.gain.setValueAtTime(0.0001, at);
  g.gain.exponentialRampToValueAtTime(Math.max(vol, 0.0002), at + 0.008); // ~8ms attack
  g.gain.exponentialRampToValueAtTime(0.0001, at + dur); // decay to ~silence
  o.connect(g);
  g.connect(c.destination);
  o.start(at);
  o.stop(at + dur + 0.02);
}

/** RECEIVED — a bright "coin"-style chirp (B5 → E6), the Mario-coin feel. */
export function playReceived(): void {
  const c = ctx();
  if (!c) return;
  const t = c.currentTime;
  blip(c, t, 987.77, 0.07, MASTER * 0.9); // B5
  blip(c, t + 0.07, 1318.51, 0.18, MASTER); // E6
}

/** CONFIRMED — a settled three-note resolve (C6 → E6 → G6), distinct from the
 *  arrival coin so you can tell "it landed" from "it's now confirmed" by ear. */
export function playConfirmed(): void {
  const c = ctx();
  if (!c) return;
  const t = c.currentTime;
  blip(c, t, 1046.5, 0.08, MASTER * 0.85, "triangle"); // C6
  blip(c, t + 0.08, 1318.51, 0.08, MASTER * 0.9, "triangle"); // E6
  blip(c, t + 0.16, 1567.98, 0.2, MASTER, "triangle"); // G6
}

/** SENT — a confident two-note rise (C5 → G5), like the tx "leaving". */
export function playSent(): void {
  const c = ctx();
  if (!c) return;
  const t = c.currentTime;
  blip(c, t, 523.25, 0.09, MASTER * 0.9); // C5
  blip(c, t + 0.085, 783.99, 0.14, MASTER); // G5
}
