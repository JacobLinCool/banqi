// WebMCP：把遊戲操作註冊成工具，讓瀏覽器裡的 AI 助理可以讀取棋局並下棋。
// 規格：https://github.com/webmachinelearning/webmcp（document.modelContext.registerTool）；
// 早期的 Chrome 預覽版是 navigator.modelContext 與 provideContext，兩者都支援。不支援的瀏覽器直接略過。

export interface Coord {
  from: number;
  to: number;
}

/** main.ts 提供給工具的遊戲操作；回傳值會以 JSON 文字交給 AI */
export interface GameApi {
  describe(): unknown;
  rules(): string;
  play(move: Coord): Promise<unknown>;
  suggest(timeMs: number): Promise<unknown>;
  newGame(opts: { mode?: "human" | "ai"; first?: "you" | "engine" | "random"; thinkTimeMs?: number }): Promise<unknown>;
  undo(): Promise<unknown>;
  shareLink(): string;
  openReplay(link: string): Promise<unknown>;
  replayGoto(ply: number): Promise<unknown>;
}

interface ToolResult {
  content: { type: "text"; text: string }[];
  isError?: boolean;
}

interface Tool {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
  annotations?: { readOnlyHint?: boolean };
  execute(input: Record<string, unknown>): Promise<ToolResult>;
}

const sqIndex = (t: string) => (Number(t[1]) - 1) * 8 + (t.charCodeAt(0) - 97);

/** 解析著法："a1"、"flip a1"、"a1 翻" 為翻子；"a1-a2"、"a1xa2"、"a1a2"、"a1 a2" 為走子或吃子 */
export function parseMove(text: string): Coord | null {
  const t = text.trim().toLowerCase();
  const flip = t.match(/^(?:flip\s+|翻\s*)?([a-h][1-4])(?:\s*(?:flip|翻))?$/);
  if (flip) return { from: sqIndex(flip[1]), to: sqIndex(flip[1]) };
  const mv = t.match(/^([a-h][1-4])\s*[-x×:>\s]?\s*([a-h][1-4])$/);
  if (mv) return { from: sqIndex(mv[1]), to: sqIndex(mv[2]) };
  return null;
}

const text = (v: unknown): ToolResult => ({
  content: [{ type: "text", text: typeof v === "string" ? v : JSON.stringify(v, null, 2) }],
});

function wrap(fn: (input: Record<string, unknown>) => unknown): Tool["execute"] {
  return async (input) => {
    try {
      return text(await fn(input ?? {}));
    } catch (e) {
      return { ...text(`Error: ${(e as Error).message}`), isError: true };
    }
  };
}

const MOVE_FORMAT =
  'Squares are file a–h + rank 1–4 (a1 is bottom-left). Flip a face-down piece with "flip a1" (or just "a1"); move or capture with "a1-a2" / "a1xa2".';

