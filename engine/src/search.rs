//! Expectiminimax 搜尋：alpha-beta（PVS）+ 機會節點 Star1 剪枝 + 置換表 + 靜態搜尋。

use crate::board::*;
use crate::eval::*;
use crate::tb::{Probe, Tablebase};

pub const TB_WIN: i32 = 20000;

pub const MATE: i32 = 30000;
pub const INF: i32 = 32000;
pub const MAXPLY: usize = 128;
/// 機會節點裡子節點值的上下界（Star1 需要）
const CB: i32 = EVAL_BOUND + 1000;

#[cfg(all(target_arch = "wasm32", feature = "wasm"))]
pub fn now_ms() -> f64 {
    js_sys::Date::now()
}
#[cfg(not(all(target_arch = "wasm32", feature = "wasm")))]
pub fn now_ms() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

#[derive(Clone, Copy, Default)]
struct TTEntry {
    key: u64,
    mv: u16,
    score: i16,
    depth: i8,
    flag: u8, // 0 empty, 1 exact, 2 lower, 3 upper
}

#[derive(Clone, Debug)]
pub struct SearchConfig {
    pub time_ms: f64,
    pub max_depth: i32,
    /// 翻子節點額外減少的深度
    pub flip_reduction: i32,
    /// 非根節點最多搜尋幾個翻子點
    pub interior_flips: usize,
    /// 根節點最多搜尋幾個「安靜」翻子點
    pub root_quiet_flips: usize,
    /// 根節點多候選的容許分差（用於不可預測的選擇）
    pub root_margin: i32,
    pub use_lmr: bool,
    /// 和棋對根節點方的分數懲罰（正值 = 避免和棋）
    pub contempt: i32,
    /// 空著剪枝（僅在仍有蓋牌時）
    pub null_move: bool,
    /// 非根節點只有剩餘深度 >= 此值才展開翻子（0 = 依思考時間自動決定）
    pub flip_min_depth: i32,
    /// 已用時間超過預算的此比例就不再開始新一層迭代
    pub id_stop: f64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            time_ms: 1000.0,
            max_depth: 64,
            flip_reduction: 1,
            interior_flips: 4,
            root_quiet_flips: 6,
            root_margin: 0,
            use_lmr: true,
            contempt: 0,
            null_move: true,
            flip_min_depth: 0,
            id_stop: 0.65,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RootMove {
    pub mv: Move,
    pub score: i32,
    /// 分數是否精確（在 margin 視窗內）
    pub exact: bool,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub best: Move,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
    pub time_ms: f64,
    pub root: Vec<RootMove>,
    pub pv: Vec<Move>,
}

pub struct Searcher {
    pub params: Params,
    pub cfg: SearchConfig,
    tt: Vec<TTEntry>,
    tt_mask: usize,
    killers: [[Move; 2]; MAXPLY],
    history: Vec<i32>,
    pub nodes: u64,
    stop: bool,
    start: f64,
    deadline: f64,
    path: Vec<u64>,
    root_side: u8,
    null_parent: bool,
    pub tb: Tablebase,
    pub use_tb: bool,
    fmd: i32,
}

#[inline(always)]
fn floor_div(a: i64, b: i64) -> i64 {
    let q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}
#[inline(always)]
fn ceil_div(a: i64, b: i64) -> i64 {
    -floor_div(-a, b)
}

impl Searcher {
    pub fn new(params: Params, cfg: SearchConfig, tt_bits: u32) -> Searcher {
        let n = 1usize << tt_bits;
        Searcher {
            params,
            cfg,
            tt: vec![TTEntry::default(); n],
            tt_mask: n - 1,
            killers: [[Move::NONE; 2]; MAXPLY],
            history: vec![0; 32 * 32],
            nodes: 0,
            stop: false,
            start: 0.0,
            deadline: 0.0,
            path: Vec::with_capacity(512),
            root_side: 0,
            null_parent: false,
            tb: Tablebase::new(),
            use_tb: true,
            fmd: 5,
        }
    }

