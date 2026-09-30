// 音效：全部以 Web Audio 即時合成，不需要音檔，離線也能播放。
//
// - 棋子敲擊用模態合成（modal synthesis），模擬硬木棋子落在厚實木盤上：棋子本身的非諧波模態
//   （1 : 2.76 : 5.40 : 8.93）以低頻為主、高階模態弱而短；柔和的接觸脈衝；木盤的三個低頻共鳴模態；
//   最後低通讓音色溫潤。每一下的音高、力度、衰減都有細微隨機，避免機械感。
// - 依棋子所在欄位左右聲像定位；程序產生的房間殘響讓聲音有空間感。
// - 終局樂句用 Karplus–Strong 撥弦合成，五聲音階，接近古箏的音色。

export type Outcome = "win" | "lose" | "draw";

const rand = (a: number, b: number) => a + Math.random() * (b - a);

/** 五聲音階（D 宮）：宮 商 角 徵 羽 */
const NOTE = { D3: 146.83, A3: 220, B3: 246.94, D4: 293.66, E4: 329.63, "F#4": 369.99, A4: 440, B4: 493.88, D5: 587.33, E5: 659.25, "F#5": 739.99, A5: 880 };

interface Voice {
  pan?: number;
  gain?: number;
  /** 殘響送出量 */
  send?: number;
  /** 相對現在的延遲（秒） */
  at?: number;
}

export class Sound {
  enabled = true;
  private vol = 0.8;
  private ctx: AudioContext | null = null;
  private bus!: GainNode;
  private reverb!: ConvolverNode;
  private master!: GainNode;
  private strings = new Map<string, AudioBuffer>();

  constructor() {
    // 瀏覽器規定音訊要在使用者互動後才能開始
    const unlock = () => {
      this.ensure();
      if (this.ctx?.state === "running") {
        removeEventListener("pointerdown", unlock, true);
        removeEventListener("keydown", unlock, true);
      }
    };
    addEventListener("pointerdown", unlock, true);
    addEventListener("keydown", unlock, true);
  }

  set volume(v: number) {
    this.vol = v;
    if (this.ctx) this.master.gain.setTargetAtTime(v, this.ctx.currentTime, 0.02);
  }

  /** 已解鎖且開啟時才回傳 context */
  private live(): AudioContext | null {
    if (!this.enabled || !this.ctx || this.ctx.state !== "running") return null;
    return this.ctx;
  }

  private ensure() {
    if (!this.ctx) {
      const ctx = new AudioContext({ latencyHint: "interactive" });
      this.ctx = ctx;
      const comp = ctx.createDynamicsCompressor();
      comp.threshold.value = -14;
      comp.knee.value = 12;
      comp.ratio.value = 4;
      comp.attack.value = 0.002;
      comp.release.value = 0.15;
      this.master = ctx.createGain();
      this.master.gain.value = this.vol;
      this.bus = ctx.createGain();
      this.reverb = ctx.createConvolver();
      this.reverb.buffer = roomImpulse(ctx);
      const wet = ctx.createGain();
      wet.gain.value = 0.9;
      // 整體再壓一點高頻，讓音色沉穩
      const shelf = ctx.createBiquadFilter();
      shelf.type = "highshelf";
      shelf.frequency.value = 3000;
      shelf.gain.value = -6;
      this.bus.connect(shelf);
      this.reverb.connect(wet).connect(shelf);
      shelf.connect(comp);
      comp.connect(this.master).connect(ctx.destination);
    }
    if (this.ctx.state === "suspended") void this.ctx.resume();
  }

  private play(ctx: AudioContext, buf: AudioBuffer, v: Voice, rate = 1) {
    const src = ctx.createBufferSource();
    src.buffer = buf;
    src.playbackRate.value = rate;
    const g = ctx.createGain();
    g.gain.value = v.gain ?? 1;
    const p = ctx.createStereoPanner();
    p.pan.value = Math.max(-1, Math.min(1, v.pan ?? 0));
    src.connect(g).connect(p);
    p.connect(this.bus);
    const send = ctx.createGain();
    send.gain.value = v.send ?? 0.12;
    p.connect(send).connect(this.reverb);
    src.start(ctx.currentTime + 0.005 + (v.at ?? 0));
  }

