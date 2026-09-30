// Types and helpers shared by the UI thread and the engine worker.

export const FACE_DOWN = 14;
export const EMPTY = 15;
export const PIECE_CHARS = "帥仕相俥傌炮兵將士象車馬包卒";
export const COLOR_NAMES = ["紅", "黑"] as const;

export interface Move {
  from: number;
  to: number;
  flip: boolean;
}
export interface HistoryEntry extends Move {
  info: number;
}

export interface Snapshot {
  cells: number[];
  side: number;
  assigned: boolean;
  nocap: number;
  drawPlies: number;
  ply: number;
  captured: number[];
  pool: number[];
  legal: number[];
  outcome: number; // -1 ongoing, 0 red, 1 black, 2 draw
  reason: string; // "", "annihilated", "blocked", "no_progress", "repetition"
  history: HistoryEntry[];
}

export interface Analysis {
  move: Move;
  score: number;
  depth: number;
  nodes: number;
  timeMs: number;
  pv: Move[];
  experts: { name: string; weight: number }[];
  candidates: { move: Move; score: number }[];
}

export type Request =
  | { type: "new"; seed: number }
  /** 以種子開局並依序走完 moves（編碼 from | to << 5）；用於回放與從回放接手 */
  | { type: "load"; seed: number; moves: number[] }
  | { type: "play"; from: number; to: number }
  | { type: "think"; slot: number; timeMs: number; variety: number }
  /** 只思考、不落子（給玩家的建議著法） */
  | { type: "suggest"; timeMs: number }
  | { type: "undo"; n: number };

export type Response =
  | { id: number; ok: true; state: Snapshot; analysis?: Analysis }
  | { id: number; ok: false; error: string };

export const colorOf = (p: number) => (p >= 7 ? 1 : 0);
export const square = (i: number) => "abcdefgh"[i % 8] + (Math.floor(i / 8) + 1);

/** Move-log notation: `c3 翻 俥`, `c3-c4`, `c3xc4 吃 馬`. */
export function notate(h: HistoryEntry): string {
  if (h.flip) return `${square(h.from)} 翻 ${PIECE_CHARS[h.info] ?? "?"}`;
  if (h.info < FACE_DOWN) return `${square(h.from)}x${square(h.to)} 吃 ${PIECE_CHARS[h.info]}`;
  return `${square(h.from)}-${square(h.to)}`;
}

/** Notation for an engine move, optionally using the board it was searched on. */
export function notateMove(m: Move, cells?: number[]): string {
  if (m.flip || m.from === m.to) return `${square(m.from)} 翻`;
  const target = cells?.[m.to];
  const cap = target !== undefined && target < FACE_DOWN;
  return `${square(m.from)}${cap ? "x" : "-"}${square(m.to)}${cap ? " 吃 " + PIECE_CHARS[target] : ""}`;
}

/** Win probability from a centipawn-ish score (~100 = one pawn, ±30000 = mate). */
export function winProb(score: number): number {
  if (score >= 19000) return 1;
  if (score <= -19000) return 0;
  return 1 / (1 + Math.exp(-score / 300));
}