    pub fn clear(&mut self) {
        for e in self.tt.iter_mut() {
            *e = TTEntry::default();
        }
        for h in self.history.iter_mut() {
            *h = 0;
        }
    }

    #[inline(always)]
    fn check_time(&mut self) {
        if self.nodes & 1023 == 0 && now_ms() > self.deadline {
            self.stop = true;
        }
    }

    fn tt_probe(&self, key: u64) -> Option<TTEntry> {
        let e = self.tt[(key as usize) & self.tt_mask];
        if e.flag != 0 && e.key == key {
            Some(e)
        } else {
            None
        }
    }

    fn tt_store(&mut self, key: u64, depth: i32, score: i32, flag: u8, mv: Move, ply: usize) {
        let i = (key as usize) & self.tt_mask;
        let e = &mut self.tt[i];
        if e.key != key || depth as i8 >= e.depth || flag == 1 {
            // 將殺分數轉成相對於此節點
            let s = if score > MATE - 500 {
                score + ply as i32
            } else if score < -MATE + 500 {
                score - ply as i32
            } else {
                score
            };
            *e = TTEntry { key, mv: mv.encode(), score: s as i16, depth: depth.clamp(-1, 120) as i8, flag };
        }
    }

    fn is_repetition(&self, pos: &Pos) -> bool {
        let n = self.path.len();
        let lim = (pos.nocap as usize).min(n);
        // path 最後一個元素就是 pos 本身
        let mut i = 2;
        while i <= lim {
            if self.path[n - 1 - i] == pos.hash {
                return true;
            }
            i += 1;
        }
        false
    }

    fn victim_value(&self, pos: &Pos, m: Move) -> i32 {
        let v = pos.cells[m.to as usize];
        if v < 14 {
            // 靜態近似值
            const V: [i32; 7] = [600, 500, 350, 250, 180, 450, 120];
            V[type_of(v) as usize]
        } else {
            0
        }
    }

    /// 排序分數
    fn order(&self, pos: &Pos, ml: &MoveList, ply: usize, tt_mv: Move, pv: &PieceVals, scores: &mut [i32], flips: bool) {
        for (i, &m) in ml.as_slice().iter().enumerate() {
            let s = if m == tt_mv {
                10_000_000
            } else if m.is_flip() {
                if flips {
                    500_000 + flip_heuristic(pos, m.from as usize, pv, &self.params) * 100
                } else {
                    -1
                }
            } else if pos.cells[m.to as usize] < 14 {
                let a = type_of(pos.cells[m.from as usize]) as i32;
                2_000_000 + self.victim_value(pos, m) * 16 - a
            } else if ply < MAXPLY && self.killers[ply][0] == m {
                1_000_000
            } else if ply < MAXPLY && self.killers[ply][1] == m {
                900_000
            } else {
                self.history[(m.from as usize) * 32 + m.to as usize].min(400_000)
            };
            scores[i] = s;
        }
    }

    #[inline(always)]
    fn draw_score(&self, pos: &Pos) -> i32 {
        if pos.side == self.root_side {
            -self.cfg.contempt
        } else {
            self.cfg.contempt
        }
    }

    pub fn qsearch(&mut self, pos: &Pos, mut alpha: i32, beta: i32, ply: usize, qd: i32) -> i32 {
        self.nodes += 1;
        self.check_time();
        if self.stop {
            return 0;
        }
        if pos.alive_color(pos.side) == 0 {
            return -MATE + ply as i32;
        }
        let stand = evaluate(pos, &self.params);
        if stand >= beta {
            return stand;
        }
        if stand > alpha {
            alpha = stand;
        }
        if qd > 16 || ply >= MAXPLY - 1 {
            return stand;
        }
        let mut ml = MoveList::new();
        pos.gen_captures(&mut ml);
        if ml.len == 0 {
            return stand;
        }
        let mut sc = [0i32; 160];
        for (i, &m) in ml.as_slice().iter().enumerate() {
            let a = type_of(pos.cells[m.from as usize]) as i32;
            sc[i] = self.victim_value(pos, m) * 16 - a;
        }
        let mut best = stand;
        for i in 0..ml.len {
            // 選擇排序
            let mut bi = i;
            for j in i + 1..ml.len {
                if sc[j] > sc[bi] {
                    bi = j;
                }
            }
            ml.moves.swap(i, bi);
            sc.swap(i, bi);
            let m = ml.moves[i];
            let child = pos.make_move(m);
            let v = -self.qsearch(&child, -beta, -alpha, ply + 1, qd + 1);
            if self.stop {
                return 0;
            }
            if v > best {
                best = v;
                if v > alpha {
                    alpha = v;
                    if v >= beta {
                        return v;
                    }
                }
            }
        }
        best
    }

