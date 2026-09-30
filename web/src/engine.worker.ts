/// <reference lib="webworker" />
import { CdcGame, initSync } from "./pkg/cdc_engine.js";
import wasmBase64 from "./pkg/cdc_engine_bg.wasm?b64";
import type { Analysis, HistoryEntry, Request, Response, Snapshot } from "./shared";

const bytes = Uint8Array.from(atob(wasmBase64), (c) => c.charCodeAt(0));
initSync({ module: bytes });

let game: CdcGame | null = null;

function snapshot(g: CdcGame): Snapshot {
  return {
    cells: Array.from(g.cells()),
    side: g.side(),
    assigned: g.assigned(),
    nocap: g.nocap(),
    drawPlies: g.draw_plies(),
    ply: g.ply(),
    captured: Array.from(g.captured()),
    pool: Array.from(g.pool()),
    legal: Array.from(g.legal_moves()),
    outcome: g.outcome(),
    reason: g.outcome_reason(),
    history: JSON.parse(g.history_json()) as HistoryEntry[],
  };
}

function handle(req: Request): { state: Snapshot; analysis?: Analysis } {
  if (req.type === "new") {
    game?.free();
    game = new CdcGame(req.seed);
    return { state: snapshot(game) };
  }
  if (!game) throw new Error("尚未開局");
  switch (req.type) {
    case "play":
      if (game.play(req.from, req.to) === 255) throw new Error("不合法的著法");
      return { state: snapshot(game) };
    case "undo":
      game.undo(req.n);
      return { state: snapshot(game) };
    case "think": {
      game.set_variety(req.variety);
      const analysis = JSON.parse(game.think(req.timeMs)) as Analysis;
      if (game.play(analysis.move.from, analysis.move.to) === 255) throw new Error("引擎回傳不合法著法");
      return { state: snapshot(game), analysis };
    }
  }
}

self.onmessage = (e: MessageEvent<{ id: number; req: Request }>) => {
  const { id, req } = e.data;
  let res: Response;
  try {
    res = { id, ok: true, ...handle(req) };
  } catch (err) {
    res = { id, ok: false, error: String((err as Error)?.message ?? err) };
  }
  self.postMessage(res);
};
