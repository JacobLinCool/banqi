import EngineWorker from "./engine.worker?worker&inline";
import type { Analysis, Request, Response, Snapshot } from "./shared";

export interface Reply {
  state: Snapshot;
  analysis?: Analysis;
}

/** Promise wrapper around the engine worker. `terminate()` aborts a running search instantly. */
export class Engine {
  private worker = new EngineWorker();
  private seq = 0;
  private pending = new Map<number, { resolve: (r: Reply) => void; reject: (e: Error) => void }>();

  constructor() {
    this.worker.onmessage = (e: MessageEvent<Response>) => {
      const res = e.data;
      const p = this.pending.get(res.id);
      if (!p) return;
      this.pending.delete(res.id);
      if (res.ok) p.resolve({ state: res.state, analysis: res.analysis });
      else p.reject(new Error(res.error));
    };
    this.worker.onerror = (e) => this.failAll(new Error(e.message || "引擎錯誤"));
  }

  call(req: Request): Promise<Reply> {
    const id = ++this.seq;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage({ id, req });
    });
  }

  terminate() {
    this.worker.terminate();
    this.failAll(new Error("terminated"));
  }

  private failAll(err: Error) {
    for (const p of this.pending.values()) p.reject(err);
    this.pending.clear();
  }
}