    /// 機會節點：在 `sq` 翻子，回傳翻子方角度的期望值。Star1 剪枝。
    fn chance(&mut self, pos: &Pos, sq: u8, depth: i32, alpha: i32, beta: i32, ply: usize) -> i32 {
        let t = pos.pool_total() as i64;
        let alpha = alpha.max(-CB) as i64;
        let beta = beta.min(CB) as i64;
        let (l, u) = (-CB as i64, CB as i64);
        let mut order: [(u8, u8); 14] = [(0, 0); 14];
        let mut n = 0;
        for k in 0..14u8 {
            let c = pos.pool[k as usize];
            if c > 0 {
                order[n] = (k, c);
                n += 1;
            }
        }
        order[..n].sort_unstable_by(|a, b| b.1.cmp(&a.1));
        let mut s: i64 = 0;
        let mut r: i64 = t;
        for &(k, c) in order[..n].iter() {
            let c = c as i64;
            let a_num = alpha * t - s - (r - c) * u;
            let b_num = beta * t - s - (r - c) * l;
            let a = floor_div(a_num, c).max(l - 1);
            let b = ceil_div(b_num, c).min(u + 1);
            let child = pos.make_flip(sq, k);
            self.path.push(child.hash);
            let v = if child.side == pos.side {
                // 首翻：顏色剛決定
                self.negamax(&child, depth, a as i32, b as i32, ply + 1)
            } else {
                -self.negamax(&child, depth, -(b as i32), -(a as i32), ply + 1)
            };
            self.path.pop();
            if self.stop {
                return 0;
            }
            let v = (v as i64).clamp(l, u);
            if v <= a {
                return alpha as i32;
            }
            if v >= b {
                return beta as i32;
            }
            s += c * v;
            r -= c;
        }
        (s / t) as i32
    }

