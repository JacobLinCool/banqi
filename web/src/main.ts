import "./style.css";
import { Sound } from "./audio";
import { Engine } from "./engine";
import { decodeRecord, type GameRecord, recordUrl } from "./replay";
import {
  type Analysis,
  COLOR_NAMES,
  colorOf,
  EMPTY,
  FACE_DOWN,
  type HistoryEntry,
  notate,
  notateMove,
  PIECE_CHARS,
  type Snapshot,
  square,
  winProb,
} from "./shared";

// ───────────────────────── settings ─────────────────────────

type Mode = "human" | "ai";
/** 人機模式："human" 玩家先、"ai" 電腦先；電腦對戰模式："human" A 先、"ai" B 先 */
type First = "human" | "ai" | "random";
interface Settings {
  mode: Mode;
  first: First;
  time: number;
  timeA: number;
  timeB: number;
  variety: number;
  auto: boolean;
  sound: boolean;
  volume: number;
}
const SETTINGS_KEY = "cdc-settings";
const settings: Settings = {
  mode: "human",
  first: "human",
  time: 2000,
  timeA: 2000,
  timeB: 2000,
  variety: 0.5,
  auto: false,
  sound: true,
  volume: 0.8,
};
try {
  Object.assign(settings, JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "{}"));
} catch {
  /* storage unavailable */
}
const saveSettings = () => {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
  } catch {
    /* ignore */
  }
};

// ───────────────────────── state ─────────────────────────

// 座位：0 = 下方（人機模式的玩家／電腦對戰的 A），1 = 上方（電腦／B）
type Seat = 0 | 1;

let engine = new Engine();
let game = 0; // generation counter; stale replies are dropped
let state: Snapshot | null = null;
let mode: Mode = settings.mode; // 本局的模式（設定改變時立即開新局）
let firstSeat: Seat = 0;
let thinking = false;
let busy = false; // a human move is in flight
let paused = false; // 電腦對戰暫停
let selected: number | null = null;
let analysis: { data: Analysis; before: number[]; seat: Seat } | null = null;
let error = "";
const score = { a: 0, b: 0, draw: 0 }; // 電腦對戰戰績（以座位計）
let seed = 0; // 本局的發牌種子（分享用）

/** 回放中的棋局：full 是整局的著法紀錄，ply 是目前停在第幾步 */
interface Replay {
  record: GameRecord;
  full: HistoryEntry[];
  final: Snapshot;
  ply: number;
  playing: boolean;
  timer: number;
}
let replay: Replay | null = null;
const SPEEDS = [0.5, 1, 2, 4];
let speed = 1;

const sound = new Sound();
sound.enabled = settings.sound;
sound.volume = settings.volume;

const $ = <T extends HTMLElement = HTMLElement>(sel: string, root: ParentNode = document) =>
  root.querySelector(sel) as T;

const boardEl = $("#board");
const barAi = $("#bar-ai");
const barMe = $("#bar-me");
const bars = [barMe, barAi] as const;
const statusEl = $("#status");
const overDialog = $<HTMLDialogElement>("#over");

const seatOfPly = (ply: number) => ((ply + firstSeat) % 2) as Seat;
const isHumanPly = (ply: number) => mode === "human" && seatOfPly(ply) === 0;

/** 座位名稱（回放時人機對弈的玩家不稱「你」） */
const seatName = (seat: Seat) => (mode === "ai" ? (seat ? "電腦 B" : "電腦 A") : seat ? "電腦" : replay ? "玩家" : "你");

function seatColor(s: Snapshot, seat: Seat): number | null {
  if (!s.assigned || s.history.length === 0) return null;
  const first = colorOf(s.history[0].info);
  return seat === firstSeat ? first : 1 - first;
}

const humanColor = (s: Snapshot) => (mode === "human" ? seatColor(s, 0) : null);

/** 該座位的思考時間 */
const seatTime = (seat: Seat) => (mode === "ai" ? (seat ? settings.timeB : settings.timeA) : settings.time);

const decode = (m: number) => ({ from: m & 31, to: (m >> 5) & 31 });

/** 依欄位（a–h）決定左右聲像 */
const panOf = (sq: number) => (((sq % 8) - 3.5) / 3.5) * 0.7;

// ───────────────────────── rendering helpers ─────────────────────────

function pieceEl(p: number, extra = ""): HTMLElement {
  const el = document.createElement("span");
  if (p === FACE_DOWN) el.className = `piece back ${extra}`;
  else {
    el.className = `piece ${colorOf(p) ? "black" : "red"} ${extra}`;
    el.textContent = PIECE_CHARS[p];
  }
  return el;
}

