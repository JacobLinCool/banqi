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

type First = "human" | "ai" | "random";
interface Settings {
  first: First;
  time: number;
  variety: number;
}
const SETTINGS_KEY = "cdc-settings";
const settings: Settings = { first: "human", time: 2000, variety: 0.5 };
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

let engine = new Engine();
let game = 0; // generation counter; stale replies are dropped
let state: Snapshot | null = null;
let humanFirst = true;
let thinking = false;
let busy = false; // a human move is in flight
let selected: number | null = null;
let analysis: { data: Analysis; before: number[] } | null = null;
let error = "";

const $ = <T extends HTMLElement = HTMLElement>(sel: string, root: ParentNode = document) =>
  root.querySelector(sel) as T;

const boardEl = $("#board");
const barAi = $("#bar-ai");
const barMe = $("#bar-me");
const statusEl = $("#status");
const overDialog = $<HTMLDialogElement>("#over");

/** Plies at which the human is to move have this parity. */
const isHumanPly = (ply: number) => ply % 2 === (humanFirst ? 0 : 1);

function humanColor(s: Snapshot): number | null {
  if (!s.assigned || s.history.length === 0) return null;
  const first = colorOf(s.history[0].info);
  return humanFirst ? first : 1 - first;
}

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
  const aiColor = hc === null ? null : 1 - hc;
  setBar(barAi, aiColor, s, !isHumanPly(s.ply) && s.outcome === -1);
  setBar(barMe, hc, s, myTurn);
  $("[data-role=thinking]", barAi).hidden = !thinking;
  barAi.classList.toggle("is-thinking", thinking);

  // status line
  statusEl.textContent = error || statusText(s, hc);
  statusEl.classList.toggle("error", !!error);

  // side panels
  renderInfo(s);
  renderLog(s);
  renderAnalysis(hc);
  ($("#btn-undo") as HTMLButtonElement).disabled = thinking || undoTarget(s) === null;
}

function setBar(bar: HTMLElement, color: number | null, s: Snapshot, toMove: boolean) {
  const tag = $("[data-role=color]", bar);
  tag.textContent = color === null ? "未定" : `${bar === barMe ? "你是" : "執"}${COLOR_NAMES[color]}`;
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
      isHumanPly(i) ? "你" : "電腦"
    }</span><span class="mv"></span>`;
    $(".mv", li).textContent = notate(h);
    log.append(li);
  });
  log.scrollTop = log.scrollHeight;
}

function renderAnalysis(hc: number | null) {
  const root = $("#analysis");
  if (!analysis) {
    root.innerHTML = `<p class="muted">${thinking ? "電腦思考中…" : "電腦走棋後，這裡會顯示評估與搜尋資訊。"}</p>`;
    return;
  }
  const { data: a, before } = analysis;
  const p = winProb(a.score);
  const aiC = hc === null ? "neutral" : hc ? "red" : "black";
  const meC = hc === null ? "neutral" : hc ? "black" : "red";
  const abs = Math.abs(a.score);
  const mate = abs >= 19000;
  // 29000 以上為搜尋找到的殺棋；19000–21000 為殘局資料庫的完美解
  const scoreText = mate
    ? abs >= 29000
      ? a.score > 0 ? "電腦將勝" : "電腦將敗"
      : a.score > 0 ? "殘局庫：電腦必勝" : "殘局庫：電腦必敗"
    : (a.score > 0 ? "+" : "") + (a.score / 100).toFixed(2);
  const fmt = (n: number) => (n >= 1e6 ? (n / 1e6).toFixed(1) + "M" : n >= 1e3 ? (n / 1e3).toFixed(1) + "K" : String(n));
  const nps = a.timeMs > 0 ? fmt(Math.round((a.nodes / a.timeMs) * 1000)) : "—";
  const maxW = Math.max(...a.experts.map((e) => e.weight), 1e-9);

  root.innerHTML = `
    <div class="eval">
      <div class="eval-head"><span>電腦勝率 <b>${(p * 100).toFixed(0)}%</b></span><span class="muted">評分 ${scoreText}</span></div>
      <div class="eval-bar"><span class="${aiC}" style="width:${p * 100}%"></span><span class="${meC}"></span></div>
      <div class="eval-legend muted"><span>電腦</span><span>你</span></div>
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
  state = next;
  render(animate);
  if (next.outcome !== -1) setTimeout(() => showOver(next), 550);
}

async function newGame() {
  engine.terminate(); // aborts any running search
  engine = new Engine();
  const gen = ++game;
  thinking = busy = false;
  selected = null;
  analysis = null;
  error = "";
  if (overDialog.open) overDialog.close();
  humanFirst = settings.first === "random" ? Math.random() < 0.5 : settings.first === "human";
  const seed = crypto.getRandomValues(new Uint32Array(1))[0];
  const { state: s } = await engine.call({ type: "new", seed });
  if (gen !== game) return;
  apply(s, false);
  maybeAiMove();
}

async function maybeAiMove() {
  const s = state;
  if (!s || s.outcome !== -1 || isHumanPly(s.ply) || thinking) return;
  const gen = game;
  thinking = true;
  render();
  startProgress(settings.time);
  const started = performance.now();
  try {
    const before = s.cells;
    const reply = await engine.call({ type: "think", timeMs: settings.time, variety: settings.variety });
    const wait = 450 - (performance.now() - started); // let quick moves still feel deliberate
    if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    if (gen !== game) return;
    thinking = false;
    analysis = reply.analysis ? { data: reply.analysis, before } : null;
    apply(reply.state, true);
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

function showOver(s: Snapshot) {
  if (state !== s || overDialog.open) return;
  const hc = humanColor(s);
  const res = s.outcome === 2 ? "和" : s.outcome === hc ? "勝" : "負";
  const mark = $("#over-mark");
  mark.textContent = res;
  mark.className = `over-mark r-${res === "勝" ? "win" : res === "負" ? "lose" : "draw"}`;
  $("#over-title").textContent = res === "勝" ? "恭喜，你贏了！" : res === "負" ? "電腦獲勝" : "和局";
  $("#over-text").textContent =
    res === "和"
      ? s.reason === "repetition"
        ? `同一局面重複出現三次，依規則判和。共 ${s.ply} 步。`
        : `連續 ${s.drawPlies} 步沒有吃子或翻子，依規則判和。共 ${s.ply} 步。`
      : `${COLOR_NAMES[s.outcome]}方獲勝：${s.reason === "blocked" ? "對方已無子可動" : "對方的子全被吃光"}。共 ${s.ply} 步。`;
  overDialog.showModal();
}

// ───────────────────────── thinking progress ─────────────────────────

let progressAnim: Animation | null = null;
function startProgress(ms: number) {
  const bar = $("[data-role=progress] span", barAi);
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
bindRadios("first", settings.first, (v) => {
  settings.first = v as First;
  saveSettings();
});
bindRadios("time", String(settings.time), (v) => {
  settings.time = Number(v);
  saveSettings();
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