    pub fn negamax(&mut self, pos: &Pos, depth: i32, mut alpha: i32, beta: i32, ply: usize) -> i32 {
        self.nodes += 1;
        self.check_time();
        if self.stop {
            return 0;
        }
        if ply >= MAXPLY - 2 {
            return evaluate(pos, &self.params);
        }
        if pos.nocap >= DRAW_PLIES || (ply > 0 && self.is_repetition(pos)) {
            return self.draw_score(pos);
        }
        if pos.alive_color(pos.side) == 0 {
            return -MATE + ply as i32;
        }
        if ply > 0 && self.use_tb && pos.hidden_n == 0 {
            if let Some(p) = self.tb.probe(pos) {
                let left = (DRAW_PLIES - pos.nocap.min(DRAW_PLIES)) as u8;
                return match p {
                    Probe::Win(d) if d <= left => TB_WIN - ply as i32 - d as i32,
                    Probe::Loss(d) if d <= left => -(TB_WIN - ply as i32 - d as i32),
                    _ => self.draw_score(pos),
                };
            }
        }
        let orig_alpha = alpha;
        let mut tt_mv = Move::NONE;
        if let Some(e) = self.tt_probe(pos.hash) {
            tt_mv = Move::decode(e.mv);
            if e.depth as i32 >= depth && ply > 0 {
                let mut s = e.score as i32;
                if s > MATE - 500 {
                    s -= ply as i32;
                } else if s < -MATE + 500 {
                    s += ply as i32;
                }
                match e.flag {
                    1 => return s,
                    2 if s >= beta => return s,
                    3 if s <= alpha => return s,
                    _ => {}
                }
            }
        }
        if depth <= 0 {
            return self.qsearch(pos, alpha, beta, ply, 0);
        }

        // 空著剪枝：有蓋牌時總有「翻子」可當等著，zugzwang 風險低
        if self.cfg.null_move
            && ply > 0
            && depth >= 3
            && pos.hidden_n >= 2
            && beta.abs() < MATE - 500
            && beta - alpha == 1
            && !self.null_parent
        {
            let stand = evaluate(pos, &self.params);
            if stand >= beta {
                let mut np = pos.clone();
                np.side ^= 1;
                np.hash ^= Z_SIDE;
                np.nocap += 1;
                self.path.push(np.hash);
                self.null_parent = true;
                let r = 2 + (depth >= 7) as i32;
                let v = -self.negamax(&np, depth - 1 - r, -beta, -beta + 1, ply + 1);
                self.null_parent = false;
                self.path.pop();
                if self.stop {
                    return 0;
                }
                if v >= beta {
                    return v.min(MATE - 500);
                }
            }
        }
        self.null_parent = false;

        let mut ml = MoveList::new();
        pos.gen_moves(&mut ml);
        if ml.len == 0 {
            return -MATE + ply as i32;
        }
        let pv = piece_values(pos, &self.params);
        let mut sc = [0i32; 160];
        let flips_active = depth >= self.fmd;
        self.order(pos, &ml, ply, tt_mv, &pv, &mut sc, flips_active);

        let mut best = -INF;
        let mut best_mv = Move::NONE;
        let mut flips_done = 0usize;
        let mut quiet_flip_done = false;
        let mut searched = 0usize;
        for i in 0..ml.len {
            let mut bi = i;
            for j in i + 1..ml.len {
                if sc[j] > sc[bi] {
                    bi = j;
                }
            }
            ml.moves.swap(i, bi);
            sc.swap(i, bi);
            let m = ml.moves[i];
            let v;
            if m.is_flip() {
                if !pos.assigned || (depth < self.fmd && m != tt_mv) {
                    // 首翻在搜尋中不展開；剩餘深度太淺時不展開翻子
                    continue;
                }
                let quiet = sc[i] == 500_000 && m != tt_mv;
                if quiet {
                    if quiet_flip_done {
                        continue;
                    }
                    quiet_flip_done = true;
                }
                if flips_done >= self.cfg.interior_flips && m != tt_mv {
                    continue;
                }
                flips_done += 1;
                let d = depth - 1 - self.cfg.flip_reduction;
                v = self.chance(pos, m.from, d, alpha, beta, ply);
            } else {
                let is_cap = pos.cells[m.to as usize] < 14;
                let child = pos.make_move(m);
                self.path.push(child.hash);
                if searched == 0 {
                    v = -self.negamax(&child, depth - 1, -beta, -alpha, ply + 1);
                } else {
                    let mut r = 0;
                    if self.cfg.use_lmr && depth >= 3 && !is_cap && searched >= 3 && sc[i] < 900_000 {
                        r = 1 + (searched >= 8) as i32;
                    }
                    let mut vv = -self.negamax(&child, depth - 1 - r, -alpha - 1, -alpha, ply + 1);
                    if vv > alpha && r > 0 {
                        vv = -self.negamax(&child, depth - 1, -alpha - 1, -alpha, ply + 1);
                    }
                    if vv > alpha && vv < beta {
                        vv = -self.negamax(&child, depth - 1, -beta, -alpha, ply + 1);
                    }
                    v = vv;
                }
                self.path.pop();
            }
            if self.stop {
                return 0;
            }
            searched += 1;
            if v > best {
                best = v;
                best_mv = m;
                if v > alpha {
                    alpha = v;
                    if v >= beta {
                        if !m.is_flip() && pos.cells[m.to as usize] == EMPTY {
                            if self.killers[ply][0] != m {
                                self.killers[ply][1] = self.killers[ply][0];
                                self.killers[ply][0] = m;
                            }
                            let h = &mut self.history[(m.from as usize) * 32 + m.to as usize];
                            *h += depth * depth;
                            if *h > 300_000 {
                                for x in self.history.iter_mut() {
                                    *x /= 2;
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
        if best == -INF {
            // 只剩被略過的翻子
            return evaluate(pos, &self.params);
        }
        let flag = if best >= beta {
            2
        } else if best > orig_alpha {
            1
        } else {
            3
        };
        self.tt_store(pos.hash, depth, best, flag, best_mv, ply);
        best
    }

    /// 根節點：產生候選著法（含翻子剪枝）
    fn root_moves(&self, pos: &Pos) -> Vec<Move> {
        let mut ml = MoveList::new();
        pos.gen_moves(&mut ml);
        let pv = piece_values(pos, &self.params);
        let mut out = Vec::new();
        let mut quiet: Vec<Move> = Vec::new();
        for &m in ml.as_slice() {
            if m.is_flip() && pos.assigned {
                let h = flip_heuristic(pos, m.from as usize, &pv, &self.params);
                let exposed = NEIGH[m.from as usize].iter().any(|&n| n != 255 && pos.cells[n as usize] < 14);
                if h == 0 && !exposed {
                    quiet.push(m);
                    continue;
                }
            }
            out.push(m);
        }
        // 安靜翻子：挑離敵方強子遠、靠近己方可保護的位置（簡單打分），保留前幾個
        if !quiet.is_empty() {
            let mut qs: Vec<(i32, Move)> = quiet.iter().map(|&m| (quiet_flip_score(pos, m.from as usize), m)).collect();
            qs.sort_by(|a, b| b.0.cmp(&a.0));
            for &(_, m) in qs.iter().take(self.cfg.root_quiet_flips.max(1)) {
                out.push(m);
            }
        }
        out
    }

    pub fn think(&mut self, pos: &Pos, game_history: &[u64]) -> SearchResult {
        self.start = now_ms();
        self.root_side = pos.side;
        if self.use_tb && pos.hidden_n == 0 {
            if Tablebase::eligible(pos) {
                self.tb.ensure(pos);
            } else if self.cfg.time_ms >= 1500.0 {
                self.tb.ensure_next(pos);
            }
        }
        self.deadline = self.start + self.cfg.time_ms;
        self.fmd = if self.cfg.flip_min_depth > 0 {
            self.cfg.flip_min_depth
        } else {
            5 + (self.cfg.time_ms >= 80.0) as i32 + (self.cfg.time_ms >= 800.0) as i32
        };
        self.stop = false;
        self.nodes = 0;
        self.killers = [[Move::NONE; 2]; MAXPLY];
        for h in self.history.iter_mut() {
            *h /= 8;
        }
        self.path.clear();
        self.path.extend_from_slice(game_history);
        if self.path.last() != Some(&pos.hash) {
            self.path.push(pos.hash);
        }

        let moves = self.root_moves(pos);
        let mut root: Vec<RootMove> = moves.iter().map(|&m| RootMove { mv: m, score: -INF, exact: false }).collect();
        if root.is_empty() {
            return SearchResult { best: Move::NONE, score: -MATE, depth: 0, nodes: 0, time_ms: 0.0, root, pv: vec![] };
        }
        if !pos.assigned || root.len() == 1 {
            let m = root[0].mv;
            return SearchResult { best: m, score: 0, depth: 0, nodes: 0, time_ms: 0.0, root, pv: vec![m] };
        }
        // 初始排序
        {
            let pv = piece_values(pos, &self.params);
            let ml = {
                let mut l = MoveList::new();
                for r in root.iter() {
                    l.push(r.mv);
                }
                l
            };
            let mut sc = [0i32; 160];
            self.order(pos, &ml, 0, Move::NONE, &pv, &mut sc, true);
            let mut idx: Vec<usize> = (0..root.len()).collect();
            idx.sort_by(|&a, &b| sc[b].cmp(&sc[a]));
            root = idx.into_iter().map(|i| root[i].clone()).collect();
        }

        let mut completed: Vec<RootMove> = root.clone();
        let mut done_depth = 0;
        let margin = self.cfg.root_margin.max(0);
        for depth in 1..=self.cfg.max_depth {
            let mut best = -INF;
            let mut cur = root.clone();
            let mut aborted = false;
            for (i, rm) in cur.iter_mut().enumerate() {
                let m = rm.mv;
                let lo = if i == 0 { -INF } else { best - margin - 1 };
                let v = if m.is_flip() {
                    let d = depth - 1 - self.cfg.flip_reduction;
                    self.chance(pos, m.from, d, lo, INF, 0)
                } else {
                    let child = pos.make_move(m);
                    self.path.push(child.hash);
                    let v = if i == 0 {
                        -self.negamax(&child, depth - 1, -INF, INF, 1)
                    } else {
                        let v0 = -self.negamax(&child, depth - 1, -lo - 1, -lo, 1);
                        if v0 > lo && !self.stop {
                            -self.negamax(&child, depth - 1, -INF, -lo, 1)
                        } else {
                            v0
                        }
                    };
                    self.path.pop();
                    v
                };
                if self.stop {
                    aborted = true;
                    // 若本層已找到比上一層最佳更好的著法，保留
                    if i > 0 && best > -INF {
                        let mut merged = cur.clone();
                        for r in merged.iter_mut().skip(i) {
                            r.score = -INF;
                            r.exact = false;
                        }
                        merged.sort_by(|a, b| b.score.cmp(&a.score));
                        if merged[0].mv != completed[0].mv && merged[0].score > completed[0].score - 30 {
                            completed = merged;
                        }
                    }
                    break;
                }
                rm.score = v;
                rm.exact = v > lo;
                if v > best {
                    best = v;
                }
            }
            if aborted {
                break;
            }
            cur.sort_by(|a, b| b.score.cmp(&a.score));
            completed = cur.clone();
            root = cur;
            done_depth = depth;
            if best.abs() > MATE - 200 {
                break;
            }
            let el = now_ms() - self.start;
            if el > self.cfg.time_ms * self.cfg.id_stop {
                break;
            }
        }
        let best = completed[0].mv;
        let score = completed[0].score;
        let pv = self.extract_pv(pos, best);
        SearchResult {
            best,
            score,
            depth: done_depth,
            nodes: self.nodes,
            time_ms: now_ms() - self.start,
            root: completed,
            pv,
        }
    }

    fn extract_pv(&self, pos: &Pos, first: Move) -> Vec<Move> {
        let mut pv = vec![first];
        if first.is_flip() {
            return pv;
        }
        let mut p = pos.make_move(first);
        for _ in 0..12 {
            match self.tt_probe(p.hash) {
                Some(e) => {
                    let m = Move::decode(e.mv);
                    if m.is_none() || m.is_flip() {
                        if m.is_flip() {
                            pv.push(m);
                        }
                        break;
                    }
                    let mut ml = MoveList::new();
                    p.gen_moves(&mut ml);
                    if !ml.as_slice().contains(&m) {
                        break;
                    }
                    pv.push(m);
                    p = p.make_move(m);
                }
                None => break,
            }
        }
        pv
    }
}

/// 安靜翻子點的偏好：靠近己方強子（可保護）、遠離敵方能吃我方的子；避開角落以外的中心堵塞
pub fn quiet_flip_score(pos: &Pos, sq: usize) -> i32 {
    let me = pos.side;
    let mut s = 0i32;
    for t in 0..NSQ {
        let q = pos.cells[t];
        if q >= 14 {
            continue;
        }
        let d = DIST[sq][t] as i32;
        let w = (8 - d).max(0);
        if color_of(q) == me {
            s += w * (7 - type_of(q) as i32);
        } else {
            s -= w * (7 - type_of(q) as i32);
        }
    }
    // 邊角翻子較少被炮線牽連
    let r = sq / COLS;
    let c = sq % COLS;
    if r == 0 || r == ROWS - 1 {
        s += 2;
    }
    if c == 0 || c == COLS - 1 {
        s += 2;
    }
    s
}