const cellEls: HTMLButtonElement[] = [];
for (let row = 3; row >= 0; row--) {
  for (let col = 0; col < 8; col++) {
    const i = row * 8 + col;
    const b = document.createElement("button");
    b.className = "cell";
    b.type = "button";
    b.setAttribute("role", "gridcell");
    b.addEventListener("click", () => onCell(i));
    if (col === 0) b.dataset.rank = String(row + 1);
    if (row === 0) b.dataset.file = "abcdefgh"[col];
    cellEls[i] = b;
    boardEl.append(b);
  }
}

function describe(p: number) {
  if (p === FACE_DOWN) return "蓋棋";
  if (p === EMPTY) return "空";
  return COLOR_NAMES[colorOf(p)] + PIECE_CHARS[p];
}

// ───────────────────────── render ─────────────────────────

function render(animate = false) {
  const s = state;
  if (!s) return;
  const hc = humanColor(s);
  const myTurn = !replay && s.outcome === -1 && isHumanPly(s.ply) && !thinking;
  const moves = s.legal.map(decode);
  const targets = new Map<number, boolean>(); // to → isCapture
  if (selected !== null && myTurn)
    for (const m of moves) if (m.from === selected && m.to !== m.from) targets.set(m.to, s.cells[m.to] < FACE_DOWN);

  const last = s.history.at(-1);
  const lastByAi = last !== undefined && !isHumanPly(s.ply - 1);
  const toMove = seatOfPly(s.ply);

  for (let i = 0; i < 32; i++) {
    const b = cellEls[i];
    const p = s.cells[i];
    b.replaceChildren();
    if (p !== EMPTY) b.append(pieceEl(p, i === selected ? "selected" : ""));
    if (targets.get(i) === false) b.append(Object.assign(document.createElement("span"), { className: "hint" }));
    const actionable =
      myTurn && (targets.has(i) || moves.some((m) => m.from === i) || (hc !== null && p < FACE_DOWN && colorOf(p) === hc));
    b.classList.toggle("actionable", actionable);
    b.classList.toggle("dest", targets.get(i) === false);
    b.classList.toggle("capture", targets.get(i) === true);
    b.classList.toggle("last-from", !!last && !last.flip && last.from === i);
    b.classList.toggle("last-to", !!last && last.to === i);
    b.classList.toggle("by-ai", lastByAi);
    b.setAttribute("aria-label", `${square(i)} ${describe(p)}`);
  }
  if (animate && last) animateMove(last);

  // player bars
  for (const seat of [0, 1] as Seat[]) {
    const bar = bars[seat];
    const active = s.outcome === -1 && toMove === seat;
    setBar(bar, seat, seatColor(s, seat), s, seat === 0 && mode === "human" ? myTurn : active && !(paused && !thinking));
    $("[data-role=thinking]", bar).hidden = !(thinking && active);
    bar.classList.toggle("is-thinking", thinking && active);
  }

  // status line
  statusEl.textContent = error || statusText(s, hc);
  statusEl.classList.toggle("error", !!error);

  // side panels
  renderInfo(s);
  renderLog(s);
  renderAnalysis(s);
  renderControls(s);
  renderReplay(s);
}

function renderControls(s: Snapshot) {
  ($("#btn-undo") as HTMLButtonElement).disabled = thinking || undoTarget(s) === null;
  const pause = $<HTMLButtonElement>("#btn-pause");
  pause.textContent = paused ? "繼續" : "暫停";
  pause.disabled = s.outcome !== -1;
  const total = score.a + score.b + score.draw;
  // A 的得分率（和棋算半分）
  const rate = total ? `${(((score.a + score.draw / 2) / total) * 100).toFixed(0)}%` : "—";
  $("#score").innerHTML = `
    <div><dt>A 勝</dt><dd>${score.a}</dd></div>
    <div><dt>B 勝</dt><dd>${score.b}</dd></div>
    <div><dt>和</dt><dd>${score.draw}</dd></div>
    <div><dt>A 得分率</dt><dd>${rate}</dd></div>`;
}

function setBar(bar: HTMLElement, seat: Seat, color: number | null, s: Snapshot, toMove: boolean) {
  const human = mode === "human" && seat === 0 && !replay;
  const avatar = $("[data-role=avatar]", bar);
  avatar.textContent = mode === "ai" ? (seat ? "B" : "A") : seat ? "機" : "人";
  avatar.classList.toggle("me", seat === 0);
  $("[data-role=name]", bar).textContent = seatName(seat);
  const tag = $("[data-role=color]", bar);
  tag.textContent = color === null ? "未定" : `${human ? "你是" : "執"}${COLOR_NAMES[color]}`;
  tag.className = `tag ${color === null ? "" : color ? "black" : "red"}`;
  bar.classList.toggle("to-move", toMove);
  const caps = $("[data-role=caps]", bar);
  caps.replaceChildren();
  if (color === null) return;
  // pieces this player has taken = captured pieces of the opposite colour
  const base = (1 - color) * 7;
  for (let t = 0; t < 7; t++) for (let k = 0; k < s.captured[base + t]; k++) caps.append(pieceEl(base + t, "mini"));
}