export function registerWebMcp(api: GameApi): boolean {
  const doc = document as unknown as { modelContext?: ModelContext };
  const nav = navigator as unknown as { modelContext?: ModelContext };
  const mc = doc.modelContext ?? nav.modelContext;
  if (!mc) return false;

  const tools: Tool[] = [
    {
      name: "get_game_state",
      description:
        "Read the current Taiwanese Chinese Dark Chess (暗棋 / banqi) game on this page: board (4 ranks × 8 files), whose turn it is, your colour, legal moves, captured and still-hidden pieces, the no-progress draw counter, move history and the result. Red pieces are 帥仕相俥傌炮兵, black are 將士象車馬包卒, ■ is face-down, ・ is empty. Call this before choosing a move.",
      inputSchema: { type: "object", properties: {} },
      annotations: { readOnlyHint: true },
      execute: wrap(() => api.describe()),
    },
    {
      name: "get_rules",
      description: "Read the full rules of Taiwanese Chinese Dark Chess as played on this page (piece ranks, cannon jumps, draw rules).",
      inputSchema: { type: "object", properties: {} },
      annotations: { readOnlyHint: true },
      execute: wrap(() => api.rules()),
    },
    {
      name: "make_move",
      description: `Play one move for the human player ("you") in a human-vs-computer game, then wait for the computer's reply. ${MOVE_FORMAT} Returns your move, the computer's reply and the new game state. Only legal moves on your turn are accepted.`,
      inputSchema: {
        type: "object",
        properties: { move: { type: "string", description: 'e.g. "flip c2", "c2-c3", "c3xd3"' } },
        required: ["move"],
      },
      execute: wrap(async ({ move }) => {
        const m = parseMove(String(move ?? ""));
        if (!m) throw new Error(`Cannot parse move "${move}". ${MOVE_FORMAT}`);
        return api.play(m);
      }),
    },
    {
      name: "suggest_move",
      description:
        "Ask the built-in engine (expectiminimax search with an endgame tablebase) for the best move for you in the current position, without playing it. Returns the move, evaluation, win probability, principal variation and alternatives.",
      inputSchema: {
        type: "object",
        properties: { thinkTimeMs: { type: "number", description: "Search time in ms (200–10000, default 1500)" } },
      },
      annotations: { readOnlyHint: true },
      execute: wrap(({ thinkTimeMs }) => api.suggest(Math.max(200, Math.min(10000, Number(thinkTimeMs) || 1500)))),
    },
    {
      name: "new_game",
      description:
        'Start a new game with a fresh random deal. mode "human" = you play against the computer; mode "ai" = watch two engines play each other. Omitted options keep the page\'s current settings.',
      inputSchema: {
        type: "object",
        properties: {
          mode: { type: "string", enum: ["human", "ai"] },
          first: { type: "string", enum: ["you", "engine", "random"], description: "Who flips first in human mode" },
          thinkTimeMs: { type: "number", enum: [500, 2000, 5000, 10000], description: "Computer think time per move" },
        },
      },
      execute: wrap((i) =>
        api.newGame({
          mode: i.mode as "human" | "ai" | undefined,
          first: i.first as "you" | "engine" | "random" | undefined,
          thinkTimeMs: i.thinkTimeMs === undefined ? undefined : Number(i.thinkTimeMs),
        }),
      ),
    },
    {
      name: "undo",
      description: "Take back your last move (and the computer's reply) in a human-vs-computer game.",
      inputSchema: { type: "object", properties: {} },
      execute: wrap(() => api.undo()),
    },
    {
      name: "get_share_link",
      description: "Get a link that replays the current game move by move (the deal and every move are encoded in the ?r= query string).",
      inputSchema: { type: "object", properties: {} },
      annotations: { readOnlyHint: true },
      execute: wrap(() => api.shareLink()),
    },
    {
      name: "open_replay",
      description: "Open a shared replay link (or just its ?r= code) on this page. Use replay_goto to step through it.",
      inputSchema: {
        type: "object",
        properties: { link: { type: "string", description: "Full replay URL or the r= code" } },
        required: ["link"],
      },
      execute: wrap(({ link }) => api.openReplay(String(link ?? ""))),
    },
    {
      name: "replay_goto",
      description: "While viewing a replay, jump to the position after the given number of moves (0 = the initial deal) and return that position.",
      inputSchema: {
        type: "object",
        properties: { ply: { type: "number", description: "Number of moves played (0 … total)" } },
        required: ["ply"],
      },
      execute: wrap(({ ply }) => api.replayGoto(Math.floor(Number(ply) || 0))),
    },
  ];

  if (typeof mc.registerTool === "function") {
    for (const t of tools) Promise.resolve(mc.registerTool(t)).catch((e) => console.warn("WebMCP registerTool failed", t.name, e));
  } else if (typeof mc.provideContext === "function") {
    mc.provideContext({ tools });
  } else return false;
  return true;
}

interface ModelContext {
  registerTool?(tool: Tool, options?: { signal?: AbortSignal }): unknown;
  provideContext?(ctx: { tools: Tool[] }): void;
}
