// ── Power Core (energy pulse) ──────────────────────────────────────────────
//
// COPIED VERBATIM from the easyBTX miner's mining-visualizer.ts (the shader is
// byte-identical) so the node's optional "Energy pulse" visual is the SAME
// power core users know from the miner — not a lookalike. The only difference
// is how it's DRIVEN: the node holds it at a low, calm intensity, so the
// fusion-flash (fires only when intensity > 0.5) and the overdrive shockwave
// (fires only at high mining power) never trigger. That's the "energy pulse
// without the heavy explosions" — a running node reads as a live, breathing
// core; a stopped node as a dim gathering one.
//
// (Follow-up: lift this into a shared package so the miner and node don't keep
// two copies. For now, copied per Mende's "it's already built, take it out and
// insert it" — the shader must not drift, so keep this a faithful copy.)
//
// A raw-WebGL fragment-shader "energy core", a single fullscreen quad driving
// an analytically-cheap fragment shader so the per-frame GPU cost stays ~1%.
//
// Two states are driven by ONE `uIntensity` uniform, eased between targets so
// the transition never hard-switches:
//   * IDLE  (intensity → 0): slow, dim, "gathering energy, not yet ignited" —
//     gentle breathing, particles drifting slowly inward, low brightness.
//   * ACTIVE (intensity → 0.30..1.00): fast kinetic streaks, bright pulsing core,
//     energy bursting outward. The exact intensity scales with mining power so
//     the user can SEE the slider:
//
//      Tier 0 (power   5–35%): "whisper"  — calm low-intensity mining
//      Tier 1 (power  36–50%): "steady"   — mid-low mining, slightly more alive
//      Tier 2 (power  51–80%): "active"   — the original active look
//      Tier 3 (power 81–100%): "overdrive"— red-orange hot tint, fast core pulse,
//                                            tiny outward shockwave, and the rare
//                                            fusion-flash drops from ~6–30s to
//                                            ~0.8–4s and lands noticeably harder.
//
// Overdrive is driven by uOverdrive (smoothstep'd 0..1 across the tier boundary)
// so the palette/effects ramp in instead of popping when the slider crosses 80%.
//
// All motion is continuous flow plus periodic sin/cos(uTime), so there is no
// visible loop seam — the animation is infinite and seamless. The canvas is
// transparent (alpha 0 clear, non-premultiplied) so it blends into the dark
// panel. The rAF loop pauses when the document is hidden, and when the user
// prefers reduced motion a single static frame is rendered with no loop.

// ── Shaders ──────────────────────────────────────────────────────────────────

const VERT_SRC = `
attribute vec2 aPos;
void main() {
  gl_Position = vec4(aPos, 0.0, 1.0);
}
`;