function statusText(s: Snapshot, hc: number | null): string {
  if (replay) {
    if (s.outcome !== -1) return s.outcome === 2 ? "和局" : `${winnerLabel(s)}獲勝`;
    const last = s.history.at(-1);
    if (!last) return "開局：按播放或 → 開始回放";
    return `第 ${s.ply} 步　${seatName(seatOfPly(s.ply - 1))}　${notate(last)}`;
  }
  if (mode === "ai") {
    if (s.outcome !== -1) return s.outcome === 2 ? "和局" : `${winnerLabel(s)}獲勝`;
    const seat = seatOfPly(s.ply);
    const c = seatColor(s, seat);
    const who = `${seatName(seat)}${c === null ? " " : `（${COLOR_NAMES[c]}）`}`;
    if (thinking) return `${who}思考中…${paused ? "（這步走完後暫停）" : ""}`;
    return paused ? `已暫停，輪到${who}` : `輪到${who}`;
  }
  if (s.outcome !== -1) return s.outcome === 2 ? "和局" : s.outcome === hc ? "你贏了！" : "電腦獲勝";
  if (thinking) return "電腦思考中…";
  if (!isHumanPly(s.ply)) return "輪到電腦";
  if (hc === null) return "輪到你：翻開任一棋子，翻出的顏色就是你的";
  return selected !== null ? "選擇要走到的位置" : `輪到你（${COLOR_NAMES[hc]}）`;
}

function renderInfo(s: Snapshot) {
  const pct = Math.min(100, (s.nocap / Math.max(1, s.drawPlies)) * 100);
  const bar = $("#nocap-bar");
  bar.style.width = `${pct}%`;
  bar.classList.toggle("warn", pct >= 75);
  $("#nocap-text").textContent = `${s.nocap} / ${s.drawPlies} 步（達上限判和）`;

  const pool = $("#pool");
  pool.replaceChildren();
  for (let p = 0; p < 14; p++) {
    const item = document.createElement("span");
    item.className = "pool-item" + (s.pool[p] ? "" : " empty");
    item.append(pieceEl(p, "mini"), Object.assign(document.createElement("b"), { textContent: `×${s.pool[p]}` }));
    pool.append(item);
  }
}

function renderLog(s: Snapshot) {
  const log = $("#log");
  log.replaceChildren();
  // 回放時列出整局，目前這步高亮、之後的淡化
  const moves = replay ? replay.full : s.history;
  if (!moves.length) log.innerHTML = `<li class="muted empty">尚未開始</li>`;
  const first = moves.length ? colorOf(moves[0].info) : 0;
  let current: HTMLElement | null = null;
  moves.forEach((h: HistoryEntry, i) => {
    const mover = i % 2 === 0 ? first : 1 - first; // colours alternate from the first flip
    const li = document.createElement("li");
    li.innerHTML = `<span class="n">${i + 1}</span><span class="dot ${mover ? "black" : "red"}"></span><span class="who">${
      mode === "ai" ? (seatOfPly(i) ? "B" : "A") : isHumanPly(i) ? (replay ? "玩家" : "你") : "電腦"
    }</span><span class="mv"></span>`;
    $(".mv", li).textContent = notate(h);
    if (replay) {
      if (i === s.ply - 1) current = li;
      li.classList.toggle("current", i === s.ply - 1);
      li.classList.toggle("future", i >= s.ply);
      li.addEventListener("click", () => {
        setPlaying(false);
        void seek(i + 1);
      });
    }
    log.append(li);
  });
  if (!replay) log.scrollTop = log.scrollHeight;
  else if (current) {
    const top = (current as HTMLElement).getBoundingClientRect().top - log.getBoundingClientRect().top + log.scrollTop;
    log.scrollTop = top - log.clientHeight / 2;
  } else log.scrollTop = 0;
}