  private hit(ctx: AudioContext, o: HitOpts, v: Voice) {
    this.play(ctx, woodHit(ctx, o), v);
  }

  private noise(ctx: AudioContext, o: NoiseOpts, v: Voice) {
    this.play(ctx, filteredNoise(ctx, o), v);
  }

  /** 拿起棋子（選取）：指尖輕扣 */
  select(pan = 0) {
    const ctx = this.live();
    if (!ctx) return;
    this.hit(ctx, { f0: rand(820, 900), decay: 0.022, body: 0.15, hardness: 0.35 }, { pan, gain: 0.26, send: 0.05 });
  }

  /** 走子：棋子在木盤上低沉地滑過，落定時一聲「篤」（配合 240ms 的移動動畫） */
  move(pan = 0) {
    const ctx = this.live();
    if (!ctx) return;
    this.noise(ctx, { dur: 0.2, lo: 180, hi: 1300, attack: 0.35 }, { pan, gain: 0.09, send: 0.03 });
    this.hit(ctx, { f0: rand(560, 640), decay: rand(0.045, 0.055), body: 0.8, hardness: 0.5 }, { pan, gain: 0.72, at: 0.2 });
  }

  /** 翻子：指尖掀起棋子，翻面後穩穩落下 */
  flip(pan = 0) {
    const ctx = this.live();
    if (!ctx) return;
    this.hit(ctx, { f0: rand(900, 980), decay: 0.016, body: 0.1, hardness: 0.3 }, { pan, gain: 0.14, send: 0.03 });
    this.noise(ctx, { dur: 0.08, lo: 400, hi: 1800, attack: 0.5 }, { pan, gain: 0.06, send: 0.04, at: 0.02 });
    this.hit(ctx, { f0: rand(640, 720), decay: rand(0.042, 0.05), body: 0.7, hardness: 0.55 }, { pan, gain: 0.68, at: 0.13 });
    // 落定後的一下輕彈
    this.hit(ctx, { f0: rand(650, 730), decay: 0.02, body: 0.2, hardness: 0.3 }, { pan, gain: 0.12, at: 0.185 });
  }

  /** 吃子：重重拍下、撞開對方棋子，被吃的子被收到一旁 */
  capture(pan = 0) {
    const ctx = this.live();
    if (!ctx) return;
    this.noise(ctx, { dur: 0.17, lo: 160, hi: 1200, attack: 0.4 }, { pan, gain: 0.08, send: 0.03 });
    this.hit(ctx, { f0: rand(470, 520), decay: rand(0.07, 0.085), body: 1.2, hardness: 0.75 }, { pan, gain: 1, at: 0.19, send: 0.16 });
    this.hit(ctx, { f0: rand(700, 780), decay: 0.03, body: 0.3, hardness: 0.6 }, { pan: pan * 1.2, gain: 0.34, at: 0.222 });
    // 被吃的棋子在一旁輕碰幾下
    const side = pan >= 0 ? 0.85 : -0.85;
    for (const [t, g] of [
      [0.36, 0.14],
      [0.45, 0.09],
      [0.52, 0.05],
    ] as const)
      this.hit(ctx, { f0: rand(680, 820), decay: 0.02, body: 0.1, hardness: 0.35 }, { pan: side, gain: g, at: t + rand(-0.01, 0.01) });
  }

  /** 不能這樣走：兩下悶響 */
  deny(pan = 0) {
    const ctx = this.live();
    if (!ctx) return;
    this.hit(ctx, { f0: 300, decay: 0.028, body: 0.6, hardness: 0.15 }, { pan, gain: 0.34, send: 0.02 });
    this.hit(ctx, { f0: 270, decay: 0.028, body: 0.6, hardness: 0.15 }, { pan, gain: 0.3, send: 0.02, at: 0.09 });
  }

