import "./style.css";
import { Engine } from "./engine";
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
}
const SETTINGS_KEY = "cdc-settings";
const settings: Settings = { mode: "human", first: "human", time: 2000, timeA: 2000, timeB: 2000, variety: 0.5, auto: false };
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

/** 座位名稱 */
const seatName = (seat: Seat) => (mode === "ai" ? (seat ? "電腦 B" : "電腦 A") : seat ? "電腦" : "你");

function seatColor(s: Snapshot, seat: Seat): number | null {
  if (!s.assigned || s.history.length === 0) return null;
  const first = colorOf(s.history[0].info);
  return seat === firstSeat ? first : 1 - first;
}

const humanColor = (s: Snapshot) => (mode === "human" ? seatColor(s, 0) : null);

/** 該座位的思考時間 */
const seatTime = (seat: Seat) => (mode === "ai" ? (seat ? settings.timeB : settings.timeA) : settings.time);

const decode = (m: number) => ({ from: m & 31, to: (m >> 5) & 31 });

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
  const myTurn = s.outcome === -1 && isHumanPly(s.ply) && !thinking;
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
  const human = mode === "human" && seat === 0;
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
  if (!s.history.length) log.innerHTML = `<li class="muted empty">尚未開始</li>`;
  const first = s.history.length ? colorOf(s.history[0].info) : 0;
  s.history.forEach((h: HistoryEntry, i) => {
    const mover = i % 2 === 0 ? first : 1 - first; // colours alternate from the first flip
    const li = document.createElement("li");
    li.innerHTML = `<span class="n">${i + 1}</span><span class="dot ${mover ? "black" : "red"}"></span><span class="who">${
      mode === "ai" ? (seatOfPly(i) ? "B" : "A") : isHumanPly(i) ? "你" : "電腦"
    }</span><span class="mv"></span>`;
    $(".mv", li).textContent = notate(h);
    log.append(li);
  });
  log.scrollTop = log.scrollHeight;
}

function renderAnalysis(s: Snapshot) {
  const root = $("#analysis");
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
  const ended = state?.outcome === -1 && next.outcome !== -1;
  state = next;
  if (ended && mode === "ai") {
    if (next.outcome === 2) score.draw++;
    else if (next.outcome === seatColor(next, 0)) score.a++;
    else score.b++;
  }
  render(animate);
  if (next.outcome === -1) return;
  if (mode === "ai" && settings.auto && ended) {
    const gen = game;
    setTimeout(() => gen === game && settings.auto && !paused && void newGame(), 2500);
  } else setTimeout(() => showOver(next), 550);
}

async function newGame() {
  engine.terminate(); // aborts any running search
  engine = new Engine();
  const gen = ++game;
  thinking = busy = paused = false;
  selected = null;
  analysis = null;
  error = "";
  if (overDialog.open) overDialog.close();
  mode = settings.mode;
  document.body.dataset.mode = mode;
  $("#subtitle").textContent = mode === "ai" ? "電腦對戰" : "與電腦對弈";
  firstSeat = settings.first === "random" ? (Math.random() < 0.5 ? 0 : 1) : settings.first === "human" ? 0 : 1;
  const seed = crypto.getRandomValues(new Uint32Array(1))[0];
  const { state: s } = await engine.call({ type: "new", seed });
  if (gen !== game) return;
  state = null; // 新局不計入上一局的終局
  apply(s, false);
  maybeAiMove();
}

async function maybeAiMove() {
  const s = state;
  if (!s || s.outcome !== -1 || isHumanPly(s.ply) || thinking || paused) return;
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
  if (!s || thinking || busy || s.outcome !== -1 || !isHumanPly(s.ply)) return;
  const moves = s.legal.map(decode);
  if (selected !== null && moves.some((m) => m.from === selected && m.to === i && m.to !== m.from))
    return void humanMove(selected, i);
  const p = s.cells[i];
  if (p === FACE_DOWN && moves.some((m) => m.from === i && m.to === i)) return void humanMove(i, i);
  const hc = humanColor(s);
  selected = p < FACE_DOWN && hc !== null && colorOf(p) === hc && selected !== i ? i : null;
  render();
}

/** Ply to rewind to: the latest earlier ply on which the human was to move. */
function undoTarget(s: Snapshot): number | null {
  for (let p = s.ply - 1; p >= 0; p--) if (isHumanPly(p)) return p;
  return null;
}

async function undo() {
  const s = state;
  if (!s || thinking || busy) return;
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
  if (overDialog.returnValue === "new") void newGame();
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && selected !== null) {
    selected = null;
    render();
  }
});

void newGame();