function renderAnalysis(s: Snapshot) {
  const root = $("#analysis");
  if (replay) {
    root.innerHTML = `<p class="muted">回放中不顯示電腦分析；可以從任一步「接手對弈」，讓電腦繼續思考。</p>`;
    return;
  }
  if (!analysis) {
    root.innerHTML = `<p class="muted">${thinking ? "電腦思考中…" : "電腦走棋後，這裡會顯示評估與搜尋資訊。"}</p>`;
    return;
  }
  const { data: a, before, seat } = analysis;
  // 評分以走這步的電腦為視角
  const me = seatName(seat);
  const sp = mode === "ai" ? " " : ""; // 「電腦 A」後接中文時補空格
  const opp = seatName((1 - seat) as Seat);
  const c = seatColor(s, seat);
  const p = winProb(a.score);
  const aiC = c === null ? "neutral" : c ? "black" : "red";
  const oppC = c === null ? "neutral" : c ? "red" : "black";
  const abs = Math.abs(a.score);
  const mate = abs >= 19000;
  // 29000 以上為搜尋找到的殺棋；19000–21000 為殘局資料庫的完美解
  const scoreText = mate
    ? abs >= 29000
      ? a.score > 0 ? `${me}${sp}將勝` : `${me}${sp}將敗`
      : a.score > 0 ? `殘局庫：${me}${sp}必勝` : `殘局庫：${me}${sp}必敗`
    : (a.score > 0 ? "+" : "") + (a.score / 100).toFixed(2);
  const fmt = (n: number) => (n >= 1e6 ? (n / 1e6).toFixed(1) + "M" : n >= 1e3 ? (n / 1e3).toFixed(1) + "K" : String(n));
  const nps = a.timeMs > 0 ? fmt(Math.round((a.nodes / a.timeMs) * 1000)) : "—";
  const maxW = Math.max(...a.experts.map((e) => e.weight), 1e-9);

  root.innerHTML = `
    <div class="eval">
      <div class="eval-head"><span>${me}${sp}勝率 <b>${(p * 100).toFixed(0)}%</b></span><span class="muted">評分 ${scoreText}</span></div>
      <div class="eval-bar"><span class="${aiC}" style="width:${p * 100}%"></span><span class="${oppC}"></span></div>
      <div class="eval-legend muted"><span>${me}</span><span>${opp}</span></div>
    </div>
    <dl class="stats">
      <div><dt>深度</dt><dd>${a.depth}</dd></div>
      <div><dt>節點</dt><dd>${fmt(a.nodes)}</dd></div>
      <div><dt>時間</dt><dd>${(a.timeMs / 1000).toFixed(2)}s</dd></div>
      <div><dt>速度</dt><dd>${nps}/s</dd></div>
    </dl>
    <h3>主要變化</h3>
    <div class="pv"></div>
    <h3>候選著法</h3>
    <table class="cands"><tbody></tbody></table>
    <h3>專家權重 <small class="muted">（MoE 閘控）</small></h3>
    <div class="experts"></div>`;

  const pv = $(".pv", root);
  a.pv.forEach((m, i) => pv.append(Object.assign(document.createElement("span"), { textContent: notateMove(m, i === 0 ? before : undefined) })));
  if (!a.pv.length) pv.innerHTML = `<span class="muted">—</span>`;

  const tbody = $(".cands tbody", root);
  const cands = a.candidates.length ? a.candidates : [{ move: a.move, score: a.score }];
  for (const c of cands) {
    const tr = document.createElement("tr");
    const chosen = c.move.from === a.move.from && c.move.to === a.move.to;
    tr.className = chosen ? "chosen" : "";
    tr.innerHTML = `<td></td><td class="num"></td><td class="num muted"></td>`;
    tr.cells[0].textContent = notateMove(c.move, before) + (chosen ? " ✓" : "");
    tr.cells[1].textContent = (c.score > 0 ? "+" : "") + c.score;
    tr.cells[2].textContent = `${(winProb(c.score) * 100).toFixed(0)}%`;
    tbody.append(tr);
  }

  const ex = $(".experts", root);
  for (const e of a.experts) {
    const row = document.createElement("div");
    row.className = "expert";
    row.innerHTML = `<span class="name"></span><span class="track"><span style="width:${(e.weight / maxW) * 100}%"></span></span><span class="num">${(
      e.weight * 100
    ).toFixed(1)}%</span>`;
    $(".name", row).textContent = e.name;
    ex.append(row);
  }
}

// ───────────────────────── animation ─────────────────────────