  /** 開局洗牌：一把棋子在盤上翻攪、碰撞 */
  shuffle() {
    const ctx = this.live();
    if (!ctx) return;
    this.noise(ctx, { dur: 1.0, lo: 250, hi: 1600, attack: 0.2 }, { gain: 0.07, send: 0.1 });
    for (let i = 0; i < 30; i++) {
      const t = Math.pow(Math.random(), 0.8) * 0.95;
      this.hit(ctx, { f0: rand(520, 950), decay: rand(0.018, 0.035), body: rand(0.1, 0.5), hardness: rand(0.3, 0.6) }, {
        pan: rand(-0.8, 0.8),
        gain: rand(0.1, 0.32) * (1 - t * 0.5),
        at: t,
        send: 0.12,
      });
    }
    // 最後把牌陣排整齊
    for (let i = 0; i < 4; i++)
      this.hit(ctx, { f0: rand(560, 640), decay: 0.04, body: 0.7, hardness: 0.45 }, { pan: rand(-0.3, 0.3), gain: 0.34, at: 1.05 + i * 0.07 });
  }

  /** 終局樂句（古箏風撥弦，五聲音階） */
  end(result: Outcome, delay = 0.35) {
    const ctx = this.live();
    if (!ctx) return;
    type N = keyof typeof NOTE;
    const phrase: [N, number, number][] =
      result === "win"
        ? // 由宮音上行的刮奏，落在高八度的宮、徵
          [["D4", 0, 0.5], ["E4", 0.06, 0.5], ["F#4", 0.12, 0.55], ["A4", 0.18, 0.6], ["B4", 0.24, 0.6], ["D5", 0.3, 0.75], ["A4", 0.62, 0.45], ["D5", 0.62, 0.7], ["A5", 0.9, 0.35]]
        : result === "lose"
          ? // 羽調下行，慢而低
            [["B4", 0, 0.55], ["A4", 0.22, 0.5], ["F#4", 0.44, 0.5], ["E4", 0.7, 0.5], ["B3", 1.0, 0.6], ["F#4", 1.0, 0.25]]
          : // 和局：宮、徵兩音平穩收束
            [["A4", 0, 0.5], ["D4", 0.28, 0.55], ["A3", 0.28, 0.35]];
    phrase.forEach(([note, t, g], i) => {
      const buf = this.string(ctx, note, NOTE[note], result === "lose" ? 0.28 : 0.4);
      this.play(ctx, buf, { pan: ((i % 5) - 2) * 0.18, gain: g, send: 0.45, at: delay + t });
    });
  }

  private string(ctx: AudioContext, key: string, f: number, bright: number) {
    const id = `${key}:${bright}`;
    let b = this.strings.get(id);
    if (!b) this.strings.set(id, (b = pluck(ctx, f, bright)));
    return b;
  }
}

// ───────────────────────── 合成 ─────────────────────────

interface HitOpts {
  /** 棋子本身的基頻（Hz） */
  f0: number;
  /** 基頻的衰減時間常數（秒） */
  decay: number;
  /** 木盤共鳴的量 */
  body: number;
  /** 撞擊硬度（0–1）：越硬接觸越短、高頻越多 */
  hardness: number;
}