// One accent color (the app's blue/cyan action color) threaded in as uAccent.
// Everything else stays neutral. Layers, from back to front:
//   1. background drift   — slow large-scale glow that breathes
//   2. concentric rings   — soft energy filaments rippling toward the core
//   3. pulsing core        — additive radial bloom with a soft chromatic rim
//   4. orbiting particles  — discrete sparks drifting IN (idle) / OUT (active)
//   5. rotating field      — subtle scan grid + sweep that reads as "working"
//
// The whole field is centered on the TRUE canvas center via uRes, and is
// aspect-corrected on the SHORT axis, so it looks correct in both the wide-short
// Tall panel and the larger square hero of the Square layout.
const FRAG_SRC = `
precision highp float;

uniform vec2  uRes;
uniform float uTime;
uniform float uIntensity;   // 0.0 idle .. 1.0 active (already eased on the CPU)
uniform vec3  uAccent;
uniform float uFlash;       // 0..1 directional flash brightness (decays on the CPU)
uniform float uFlashDir;    // -1.0 = flash left, +1.0 = flash right
uniform float uFlashMag;    // per-burst punch (~0.85..1.45) so some hit harder
uniform float uOverdrive;   // 0..1 — smoothstep'd activation of the high-power "overdrive" look
uniform vec3  uAccentHot;   // red-orange accent the overdrive layer blends toward

// Cheap hash + value noise (no textures, a handful of ALU ops).
float hash(vec2 p) {
  p = fract(p * vec2(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}

float noise(vec2 p) {
  vec2 i = floor(p);
  vec2 f = fract(p);
  vec2 u = f * f * (3.0 - 2.0 * f);
  float a = hash(i);
  float b = hash(i + vec2(1.0, 0.0));
  float c = hash(i + vec2(0.0, 1.0));
  float d = hash(i + vec2(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Fractal brownian motion — layered, rotated octaves of value noise. This is the
// turbulence primitive: summing noise at doubling frequencies (with a rotation
// between octaves to break grid alignment) yields the organic, folded structure
// of plasma / nebulae rather than smooth blobs.
float fbm(vec2 p) {
  float v = 0.0;
  float a = 0.5;
  mat2 m = mat2(1.6, 1.2, -1.2, 1.6);
  for (int i = 0; i < 5; i++) {
    v += a * noise(p);
    p = m * p;
    a *= 0.5;
  }
  return v;
}

mat2 rot(float a) {
  float s = sin(a), c = cos(a);
  return mat2(c, -s, s, c);
}

void main() {
  // Aspect-correct, centered coords. uv in roughly [-1,1] on the short axis.
  // Centering on 0.5 * uRes means the core always sits at the true canvas
  // center regardless of the panel's aspect ratio.
  vec2 uv = (gl_FragCoord.xy - 0.5 * uRes) / min(uRes.x, uRes.y);

  float t = uTime;
  float inten = clamp(uIntensity, 0.0, 1.0);

  // Speed + brightness scale smoothly between idle and active. The idle floors
  // are deliberately well above zero so idle reads as ALIVE and "charging",
  // never as a dead dim dot.
  float speed = mix(0.6, 1.8, inten);
  float bright = mix(0.85, 1.35, inten);

  // Distance + angle to the core.
  float r = length(uv);
  float ang = atan(uv.y, uv.x);

  vec3 col = vec3(0.0);

  // Subtle chromatic shimmer: a slowly-rotating channel split so the accent
  // breathes between its true hue and a swapped-channel tint. Calm, never neon.
  float shimmer = 0.5 + 0.5 * sin(t * 0.5 + r * 3.0);
  vec3 accentA = uAccent;
  vec3 accentB = mix(uAccent, uAccent.gbr, 0.5);
  vec3 accent = mix(accentA, accentB, shimmer * mix(0.35, 0.55, inten));

  // ── Overdrive: blend the whole palette toward a hot red-orange tint ──────
  // One lerp here threads the high-tier color through every layer below
  // (rings, core, particles, fusion-flash) without re-doing each one's color
  // logic. uOverdrive is smoothstep'd on the CPU across the 80→81% boundary so
  // the tint ramps in cleanly instead of popping.
  vec3 hotAccent = mix(accent, uAccentHot, 0.55);
  accent = mix(accent, hotAccent, uOverdrive);

  // ── Layer 1: churning nebula (domain-warped fbm) — the "creation" chaos ──
  // Domain warping: sample fbm at a point that is ITSELF displaced by another
  // fbm. The field folds back on itself and churns organically — a forming
  // nebula / birth-of-a-universe feel rather than a smooth drift. The warp
  // animates and gets more turbulent (faster, deeper folds) when active.
  float churn = mix(0.5, 1.4, inten);
  vec2 q = vec2(
    fbm(uv * 1.5 + vec2(0.0, t * 0.12 * churn)),
    fbm(uv * 1.5 + vec2(5.2, -t * 0.10 * churn))
  );
  vec2 warp = uv * 1.6 + 1.4 * q + vec2(t * 0.05 * speed, -t * 0.035 * speed);
  float neb = fbm(warp);
  neb *= neb;                                        // deepen the voids = more contrast/chaos
  float bgGlow = smoothstep(1.2, 0.0, r);            // fade toward the edges
  col += accent * neb * bgGlow * mix(0.32, 0.30, inten) * bright;
  // Hot turbulent veins where the warp folds onto itself — bright filaments that
  // flicker through the cloud, the "matter condensing" detail.
  float veins = smoothstep(0.60, 0.95, fbm(warp * 2.0 + q));
  col += mix(accent, vec3(1.0), 0.30) * veins * bgGlow * mix(0.10, 0.22, inten) * bright;

  // ── Layer 2: concentric energy rings / filaments rippling inward ─────────
  // Soft rings travelling toward the core in idle (gathering), pushed outward
  // and tighter when active. Reads as energy charging into the center.
  float ringDir = mix(1.0, -1.0, inten);             // inward (idle) / outward (active)
  float ringFreq = mix(7.0, 11.0, inten);
  float rings = 0.5 + 0.5 * sin(r * ringFreq + ringDir * t * 1.6 * speed);
  rings = pow(rings, mix(2.2, 3.5, inten));
  float ringMask = smoothstep(0.02, 0.18, r) * smoothstep(1.1, 0.3, r);
  col += accent * rings * ringMask * mix(0.22, 0.30, inten) * bright;

  // ── Layer 3: pulsing energy core (additive radial bloom + chromatic rim) ─
  // Visible breathing in idle (bigger amplitude than before), faster pulse in
  // active. Two incommensurate sin terms so the envelope never repeats short.
  float pulse  = 0.5 + 0.5 * sin(t * mix(1.6, 3.6, inten));
  float pulse2 = 0.5 + 0.5 * sin(t * mix(1.0, 2.3, inten) + 1.7);
  float breathe = mix(pulse, pulse2, 0.5);
  float coreR = mix(0.20, 0.32, inten) * (0.82 + 0.32 * breathe);
  float core = coreR / (r + 0.001);                  // 1/r bloom falloff
  core = pow(core, 1.65) * mix(0.07, 0.055, inten);
  // Soft chromatic rim: nudge the accent toward white in the hot center and
  // bleed a faint cyan/violet split at the rim.
  vec3 hot = mix(accent, vec3(1.0), smoothstep(0.0, 0.18, coreR - r));
  float rim = smoothstep(coreR * 1.9, coreR, r) * (1.0 - smoothstep(coreR, coreR * 0.4, r));
  vec3 rimCol = mix(accent, accent.bgr, 0.4);        // subtle channel swap = chroma
  col += hot * core * bright;
  col += rimCol * rim * mix(0.42, 0.40, inten) * bright;

  // ── Layer 4: orbiting / drifting particles (foreground sparks) ───────────
  // Angular streaks that spiral. Idle: orbit + drift slowly INWARD, clearly
  // visible (not faint). Active: burst OUTWARD, brighter and faster. Continuous
  // flow on t keeps it seamless; the spiral twist on ang reads as orbiting.
  float flowDir = mix(1.0, -1.0, inten);             // inward (idle) vs outward (active)
  float streakT = t * speed * 1.4;
  float spiral = r * 6.0 + ang * 1.6 + flowDir * streakT;
  float streak = pow(0.5 + 0.5 * sin(spiral), mix(3.4, 2.2, inten));
  float annulus = smoothstep(0.04, 0.22, r) * smoothstep(1.08, 0.35, r);
  // Angular sparkle so streaks break into discrete particles, not solid rings.
  float spark = noise(vec2(ang * 3.5, r * 4.0 - streakT * 0.6));
  spark = smoothstep(0.40, 0.95, spark);
  col += accent * streak * annulus * spark * mix(0.40, 0.78, inten) * bright;

  // A second faster, finer particle band so idle has layered motion + depth.
  float spark2 = noise(vec2(ang * 6.0 - streakT * 0.4, r * 7.0 + flowDir * streakT * 0.5));
  spark2 = smoothstep(0.62, 0.98, spark2);
  col += accent * spark2 * annulus * mix(0.20, 0.34, inten) * bright;

  // ── Layer 5: rotating field (subtle structure) ───────────────────────────
  vec2 g = uv * rot(t * 0.12 * speed);
  float scan = sin(g.x * 22.0) * sin(g.y * 22.0);
  scan = smoothstep(0.85, 1.0, scan) * smoothstep(0.95, 0.2, r);
  col += accent * scan * mix(0.07, 0.06, inten) * bright;

  // A faint sweeping scan line, period chosen prime-ish vs the pulse periods.
  float sweep = smoothstep(0.02, 0.0, abs(fract(ang / 6.28318 + t * 0.07 * speed) - 0.5) - 0.014);
  col += accent * sweep * annulus * mix(0.20, 0.22, inten) * bright;

  // ── Fusion-flash: a rare, sharp "mini detonation" while active ────────────
  // uFlash decays 1→0 on the CPU (rare, every ~6–30s). The envelope is SHARP, not
  // a fade: pow() curves make a white-hot core snap off instantly while the body
  // stays bright a touch longer — the shader equivalent of expo.out, so it reads
  // as an explosion. Three layers give it drama + lateral spread:
  //   • beam     — fired toward uFlashDir (kept from the original)
  //   • lateral  — a symmetric horizontal flare spanning BOTH sides (left+right)
  //   • shock    — a thin ring racing outward as it decays (the detonation tell)
  // uFlashMag varies the punch per burst so some are noticeably stronger.
  if (uFlash > 0.001) {
    float fl = uFlash;
    float spike = pow(fl, 0.5);                           // bright across the body
    float snap  = pow(fl, 4.5);                           // white-hot, only at the PEAK (sharper)
    float mag   = uFlashMag;

    float along = uv.x * uFlashDir;                       // >0 on the firing side
    // Tighter vertical band → a thinner, sharper beam (less round).
    float beamBand = exp(-uv.y * uv.y * mix(420.0, 150.0, fl));
    float beam = beamBand
               * smoothstep(-0.04, 0.12, along)
               * smoothstep(1.7 * mag, 0.05, along);

    // Lateral flare — a thinner bright bar across the width: the sideways spread.
    float lateral = exp(-uv.y * uv.y * 520.0) * exp(-abs(uv.x) * mix(3.0, 1.0, fl));

    // Star-burst spikes — a few HARD rays so the detonation reads angular, not a
    // round glow. abs() before pow keeps it defined; the high power = crisp spikes
    // that tighten as the flash snaps off.
    float ang = atan(uv.y, uv.x);
    float rays = pow(abs(cos(ang * 5.0)), mix(8.0, 18.0, 1.0 - fl)); // ~10 sharp rays
    float core = exp(-r * r * mix(34.0, 90.0, 1.0 - fl));            // tightens as it decays
    float starburst = rays * core;

    // Expanding shockwave ring — THIN + fast front = a crisp detonation edge.
    float ringR = (1.0 - fl) * 1.5;
    float sd = (r - ringR) * 16.0;                        // ×16 (was ×8) → thin, sharp ring
    float shock = exp(-sd * sd);                          // (pow(neg,2.0) is UB in GLSL)

    float burst = beam * spike
                + lateral * spike * 0.8
                + starburst * spike * 1.0
                + shock * spike * 0.5;
    col += mix(accent, vec3(1.0), 0.7) * burst * (3.2 * mag) * bright;
    // Hard pure-white kicker at the onset so it lands as a sharp flash, not a glow.
    col += vec3(1.0) * (beamBand + lateral + starburst * 0.7) * snap * (1.5 * mag) * bright;
  }

  // ── Overdrive overheat: fast-pulsing red-orange core at the highest tier ──
  // Layers a tight inner glow + a tiny outward shockwave on top of the existing
  // core, so the "the miner is FLAT OUT" tier reads viscerally — not just a
  // brighter version of the same animation. The pulse frequency (~7 Hz) is
  // intentionally faster than the base breathe so it feels stressed, not calm.
  if (uOverdrive > 0.001) {
    float fastPulse = 0.55 + 0.45 * sin(t * 7.0);
    float overheat = exp(-r * r * 40.0) * fastPulse;
    col += uAccentHot * overheat * uOverdrive * 0.55 * bright;
    float pulseR = 0.18 + 0.07 * fastPulse;
    float dring = (r - pulseR) * 16.0;
    float shockSm = exp(-dring * dring);
    col += uAccentHot * shockSm * uOverdrive * 0.30 * bright;
  }

  // ── Tone + alpha ─────────────────────────────────────────────────────────
  // Vignette so the panel edges fade cleanly into the dark surface.
  col *= smoothstep(1.3, 0.2, r);
  // Gentle filmic-ish curve to tame the additive highlights.
  col = col / (col + vec3(0.6));
  col *= 1.45;

  // Alpha follows luminance so the transparent canvas blends additively into
  // the dark panel (premultiplied disabled at context creation).
  float a = clamp(max(col.r, max(col.g, col.b)) * 1.15, 0.0, 1.0);
  gl_FragColor = vec4(col, a);
}
`;