function animateMove(h: HistoryEntry) {
  const toEl = cellEls[h.to];
  const piece = toEl.querySelector(".piece") as HTMLElement | null;
  if (!piece || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  if (h.flip) {
    piece.animate(
      [
        { transform: "perspective(400px) rotateY(90deg) scale(1.15)", filter: "brightness(1.4)" },
        { transform: "perspective(400px) rotateY(0) scale(1)", filter: "none" },
      ],
      { duration: 320, easing: "cubic-bezier(.2,.8,.3,1.2)" },
    );
    return;
  }
  const a = cellEls[h.from].getBoundingClientRect();
  const b = toEl.getBoundingClientRect();
  if (h.info < FACE_DOWN) {
    const ghost = pieceEl(h.info, "ghost");
    toEl.prepend(ghost);
    ghost.animate([{ opacity: 1, transform: "scale(1)" }, { opacity: 0, transform: "scale(1.35)" }], {
      duration: 380,
      delay: 140,
      easing: "ease-out",
      fill: "forwards",
    }).onfinish = () => ghost.remove();
  }
  piece.animate(
    [
      { transform: `translate(${a.left - b.left}px, ${a.top - b.top}px) scale(1.08)`, zIndex: 3 },
      { transform: "translate(0,0) scale(1)", zIndex: 3 },
    ],
    { duration: 240, easing: "cubic-bezier(.3,.7,.3,1)" },
  );
}

// ───────────────────────── game flow ─────────────────────────

function apply(next: Snapshot, animate: boolean) {
  const prev = state;
  const ended = prev?.outcome === -1 && next.outcome !== -1;
  state = next;
  if (ended && mode === "ai" && !replay) {
    if (next.outcome === 2) score.draw++;
    else if (next.outcome === seatColor(next, 0)) score.a++;
    else score.b++;
  }
  render(animate);
  if (animate && prev && next.ply === prev.ply + 1) moveSound(next);
  if (next.outcome === -1) return;
  if (ended) endSound(next);
  if (replay) return;
  if (mode === "ai" && settings.auto && ended) {
    const gen = game;
    setTimeout(() => gen === game && settings.auto && !paused && void newGame(), 2500);
  } else setTimeout(() => showOver(next), 550);
}

function moveSound(s: Snapshot) {
  const last = s.history.at(-1);
  if (!last) return;
  const pan = panOf(last.to);
  if (last.flip) sound.flip(pan);
  else if (last.info < FACE_DOWN) sound.capture(pan);
  else sound.move(pan);
}

function endSound(s: Snapshot) {
  if (s.outcome === 2) sound.end("draw");
  else if (mode === "human" && !replay) sound.end(s.outcome === humanColor(s) ? "win" : "lose");
  else sound.end("win");
}

/** 結束目前的對局或回放、重設引擎；回傳新的世代編號 */
function reset(): number {
  engine.terminate(); // aborts any running search
  engine = new Engine();
  stopReplay();
  thinking = busy = paused = false;
  selected = null;
  analysis = null;
  error = "";
  stopProgress();
  if (overDialog.open) overDialog.close();
  return ++game;
}

/** 開新局；from 用於從回放的某一步接手（人機對弈，玩家執輪到走的一方） */
async function newGame(from?: { seed: number; moves: number[] }) {
  const gen = reset();
  if (location.search) history.replaceState(null, "", location.pathname);
  mode = from ? "human" : settings.mode;
  document.body.dataset.mode = mode;
  $("#subtitle").textContent = mode === "ai" ? "電腦對戰" : "與電腦對弈";
  firstSeat = from
    ? ((from.moves.length % 2) as Seat)
    : settings.first === "random"
      ? Math.random() < 0.5
        ? 0
        : 1
      : settings.first === "human"
        ? 0
        : 1;
  seed = from ? from.seed : crypto.getRandomValues(new Uint32Array(1))[0];
  const { state: s } = await engine.call(from ? { type: "load", seed, moves: from.moves } : { type: "new", seed });
  if (gen !== game) return;
  state = null; // 新局不計入上一局的終局
  apply(s, false);
  if (!from) sound.shuffle();
  maybeAiMove();
}

// ───────────────────────── replay ─────────────────────────

function stopReplay() {
  if (replay) clearTimeout(replay.timer);
  replay = null;
}

/** 載入回放；著法不合法（連結損毀）時改開新局 */
async function startReplay(record: GameRecord, autoplay: boolean) {
  const gen = reset();
  mode = record.mode;
  firstSeat = record.firstSeat;
  seed = record.seed;
  document.body.dataset.mode = "replay";
  $("#subtitle").textContent = "棋局回放";
  let final: Snapshot;
  try {
    final = (await engine.call({ type: "load", seed, moves: record.moves })).state;
  } catch (e) {
    if (gen !== game) return;
    toast(`回放連結無效：${(e as Error).message}`);
    return void newGame();
  }
  if (gen !== game) return;
  replay = { record, full: final.history, final, ply: 0, playing: false, timer: 0 };
  history.replaceState(null, "", recordUrl(record));
  state = null;
  await seek(0);
  if (autoplay && record.moves.length) replay.timer = window.setTimeout(() => setPlaying(true), 900);
}

let seekSeq = 0;
/** 跳到第 k 步（前進一步時有動畫與音效） */
async function seek(k: number) {
  const r = replay;
  if (!r) return;
  k = Math.max(0, Math.min(r.record.moves.length, k));
  const step = state !== null && k === r.ply + 1;
  r.ply = k;
  const seq = ++seekSeq;
  const gen = game;
  const { state: s } = await engine.call({ type: "load", seed: r.record.seed, moves: r.record.moves.slice(0, k) });
  if (gen !== game || seq !== seekSeq || replay !== r) return;
  apply(s, step);
}

function setPlaying(on: boolean) {
  const r = replay;
  if (!r) return;
  clearTimeout(r.timer);
  r.playing = on && r.record.moves.length > 0;
  if (r.playing) {
    if (r.ply >= r.record.moves.length) void seek(0);
    const tick = () => {
      if (!replay?.playing || replay !== r) return;
      if (r.ply >= r.record.moves.length) return setPlaying(false);
      void seek(r.ply + 1);
      r.timer = window.setTimeout(tick, 900 / speed);
    };
    r.timer = window.setTimeout(tick, r.ply === 0 ? 400 : 900 / speed);
  }
  if (state) render();
}

function renderReplay(s: Snapshot) {
  const r = replay;
  if (!r) return;
  const n = r.record.moves.length;
  const seekEl = $<HTMLInputElement>("#rp-seek");
  seekEl.max = String(n);
  seekEl.value = String(r.ply);
  ($("#rp-first") as HTMLButtonElement).disabled = ($("#rp-prev") as HTMLButtonElement).disabled = r.ply === 0;
  ($("#rp-last") as HTMLButtonElement).disabled = ($("#rp-next") as HTMLButtonElement).disabled = r.ply >= n;
  const play = $<HTMLButtonElement>("#rp-play");
  play.classList.toggle("playing", r.playing);
  play.setAttribute("aria-label", r.playing ? "暫停" : "播放");
  play.disabled = n === 0;
  $("#rp-speed").textContent = `${speed}×`;
  ($("#rp-takeover") as HTMLButtonElement).disabled = s.outcome !== -1;
  const t = (ms: number) => `${ms / 1000}s`;
  const who = r.record.mode === "ai" ? `電腦對戰（A ${t(r.record.timeA)}・B ${t(r.record.timeB)}）` : `人機對弈（電腦 ${t(r.record.timeA)}）`;
  const o = r.final.outcome;
  const result = o === -1 ? "未下完" : o === 2 ? "和局" : `${COLOR_NAMES[o]}方勝`;
  $("#replay-meta").textContent = `${who}・共 ${n} 步・${result}`;
}

// ───────────────────────── share ─────────────────────────

function currentRecord(): GameRecord | null {
  if (replay) return replay.record;
  if (!state || !state.history.length) return null;
  return {
    mode,
    firstSeat,
    timeA: mode === "ai" ? settings.timeA : settings.time,
    timeB: settings.timeB,
    seed,
    moves: state.history.map((h) => h.from | (h.to << 5)),
  };
}

async function share() {
  const r = currentRecord();
  if (!r) return toast("還沒有任何著法可以分享");
  const url = recordUrl(r);
  // 手機用系統分享面板，桌機直接複製
  if (navigator.share && matchMedia("(pointer: coarse)").matches) {
    try {
      await navigator.share({ title: "台灣暗棋棋局", text: `一盤 ${r.moves.length} 步的暗棋`, url });
      return;
    } catch (e) {
      if ((e as Error).name === "AbortError") return;
    }
  }
  try {
    await navigator.clipboard.writeText(url);
    toast("已複製回放連結");
  } catch {
    prompt("複製這個回放連結：", url);
  }
}

let toastTimer = 0;
function toast(msg: string) {
  const el = $("#toast");
  el.textContent = msg;
  el.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => el.classList.remove("show"), 2600);
}