/** 硬木棋子落在厚木盤上 */
export function woodHit(ctx: AudioContext, o: HitOpts): AudioBuffer {
  const sr = ctx.sampleRate;
  const len = Math.ceil(sr * (Math.max(o.decay * 6, 0.16) + 0.05));
  const buf = ctx.createBuffer(1, len, sr);
  const out = buf.getChannelData(0);
  // 棋子：自由–自由樑的非諧波模態，高階模態弱而短（硬木的內部阻尼讓高頻很快消失）
  const modes: [number, number, number][] = [
    [1, 1, 1],
    [2.756 * rand(0.985, 1.015), 0.3, 0.42],
    [5.404 * rand(0.98, 1.02), 0.09, 0.24],
    [8.933 * rand(0.98, 1.02), 0.03, 0.15],
  ];
  for (const [ratio, amp, dk] of modes) {
    const f = o.f0 * ratio;
    const w = (2 * Math.PI * f) / sr;
    const tau = o.decay * dk * sr;
    const a = ratio === 1 ? amp : amp * (0.4 + o.hardness);
    const ph = Math.random() * Math.PI * 2;
    for (let n = 0; n < len; n++) out[n] += a * Math.exp(-n / tau) * Math.sin(w * n + ph);
  }
  // 木盤：三個低頻共鳴模態，受敲擊後才慢慢振起來，給聲音厚度
  if (o.body > 0) {
    const rise = 0.0025 * sr;
    for (const [f, amp, dec] of [
      [rand(92, 104), 0.9, 0.07],
      [rand(175, 195), 0.7, 0.055],
      [rand(300, 335), 0.6, 0.042],
    ]) {
      const w = (2 * Math.PI * f) / sr;
      const tau = dec * sr;
      for (let n = 0; n < len; n++) out[n] += o.body * 0.5 * amp * (1 - Math.exp(-n / rise)) * Math.exp(-n / tau) * Math.sin(w * n);
    }
  }
  // 接觸：半餘弦脈衝，硬度越低越寬越柔（1.2–3 ms）
  const width = Math.ceil(sr * (0.003 - 0.0018 * o.hardness));
  for (let n = 0; n < Math.min(width, len); n++) out[n] -= 0.9 * Math.sin((Math.PI * n) / width);
  // 溫潤：兩次一階低通
  const cut = 1800 + 2600 * o.hardness;
  const k = 1 - Math.exp((-2 * Math.PI * cut) / sr);
  let y1 = 0, y2 = 0;
  for (let n = 0; n < len; n++) {
    y1 += k * (out[n] - y1);
    y2 += k * (y1 - y2);
    out[n] = y2;
  }
  // 尾端淡出並正規化
  const fade = Math.ceil(sr * 0.03);
  for (let n = len - fade; n < len; n++) out[n] *= (len - n) / fade;
  normalize(out, 0.9);
  return buf;
}

interface NoiseOpts {
  dur: number;
  lo: number;
  hi: number;
  /** 包絡峰值位置（0–1） */
  attack: number;
}

/** 摩擦 / 洗牌的噪音：帶通濾波的粉紅噪音，平滑包絡 */
export function filteredNoise(ctx: AudioContext, o: NoiseOpts): AudioBuffer {
  const sr = ctx.sampleRate;
  const len = Math.ceil(sr * o.dur);
  const buf = ctx.createBuffer(1, len, sr);
  const out = buf.getChannelData(0);
  // Paul Kellet 粉紅噪音
  let b0 = 0, b1 = 0, b2 = 0;
  // 一階高通 + 一階低通組成帶通
  const hp = Math.exp((-2 * Math.PI * o.lo) / sr);
  const lp = 1 - Math.exp((-2 * Math.PI * o.hi) / sr);
  let hpPrevIn = 0, hpOut = 0, lpOut = 0;
  for (let n = 0; n < len; n++) {
    const w = Math.random() * 2 - 1;
    b0 = 0.99765 * b0 + w * 0.099046;
    b1 = 0.963 * b1 + w * 0.2965164;
    b2 = 0.57 * b2 + w * 1.0526913;
    const pink = b0 + b1 + b2 + w * 0.1848;
    hpOut = hp * (hpOut + pink - hpPrevIn);
    hpPrevIn = pink;
    lpOut += lp * (hpOut - lpOut);
    const t = n / len;
    const env = t < o.attack ? Math.sin((t / o.attack) * (Math.PI / 2)) : Math.pow(1 - (t - o.attack) / (1 - o.attack), 1.6);
    // 滑動時的細微顆粒感
    const grain = 0.75 + 0.25 * Math.sin(n * 0.0021 + Math.sin(n * 0.00037) * 3);
    out[n] = lpOut * env * grain;
  }
  normalize(out, 0.8);
  return buf;
}