// ── GL helpers ─────────────────────────────────────────────────────────────

function compile(gl: WebGLRenderingContext, type: number, src: string): WebGLShader {
  const sh = gl.createShader(type);
  if (!sh) throw new Error("createShader failed");
  gl.shaderSource(sh, src);
  gl.compileShader(sh);
  if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(sh);
    gl.deleteShader(sh);
    throw new Error(`shader compile failed: ${log}`);
  }
  return sh;
}

function link(
  gl: WebGLRenderingContext,
  vs: WebGLShader,
  fs: WebGLShader,
): WebGLProgram {
  const prog = gl.createProgram();
  if (!prog) throw new Error("createProgram failed");
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(prog);
    gl.deleteProgram(prog);
    throw new Error(`program link failed: ${log}`);
  }
  return prog;
}

// Parse a CSS color (hex or rgb()) into a 0..1 RGB triple for the shader.
function parseAccent(css: string): [number, number, number] {
  const s = css.trim();
  const hex = s.match(/^#?([0-9a-f]{6})$/i);
  if (hex) {
    const n = parseInt(hex[1], 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
  }
  const rgb = s.match(/rgba?\(([^)]+)\)/i);
  if (rgb) {
    const parts = rgb[1].split(",").map((p) => parseFloat(p.trim()));
    if (parts.length >= 3) return [parts[0] / 255, parts[1] / 255, parts[2] / 255];
  }
  // Fallback: the app's accent — Bitcoin orange (#f7931a).
  return [0xf7 / 255, 0x93 / 255, 0x1a / 255];
}

// Cubic ease (≈ power2.inOut) used to smooth the idle⇄active intensity ramp.
function easeInOut(x: number): number {
  return x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
}

// ── Public controller ────────────────────────────────────────────────────────

export interface MiningVisualizer {
  /** Set the target state. `true` = active (mining), `false` = idle. */
  setActive(active: boolean): void;
  /**
   * Set the mining-power percentage (5..100) the visualizer modulates around.
   * Maps to one of four tiers:
   *   0–35  → whisper, 36–50 → steady, 51–80 → active, 81–100 → overdrive.
   * Only takes visible effect while the visualizer is `active` (mining).
   */
  setPower(percent: number): void;
  /** No-op on the core — present so MiningVisualizer satisfies MiningSkin. */
  setRate(perHr: number): void;
  /** No-op on the core — present so MiningVisualizer satisfies MiningSkin. */
  onShare(delta: number): void;
  /** Tear down GL resources and stop the loop. */
  dispose(): void;
}

/**
 * The four mining-power tiers. Pure helper exposed for any consumer that needs
 * to label / style the current tier alongside the visualizer (e.g. the UI chip).
 */
export function powerTier(percent: number): 0 | 1 | 2 | 3 {
  if (percent <= 35) return 0;
  if (percent <= 50) return 1;
  if (percent <= 80) return 2;
  return 3;
}

// Red-orange hot accent threaded into the shader's `uAccentHot`. Used by the
// overdrive (tier-3) blend so the high-power look reads as "stressed/hot",
// regardless of the user's primary accent theme. #ff5510 lands between a
// pure-red alarm and Bitcoin orange — warm, urgent, still legible against the
// existing accent's blue/orange.
const HOT_ACCENT: readonly [number, number, number] = [0xff / 255, 0x55 / 255, 0x10 / 255];

// Per-theme overdrive (high-power) hot accent. The default Orange shifts to a
// red-orange "alarm". The dark-by-design themes instead stay IN-HUE but brighter,
// so 80–100% power keeps the theme color — Quantum Blue stays a hot electric blue,
// BTX Green a hot lime — instead of washing out to orange. Keyed by data-accent.
const HOT_ACCENT_BY_THEME: Record<string, readonly [number, number, number]> = {
  ocean: [0x4c / 255, 0xcf / 255, 0xff / 255], // hot electric blue (Quantum Blue)
  green: [0x7d / 255, 0xff / 255, 0x4d / 255], // hot lime (BTX Green)
};
function hotAccentForTheme(accentName: string | null): readonly [number, number, number] {
  return (accentName && HOT_ACCENT_BY_THEME[accentName]) || HOT_ACCENT;
}

// Intensity floor while mining: even tier-0 must look ALIVE, not idle. Maps
// power 5..100 → intensity 0.30..1.00 linearly so the shader's existing
// idle⇄active mix() blends pick up gracefully.
function intensityFromPower(p: number, active: boolean): number {
  if (!active) return 0;
  const x = Math.max(0, Math.min(1, (p - 5) / 95));
  return 0.30 + 0.70 * x;
}

// Overdrive ramps in smoothly across power 75..85 so the palette/effects
// don't pop when crossing the 80% tier boundary. smoothstep(75,85,p).
function overdriveFromPower(p: number, active: boolean): number {
  if (!active) return 0;
  const x = Math.max(0, Math.min(1, (p - 75) / 10));
  return x * x * (3 - 2 * x);
}

const TRANSITION_MS = 400; // eased ramp duration on a state change

/**
 * Mount the energy-core visualizer onto `canvas`. Returns a controller whose
 * `setActive` flips between the idle and active states. Returns a no-op
 * controller (and renders nothing) if WebGL is unavailable.
 */
export function mountPowerCore(canvas: HTMLCanvasElement): MiningVisualizer {
  const gl =
    (canvas.getContext("webgl", {
      alpha: true,
      premultipliedAlpha: false,
      antialias: true,
      depth: false,
      stencil: false,
    }) as WebGLRenderingContext | null) ?? null;

  if (!gl) {
    console.warn("[visualizer] WebGL unavailable — skipping mining HUD");
    return { setActive: () => {}, setPower: () => {}, setRate: () => {}, onShare: () => {}, dispose: () => {} };
  }

  // Resolve the accent color from the live CSS (respects light/dark theme).
  const accentCss =
    getComputedStyle(document.documentElement)
      .getPropertyValue("--color-accent")
      .trim() || "#f7931a";
  const accent = parseAccent(accentCss);
  // Re-read live (in tick, ~once a second) so switching the color theme
  // (Orange / BTX Green / Quantum Blue) recolors the energy core WITHOUT a
  // remount. The Silver Surf skin already does this; the core didn't, so it kept
  // its mount-time color — e.g. an orange core left stuck when you picked Quantum
  // Blue, which read as "the blue Power Core is missing".
  let liveAccent: [number, number, number] = accent;
  let liveHot: readonly [number, number, number] = hotAccentForTheme(
    document.documentElement.getAttribute("data-accent"),
  );
  let accentFrame = 0;

  // GL objects are recreated on context-restore, so they're reassignable.
  let vs!: WebGLShader;
  let fs!: WebGLShader;
  let prog!: WebGLProgram;
  let buf: WebGLBuffer | null = null;
  let uRes: WebGLUniformLocation | null = null;
  let uTime: WebGLUniformLocation | null = null;
  let uIntensity: WebGLUniformLocation | null = null;
  let uAccent: WebGLUniformLocation | null = null;
  let uFlash: WebGLUniformLocation | null = null;
  let uFlashDir: WebGLUniformLocation | null = null;
  let uFlashMag: WebGLUniformLocation | null = null;
  let uOverdrive: WebGLUniformLocation | null = null;
  let uAccentHot: WebGLUniformLocation | null = null;

  // (Re)build shaders, program, quad buffer, attrib + uniform locations, and blend
  // state. Called at mount AND on `webglcontextrestored`, so an idle/sleep GPU
  // context loss self-heals instead of leaving a permanently black HUD.
  function setupGl(): void {
    const g = gl!;
    vs = compile(g, g.VERTEX_SHADER, VERT_SRC);
    fs = compile(g, g.FRAGMENT_SHADER, FRAG_SRC);
    prog = link(g, vs, fs);
    g.useProgram(prog);

    // Fullscreen quad (two triangles).
    buf = g.createBuffer();
    g.bindBuffer(g.ARRAY_BUFFER, buf);
    g.bufferData(
      g.ARRAY_BUFFER,
      new Float32Array([-1, -1, 1, -1, -1, 1, -1, 1, 1, -1, 1, 1]),
      g.STATIC_DRAW,
    );
    const aPos = g.getAttribLocation(prog, "aPos");
    g.enableVertexAttribArray(aPos);
    g.vertexAttribPointer(aPos, 2, g.FLOAT, false, 0, 0);

    uRes = g.getUniformLocation(prog, "uRes");
    uTime = g.getUniformLocation(prog, "uTime");
    uIntensity = g.getUniformLocation(prog, "uIntensity");
    uAccent = g.getUniformLocation(prog, "uAccent");
    uFlash = g.getUniformLocation(prog, "uFlash");
    uFlashDir = g.getUniformLocation(prog, "uFlashDir");
    uFlashMag = g.getUniformLocation(prog, "uFlashMag");
    uOverdrive = g.getUniformLocation(prog, "uOverdrive");
    uAccentHot = g.getUniformLocation(prog, "uAccentHot");
    g.uniform3f(uAccent, liveAccent[0], liveAccent[1], liveAccent[2]);
    // Hot accent is static for the lifetime of the program; the shader blends
    // toward it via uOverdrive each frame, so we set it once here.
    g.uniform3f(uAccentHot, liveHot[0], liveHot[1], liveHot[2]);

    // Additive-friendly blending into the transparent panel. Source-over with a
    // non-premultiplied source lets the luminance-driven alpha glow read like
    // additive light on the dark surface without blowing out to white.
    g.enable(g.BLEND);
    g.blendFuncSeparate(g.SRC_ALPHA, g.ONE_MINUS_SRC_ALPHA, g.ONE, g.ONE_MINUS_SRC_ALPHA);
    g.clearColor(0, 0, 0, 0);
  }
  setupGl();

  // DPR 1.5 (not 2): a glow/plasma field needs no full-Retina backing store, and
  // 1.5× cuts the fragment-shader pixel count by ~44% — the shader runs fbm()
  // ~4×/pixel, so fill cost dominates. Invisible softening, real GPU headroom.
  const DPR_CAP = 1.5;
  // Cap the animation to TARGET_FPS so the energy-core shader leaves the GPU free
  // for the Metal miner. rAF still fires at the display rate; the loop just skips
  // the heavy draw until the next frame slot is due. 30fps reads as smooth for
  // ambient motion and roughly halves the per-second GPU cost.
  const TARGET_FPS = 30;
  const MIN_FRAME_MS = 1000 / TARGET_FPS - 1;

  function resize(): void {
    const dpr = Math.min(window.devicePixelRatio || 1, DPR_CAP);
    const w = Math.max(1, Math.round(canvas.clientWidth * dpr));
    const h = Math.max(1, Math.round(canvas.clientHeight * dpr));
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    gl!.viewport(0, 0, canvas.width, canvas.height);
    gl!.uniform2f(uRes, canvas.width, canvas.height);
  }

  // ── State ──────────────────────────────────────────────────────────────
  let time = 0; // accumulated animation time (seconds), delta-driven
  let intensity = 0; // current eased value sent to the shader
  let rampFrom = 0;
  let rampTo = 0;
  let rampStart = 0; // performance.now() at the start of a transition
  let rampActive = false;
  let raf = 0;
  let lastTs = 0;
  let disposed = false;

  // Mining-power tier driver. `currentPower` is the slider value (5..100) we
  // map into intensity/overdrive. `active` tracks whether mining is actually
  // happening — the slider's value only matters while mining; idle is always
  // intensity=0 regardless of power. Overdrive snaps with the power change (the
  // change is in 5% steps, so a tiny pop is barely perceptible vs the cost of a
  // second ramp variable).
  let currentPower = 100;
  let active = false;
  let overdrive = 0; // shader uniform driving the tier-3 hot tint + overheat layer

  // Occasional directional flash. Outside overdrive (tier 0–2): rare, every
  // ~6–30s — kept as a delightful surprise. In overdrive (tier 3): ~0.8–4s with
  // a noticeably harder punch (uFlashMag 1.2–1.8 vs 0.85–1.45) so the highest
  // power tier reads as "constantly bursting". Schedule is recomputed in tick()
  // based on the current overdrive value.
  let flash = 0;
  let flashDir = 1;
  let flashMag = 1; // per-burst punch, randomized on each trigger
  let nextFlashAt = 0; // performance.now() ms; 0 = "reschedule"
  function flashDelay(): number {
    return overdrive > 0.5
      ? 800 + Math.random() * 3200       // ~0.8–4s in overdrive
      : 6000 + Math.random() * 24000;    // ~6–30s elsewhere
  }
  function rollFlashMag(): number {
    return overdrive > 0.5
      ? 1.2 + Math.random() * 0.6        // ~1.2–1.8 in overdrive
      : 0.85 + Math.random() * 0.6;      // ~0.85–1.45 elsewhere
  }
  const FLASH_DECAY_S = 0.45;

  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  function drawFrame(): void {
    resize();
    gl!.clear(gl!.COLOR_BUFFER_BIT);
    gl!.uniform1f(uTime, time);
    gl!.uniform1f(uIntensity, intensity);
    gl!.uniform1f(uFlash, flash);
    gl!.uniform1f(uFlashDir, flashDir);
    gl!.uniform1f(uFlashMag, flashMag);
    gl!.uniform1f(uOverdrive, overdrive);
    gl!.drawArrays(gl!.TRIANGLES, 0, 6);
  }

  function tick(ts: number): void {
    if (disposed) return;
    // Animation disabled in Settings (html[data-anim="off"]): skip all per-frame
    // work + drawing (CSS hides the canvas, leaving a plain themed background), but
    // keep the rAF alive cheaply so motion resumes instantly when re-enabled.
    if (document.documentElement.getAttribute("data-anim") === "off") {
      lastTs = ts;
      raf = requestAnimationFrame(tick);
      return;
    }
    // Frame-rate cap (see TARGET_FPS): skip the heavy shader draw between frame
    // slots so the GPU stays free for mining. The next rAF is still scheduled.
    if (lastTs && ts - lastTs < MIN_FRAME_MS) {
      raf = requestAnimationFrame(tick);
      return;
    }
    const dt = lastTs ? Math.min((ts - lastTs) / 1000, 0.05) : 0.016;
    lastTs = ts;
    time += dt;

    // Recolor live on a theme switch (~once a second at the capped rate). Cheap:
    // one getComputedStyle + a compare; only writes the uniform when it changes.
    if (++accentFrame >= 30) {
      accentFrame = 0;
      const css = getComputedStyle(document.documentElement).getPropertyValue("--color-accent").trim();
      if (css) {
        const next = parseAccent(css);
        if (next[0] !== liveAccent[0] || next[1] !== liveAccent[1] || next[2] !== liveAccent[2]) {
          liveAccent = next;
          gl!.uniform3f(uAccent, next[0], next[1], next[2]);
          // Re-derive the overdrive hot accent so 80–100% power stays in-theme
          // (Quantum Blue stays blue, BTX Green green) instead of going orange.
          liveHot = hotAccentForTheme(document.documentElement.getAttribute("data-accent"));
          gl!.uniform3f(uAccentHot, liveHot[0], liveHot[1], liveHot[2]);
        }
      }
    }

    if (rampActive) {
      const p = Math.min((ts - rampStart) / TRANSITION_MS, 1);
      intensity = rampFrom + (rampTo - rampFrom) * easeInOut(p);
      if (p >= 1) {
        intensity = rampTo;
        rampActive = false;
      }
    }

    // Schedule + fire the directional flash only while genuinely mining.
    // Cadence + magnitude come from helpers that read the current `overdrive`
    // value so tier 3 lands as a faster, harder beat.
    if (intensity > 0.5) {
      if (nextFlashAt === 0) nextFlashAt = ts + flashDelay();
      if (ts >= nextFlashAt && flash <= 0.001) {
        flash = 1;
        flashDir = Math.random() < 0.5 ? -1 : 1;
        flashMag = rollFlashMag();
        nextFlashAt = ts + flashDelay();
      }
    } else {
      nextFlashAt = 0;
    }
    if (flash > 0) flash = Math.max(0, flash - dt / FLASH_DECAY_S);

    drawFrame();
    raf = requestAnimationFrame(tick);
  }

  function startLoop(): void {
    if (reduceMotion || raf || disposed) return;
    lastTs = 0;
    raf = requestAnimationFrame(tick);
  }

  function stopLoop(): void {
    if (raf) {
      cancelAnimationFrame(raf);
      raf = 0;
    }
  }

  // Pause the loop while the document/tab is hidden — no point burning GPU on
  // a window the user can't see, and it keeps the miner's cores free.
  function onVisibility(): void {
    if (document.hidden) {
      stopLoop();
    } else if (!reduceMotion) {
      startLoop();
    } else {
      drawFrame();
    }
  }
  document.addEventListener("visibilitychange", onVisibility);

  // Redraw a static frame on resize even when the loop is paused/reduced.
  function onResize(): void {
    if (reduceMotion || document.hidden) drawFrame();
  }
  window.addEventListener("resize", onResize);

  // Observe the canvas's own box so the viewport tracks container/aspect changes
  // — e.g. switching between the wide-short Tall panel and the square hero, or a
  // fluid window resize that the global `resize` event alone wouldn't catch.
  // `resize()` (called each frame, and here for the paused case) reads
  // clientWidth/clientHeight, so all we need is to nudge a redraw when idle.
  let resizeObserver: ResizeObserver | null = null;
  if (typeof ResizeObserver !== "undefined") {
    resizeObserver = new ResizeObserver(() => {
      if (reduceMotion || document.hidden) drawFrame();
    });
    resizeObserver.observe(canvas);
  }

  // GPU context loss (idle, system sleep, GPU memory pressure on macOS) would
  // otherwise leave every gl.* call a silent no-op while the rAF loop spins
  // forever on a black HUD. Pause on loss; rebuild + restart on restore.
  function onContextLost(e: Event): void {
    e.preventDefault(); // required, or the browser never fires 'restored'
    stopLoop();
  }
  function onContextRestored(): void {
    setupGl();
    resize();
    if (!reduceMotion && !document.hidden) startLoop();
    else drawFrame();
  }
  canvas.addEventListener("webglcontextlost", onContextLost as EventListener);
  canvas.addEventListener("webglcontextrestored", onContextRestored as EventListener);

  // Kick off: reduced-motion users get one static frame and no loop.
  if (reduceMotion) {
    intensity = 0.55; // a representative mid frame, neither dead nor frantic
    drawFrame();
  } else {
    startLoop();
  }

  function retargetIntensity(): void {
    const target = intensityFromPower(currentPower, active);
    overdrive = overdriveFromPower(currentPower, active);
    if (target === rampTo && (rampActive || intensity === target)) return;
    rampFrom = intensity;
    rampTo = target;
    rampStart = performance.now();
    rampActive = true;
    if (reduceMotion) {
      intensity = target;
      rampActive = false;
      drawFrame();
    }
  }

  return {
    setActive(isActive: boolean): void {
      if (active === isActive) return;
      active = isActive;
      retargetIntensity();
    },
    setPower(percent: number): void {
      const p = Math.max(5, Math.min(100, percent | 0));
      if (p === currentPower) return;
      currentPower = p;
      // Only re-target the eased ramp when we're actually mining; an idle
      // change is purely "remembered for next time we start".
      if (active) retargetIntensity();
    },
    setRate(_perHr: number): void {},
    onShare(_delta: number): void {},
    dispose(): void {
      disposed = true;
      stopLoop();
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("resize", onResize);
      canvas.removeEventListener("webglcontextlost", onContextLost as EventListener);
      canvas.removeEventListener("webglcontextrestored", onContextRestored as EventListener);
      resizeObserver?.disconnect();
      gl!.deleteBuffer(buf);
      gl!.deleteProgram(prog);
      gl!.deleteShader(vs);
      gl!.deleteShader(fs);
    },
  };
}