async function maybeAiMove() {
  const s = state;
  if (!s || replay || s.outcome !== -1 || isHumanPly(s.ply) || thinking || paused) return;
  const gen = game;
  const seat = seatOfPly(s.ply);
  const time = seatTime(seat);
  thinking = true;
  render();
  startProgress(seat, time);
  const started = performance.now();
  try {
    const before = s.cells;
    // 人機模式只用一顆大腦；電腦對戰時 A、B 各用一顆
    const slot = mode === "ai" ? seat : 0;
    const reply = await engine.call({ type: "think", slot, timeMs: time, variety: settings.variety });
    const wait = 450 - (performance.now() - started); // let quick moves still feel deliberate
    if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    if (gen !== game) return;
    thinking = false;
    analysis = reply.analysis ? { data: reply.analysis, before, seat } : null;
    apply(reply.state, true);
    if (mode === "ai") maybeAiMove();
  } catch (e) {
    if (gen !== game) return;
    thinking = false;
    error = `引擎錯誤：${(e as Error).message}`;
    render();
  } finally {
    if (gen === game) stopProgress();
  }
}

async function humanMove(from: number, to: number) {
  if (busy) return;
  busy = true;
  const gen = game;
  selected = null;
  try {
    const { state: s } = await engine.call({ type: "play", from, to });
    if (gen !== game) return;
    error = "";
    apply(s, true);
    maybeAiMove();
  } catch (e) {
    if (gen === game) {
      error = (e as Error).message;
      render();
    }
  } finally {
    busy = false;
  }
}