/** Karplus–Strong 撥弦：延遲線 + 平均濾波，一階全通濾波修正分數延遲使音準精確 */
export function pluck(ctx: AudioContext, f: number, bright: number): AudioBuffer {
  const sr = ctx.sampleRate;
  const dur = 3.2;
  const len = Math.ceil(sr * dur);
  const buf = ctx.createBuffer(1, len, sr);
  const out = buf.getChannelData(0);
  const period = sr / f - 0.5;
  const N = Math.floor(period);
  const frac = period - N;
  const c = (1 - frac) / (1 + frac);
  // 每個週期的衰減：讓 T60 約 2.6 秒（高音稍短）
  const t60 = 2.6 * Math.pow(293.66 / f, 0.35);
  const rho = Math.pow(10, -3 / (f * t60));
  // 激發：低通後的噪音，並以撥弦位置（約 1/7 處）做梳狀濾波，指甲撥弦的明亮度由 bright 控制
  const exc = new Float32Array(N + 1);
  let lpv = 0;
  for (let i = 0; i <= N; i++) {
    lpv += bright * (Math.random() * 2 - 1 - lpv);
    exc[i] = lpv;
  }
  const pos = Math.max(1, Math.round(N / 7));
  for (let i = N; i >= pos; i--) exc[i] -= exc[i - pos];
  let apx = 0, apy = 0;
  for (let n = 0; n < len; n++) {
    if (n <= N) {
      out[n] = exc[n];
      continue;
    }
    const a = out[n - N];
    const b = out[n - N - 1];
    const x = rho * 0.5 * (a + b);
    const y = c * x + apx - c * apy;
    apx = x;
    apy = y;
    out[n] = y;
  }
  // 琴體：輕微的低頻共鳴與柔和的起音
  const atk = Math.ceil(sr * 0.002);
  for (let n = 0; n < atk; n++) out[n] *= n / atk;
  // 尾端淡出
  const fade = Math.ceil(sr * 0.3);
  for (let n = len - fade; n < len; n++) out[n] *= (len - n) / fade;
  normalize(out, 0.7);
  return buf;
}

/** 房間殘響：早期反射 + 指數衰減、逐漸變暗的去相關噪音（立體聲） */
export function roomImpulse(ctx: AudioContext): AudioBuffer {
  const sr = ctx.sampleRate;
  const len = Math.ceil(sr * 1.8);
  const buf = ctx.createBuffer(2, len, sr);
  for (let ch = 0; ch < 2; ch++) {
    const d = buf.getChannelData(ch);
    let lp = 0;
    for (let n = 0; n < len; n++) {
      const t = n / sr;
      const env = Math.exp(-t / 0.38);
      // 越晚越暗：低通係數隨時間變小（整體偏暗，像木造的房間）
      const k = 0.26 * Math.exp(-t / 0.3) + 0.03;
      lp += k * (Math.random() * 2 - 1 - lp);
      d[n] = lp * env * (t < 0.008 ? t / 0.008 : 1);
    }
    for (const [t, g] of [
      [0.011, 0.5],
      [0.019, 0.35],
      [0.027, 0.3],
      [0.041, 0.22],
      [0.057, 0.16],
    ] as const) {
      const i = Math.floor((t + (ch ? 0.0023 : 0)) * sr);
      d[i] += ch ? -g : g;
    }
    normalize(d, 0.5);
  }
  return buf;
}

function normalize(a: Float32Array, peak: number) {
  let m = 0;
  for (let i = 0; i < a.length; i++) m = Math.max(m, Math.abs(a[i]));
  if (m > 0) {
    const k = peak / m;
    for (let i = 0; i < a.length; i++) a[i] *= k;
  }
}
