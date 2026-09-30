// 回放連結：一局棋由「發牌種子 + 著法序列」完整決定，編進 `?r=` 讓別人打開連結就能觀看。
//
// 二進位格式（base64url，不補 =）：
//   byte 0      版本（1）
//   byte 1      bit0 模式（0 人機、1 電腦對戰）、bit1 先手座位、bit2–3 A（或人機的電腦）思考時間、bit4–5 B 思考時間
//   byte 2–5    種子（u32 little-endian）
//   其後        每步 10 bits：from（5）| to（5），翻子 from == to；位元組尾端補零（不足 10 bits，不會多解出一步）

export const THINK_TIMES = [500, 2000, 5000, 10000] as const;

export interface GameRecord {
  mode: "human" | "ai";
  firstSeat: 0 | 1;
  /** 人機模式為電腦的思考時間；電腦對戰為 A 的 */
  timeA: number;
  timeB: number;
  seed: number;
  /** 著法，編碼 from | to << 5（與引擎 legal_moves 相同） */
  moves: number[];
}

const VERSION = 1;

const timeIndex = (ms: number) => {
  const i = THINK_TIMES.indexOf(ms as (typeof THINK_TIMES)[number]);
  return i < 0 ? 1 : i;
};

export function encodeRecord(r: GameRecord): string {
  const head = [
    VERSION,
    (r.mode === "ai" ? 1 : 0) | (r.firstSeat << 1) | (timeIndex(r.timeA) << 2) | (timeIndex(r.timeB) << 4),
    r.seed & 255,
    (r.seed >>> 8) & 255,
    (r.seed >>> 16) & 255,
    (r.seed >>> 24) & 255,
  ];
  const bytes: number[] = [...head];
  let acc = 0;
  let bits = 0;
  for (const m of r.moves) {
    acc = (acc << 10) | ((m & 31) << 5) | ((m >> 5) & 31);
    bits += 10;
    while (bits >= 8) {
      bits -= 8;
      bytes.push((acc >> bits) & 255);
    }
    acc &= (1 << bits) - 1;
  }
  if (bits > 0) bytes.push((acc << (8 - bits)) & 255);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** 解不開時回傳 null（格式錯誤或版本不符）；著法是否合法由引擎重播時檢查 */
export function decodeRecord(code: string): GameRecord | null {
  let bin: string;
  try {
    bin = atob(code.replace(/-/g, "+").replace(/_/g, "/"));
  } catch {
    return null;
  }
  const bytes = Array.from(bin, (c) => c.charCodeAt(0));
  if (bytes.length < 6 || bytes[0] !== VERSION) return null;
  const flags = bytes[1];
  const seed = (bytes[2] | (bytes[3] << 8) | (bytes[4] << 16) | (bytes[5] << 24)) >>> 0;
  const moves: number[] = [];
  let acc = 0;
  let bits = 0;
  for (const b of bytes.slice(6)) {
    acc = (acc << 8) | b;
    bits += 8;
    if (bits >= 10) {
      bits -= 10;
      const v = (acc >> bits) & 1023;
      moves.push((v >> 5) | ((v & 31) << 5));
      acc &= (1 << bits) - 1;
    }
  }
  return {
    mode: flags & 1 ? "ai" : "human",
    firstSeat: ((flags >> 1) & 1) as 0 | 1,
    timeA: THINK_TIMES[(flags >> 2) & 3],
    timeB: THINK_TIMES[(flags >> 4) & 3],
    seed,
    moves,
  };
}

export function recordUrl(r: GameRecord): string {
  const url = new URL(location.href);
  url.search = "";
  url.hash = "";
  url.searchParams.set("r", encodeRecord(r));
  return url.toString();
}