function onCell(i: number) {
  const s = state;
  if (!s || replay || thinking || busy || s.outcome !== -1 || !isHumanPly(s.ply)) return;
  const moves = s.legal.map(decode);
  if (selected !== null && moves.some((m) => m.from === selected && m.to === i && m.to !== m.from))
    return void humanMove(selected, i);
  const p = s.cells[i];
  if (p === FACE_DOWN && moves.some((m) => m.from === i && m.to === i)) return void humanMove(i, i);
  const hc = humanColor(s);
  const own = p < FACE_DOWN && hc !== null && colorOf(p) === hc;
  if (own && selected !== i) sound.select(panOf(i));
  // 點了走不到的格子，或沒選子時點對方的子
  else if (!own && (selected !== null || p < FACE_DOWN)) sound.deny(panOf(i));
  selected = own && selected !== i ? i : null;
  render();
}

/** Ply to rewind to: the latest earlier ply on which the human was to move. */
function undoTarget(s: Snapshot): number | null {
  for (let p = s.ply - 1; p >= 0; p--) if (isHumanPly(p)) return p;
  return null;
}

async function undo() {
  const s = state;
  if (!s || replay || thinking || busy) return;
  const target = undoTarget(s);
  if (target === null) return;
  const gen = game;
  const { state: next } = await engine.call({ type: "undo", n: s.ply - target });
  if (gen !== game) return;
  selected = null;
  analysis = null;
  error = "";
  if (overDialog.open) overDialog.close();
  apply(next, false);
}

/** 電腦對戰的勝方，例如「電腦 A（紅）」 */
function winnerLabel(s: Snapshot) {
  const seat: Seat = s.outcome === seatColor(s, 0) ? 0 : 1;
  return `${seatName(seat)}（${COLOR_NAMES[s.outcome]}）`;
}

function showOver(s: Snapshot) {
  if (state !== s || overDialog.open) return;
  const mark = $("#over-mark");
  if (mode === "ai") {
    // 勝字以勝方顏色呈現
    mark.textContent = s.outcome === 2 ? "和" : "勝";
    mark.className = `over-mark r-${s.outcome === 2 ? "draw" : s.outcome === 0 ? "win" : "lose"}`;
    $("#over-title").textContent = s.outcome === 2 ? "和局" : `${winnerLabel(s)}獲勝`;
  } else {
    const hc = humanColor(s);
    const res = s.outcome === 2 ? "和" : s.outcome === hc ? "勝" : "負";
    mark.textContent = res;
    mark.className = `over-mark r-${res === "勝" ? "win" : res === "負" ? "lose" : "draw"}`;
    $("#over-title").textContent = res === "勝" ? "恭喜，你贏了！" : res === "負" ? "電腦獲勝" : "和局";
  }
  $("#over-text").textContent =
    s.outcome === 2
      ? s.reason === "repetition"
        ? `同一局面重複出現三次，依規則判和。共 ${s.ply} 步。`
        : `連續 ${s.drawPlies} 步沒有吃子或翻子，依規則判和。共 ${s.ply} 步。`
      : `${COLOR_NAMES[s.outcome]}方獲勝：${s.reason === "blocked" ? "對方已無子可動" : "對方的子全被吃光"}。共 ${s.ply} 步。`;
  overDialog.showModal();
}

// ───────────────────────── thinking progress ─────────────────────────

let progressAnim: Animation | null = null;
function startProgress(seat: Seat, ms: number) {
  const bar = $("[data-role=progress] span", bars[seat]);
  progressAnim?.cancel();
  progressAnim = bar.animate([{ transform: "scaleX(0)" }, { transform: "scaleX(1)" }], { duration: ms, easing: "linear", fill: "forwards" });
}
function stopProgress() {
  progressAnim?.cancel();
  progressAnim = null;
}

// ───────────────────────── controls ─────────────────────────

function bindRadios(name: string, value: string, onChange: (v: string) => void) {
  for (const input of document.querySelectorAll<HTMLInputElement>(`input[name=${name}]`)) {
    input.checked = input.value === value;
    input.addEventListener("change", () => input.checked && onChange(input.value));
  }
}
const relabelFirst = () => {
  for (const span of document.querySelectorAll<HTMLElement>("#opt-first span"))
    span.textContent = span.dataset[settings.mode === "ai" ? "labelAi" : "labelHuman"] ?? span.textContent;
};
relabelFirst();
bindRadios("mode", settings.mode, (v) => {
  settings.mode = v as Mode;
  saveSettings();
  relabelFirst();
  void newGame();
});
bindRadios("first", settings.first, (v) => {
  settings.first = v as First;
  saveSettings();
});
for (const key of ["time", "timeA", "timeB"] as const)
  bindRadios(key, String(settings[key]), (v) => {
    settings[key] = Number(v);
    saveSettings();
  });
const auto = $<HTMLInputElement>("#opt-auto");
auto.checked = settings.auto;
auto.addEventListener("change", () => {
  settings.auto = auto.checked;
  saveSettings();
  // 已終局時勾選就直接開下一局
  if (settings.auto && mode === "ai" && state && state.outcome !== -1 && !paused) void newGame();
});
const variety = $<HTMLInputElement>("#opt-variety");
const varietyOut = $("#variety-out");
const showVariety = () => {
  const v = settings.variety;
  varietyOut.textContent = v < 0.35 ? "穩健" : v < 1.2 ? "適中" : "多變";
};
variety.value = String(settings.variety);
showVariety();
variety.addEventListener("input", () => {
  settings.variety = Number(variety.value);
  showVariety();
  saveSettings();
});

$("#btn-new").addEventListener("click", () => void newGame());
const HINT_KEY = "cdc-rotate-hint-dismissed";
try {
  if (localStorage.getItem(HINT_KEY)) document.body.classList.add("hint-dismissed");
} catch {
  /* storage unavailable */
}
$("#rotate-dismiss").addEventListener("click", () => {
  document.body.classList.add("hint-dismissed");
  try {
    localStorage.setItem(HINT_KEY, "1");
  } catch {
    /* ignore */
  }
});
$("#btn-undo").addEventListener("click", () => void undo());
$("#btn-pause").addEventListener("click", () => {
  paused = !paused;
  render();
  maybeAiMove();
});
$("#btn-reset-score").addEventListener("click", () => {
  score.a = score.b = score.draw = 0;
  render();
});
overDialog.addEventListener("close", () => {
  const v = overDialog.returnValue;
  if (v === "new") void newGame();
  else if (v === "share") void share();
  else if (v === "replay") {
    const r = currentRecord();
    if (r) void startReplay(r, true);
  }
});
$("#btn-share").addEventListener("click", () => void share());

// 回放控制
const step = (k: number) => {
  setPlaying(false);
  void seek(k);
};
$("#rp-first").addEventListener("click", () => step(0));
$("#rp-prev").addEventListener("click", () => replay && step(replay.ply - 1));
$("#rp-next").addEventListener("click", () => replay && step(replay.ply + 1));
$("#rp-last").addEventListener("click", () => replay && step(replay.record.moves.length));
$("#rp-play").addEventListener("click", () => replay && setPlaying(!replay.playing));
$<HTMLInputElement>("#rp-seek").addEventListener("input", (e) => step(Number((e.target as HTMLInputElement).value)));
$("#rp-speed").addEventListener("click", () => {
  speed = SPEEDS[(SPEEDS.indexOf(speed) + 1) % SPEEDS.length];
  if (replay?.playing) setPlaying(true);
  else if (state) render();
});
$("#rp-exit").addEventListener("click", () => void newGame());
$("#rp-takeover").addEventListener("click", () => {
  if (!replay || !state || state.outcome !== -1) return;
  void newGame({ seed: replay.record.seed, moves: replay.record.moves.slice(0, replay.ply) });
});

// 音效設定
const soundOn = $<HTMLInputElement>("#opt-sound");
const volume = $<HTMLInputElement>("#opt-volume");
soundOn.checked = settings.sound;
volume.value = String(settings.volume);
volume.disabled = !settings.sound;
soundOn.addEventListener("change", () => {
  settings.sound = sound.enabled = soundOn.checked;
  volume.disabled = !soundOn.checked;
  saveSettings();
  sound.select();
});
volume.addEventListener("input", () => {
  settings.volume = sound.volume = Number(volume.value);
  saveSettings();
});
volume.addEventListener("change", () => sound.move());

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && selected !== null) {
    selected = null;
    render();
  }
  if (!replay) return;
  const tag = (e.target as HTMLElement).tagName;
  if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
  if (e.key === "ArrowRight") step(replay.ply + 1);
  else if (e.key === "ArrowLeft") step(replay.ply - 1);
  else if (e.key === "Home") step(0);
  else if (e.key === "End") step(replay.record.moves.length);
  else if (e.key === " " && tag !== "BUTTON") setPlaying(!replay.playing);
  else return;
  e.preventDefault();
});

// 網址帶有回放（?r=）時直接進入回放
const code = new URLSearchParams(location.search).get("r");
const shared = code ? decodeRecord(code) : null;
if (shared) void startReplay(shared, true);
else {
  if (code) toast("回放連結無效");
  void newGame();
}

// PWA：離線快取（file:// 與不支援的瀏覽器略過）
if ("serviceWorker" in navigator && location.protocol !== "file:") {
  addEventListener("load", () => {
    navigator.serviceWorker.register("./sw.js").catch(() => {
      /* 註冊失敗只是少了離線支援 */
    });
  });
}
