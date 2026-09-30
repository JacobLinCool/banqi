//! Mixture of Experts 決策層。
//!
//! 由 gating 函數依局面特徵（未翻子數、子力差、剩餘棋子、無進展步數）決定各專家的權重：
//! - 開局翻子專家：翻子位置的風險 / 保護評估
//! - 戰術搜尋專家：Expectiminimax 的主搜尋分數（所有決策的骨幹）
//! - 殘局獵手：無蓋牌時的追殺、逼角
//! - 守勢專家：落後時偏好降低風險、保住和棋
//! - 攻勢專家：領先時偏好吃子 / 施壓、避免重複
//! 最終在「搜尋分數接近最佳」的候選中，用各專家偏好加權後以低溫度抽樣，讓對手難以預測。

use crate::board::*;
use crate::eval::*;
use crate::game::*;
use crate::search::*;

#[derive(Clone, Debug)]
pub struct ExpertWeight {
    pub name: &'static str,
    pub weight: f64,
}

#[derive(Clone, Debug)]
pub struct Decision {
    pub mv: Move,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
    pub time_ms: f64,
    pub pv: Vec<Move>,
    pub experts: Vec<ExpertWeight>,
    pub candidates: Vec<(Move, i32)>,
}

pub struct Brain {
    pub searcher: Searcher,
    pub rng: Rng,
    /// 0 = 完全確定性；越大越多變化
    pub variety: f64,
}

pub struct Features {
    pub hidden: f64,
    pub material: f64,
    pub my_pieces: f64,
    pub opp_pieces: f64,
    pub nocap: f64,
}

pub fn features(pos: &Pos, params: &Params) -> Features {
    let pv = piece_values(pos, params);
    let me = pos.side as usize;
    let mut m = [0f64; 2];
    for c in 0..2 {
        for t in 0..7 {
            m[c] += pos.alive[c * 7 + t] as f64 * pv.v[c][t] as f64;
        }
    }
    Features {
        hidden: pos.hidden_n as f64,
        material: m[me] - m[1 - me],
        my_pieces: pos.alive_color(me as u8) as f64,
        opp_pieces: pos.alive_color(1 - me as u8) as f64,
        nocap: pos.nocap as f64,
    }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

pub fn gate(f: &Features) -> [f64; 6] {
    let opening = sigmoid((f.hidden - 20.0) / 3.0);
    let endgame = sigmoid((4.0 - f.hidden) / 1.5);
    let tactical = 1.0;
    let defend = sigmoid((-f.material - 150.0) / 80.0);
    let attack = sigmoid((f.material - 150.0) / 80.0);
    // 殘局資料庫：全翻開且 ≤4 子時可給出完美解，壓過其他專家
    let tablebase = if f.hidden == 0.0 && f.my_pieces + f.opp_pieces <= 4.0 { 6.0 } else { 0.0 };
    let raw = [opening, tactical, endgame, defend, attack, tablebase];
    let s: f64 = raw.iter().sum();
    let mut out = [0.0; 6];
    for i in 0..6 {
        out[i] = raw[i] / s;
    }
    out
}

pub const EXPERT_NAMES: [&str; 6] = ["開局翻子", "戰術搜尋", "殘局獵手", "守勢", "攻勢", "殘局資料庫"];

impl Brain {
    pub fn new(seed: u64, tt_bits: u32) -> Brain {
        Brain {
            searcher: Searcher::new(Params::default(), SearchConfig { root_margin: 12, contempt: 10, ..Default::default() }, tt_bits),
            rng: Rng::new(seed),
            variety: 1.0,
        }
    }

    /// 專家對單一候選著法的偏好（分數單位與評估一致，數值小，只在接近的候選間起作用）
    fn expert_bias(&self, pos: &Pos, m: Move, g: &[f64; 6], hist: &[u64]) -> f64 {
        let params = &self.searcher.params;
        let pv = piece_values(pos, params);
        let mut b = 0.0;
        if m.is_flip() {
            // 開局：翻子位置的風險 + 安靜位置偏好
            let fh = flip_heuristic(pos, m.from as usize, &pv, params) as f64;
            let q = quiet_flip_score(pos, m.from as usize) as f64;
            b += g[0] * (fh * 0.3 + q * 0.4);
            // 守勢：翻子帶來變數，落後時適度偏好
            b += g[3] * 6.0;
            // 攻勢：領先時不想增加變數
            b -= g[4] * 6.0;
        } else {
            let cap = pos.cells[m.to as usize] < 14;
            let child = pos.make_move(m);
            if cap {
                b += g[4] * 10.0;
            }
            // 攻勢 / 殘局：避免重複局面
            if hist.iter().rev().take(child.nocap as usize + 1).any(|&h| h == child.hash) {
                b -= (g[4] + g[2]) * 40.0;
                b += g[3] * 15.0;
            }
            // 殘局獵手：縮短與可吃目標的距離
            if g[2] > 0.05 {
                let t = type_of(pos.cells[m.from as usize]) as usize;
                let opp = pos.side ^ 1;
                let mut before = 99;
                let mut after = 99;
                for s in 0..NSQ {
                    let q = pos.cells[s];
                    if q < 14 && color_of(q) == opp && ADJ_CAP[t][type_of(q) as usize] {
                        before = before.min(DIST[m.from as usize][s]);
                        after = after.min(DIST[m.to as usize][s]);
                    }
                }
                if before < 99 && after < before {
                    b += g[2] * 8.0;
                }
            }
        }
        b
    }

    pub fn decide(&mut self, game: &Game, time_ms: f64) -> Decision {
        let pos = &game.pos;
        let f = features(pos, &self.searcher.params);
        let g = gate(&f);
        let experts: Vec<ExpertWeight> =
            (0..6).map(|i| ExpertWeight { name: EXPERT_NAMES[i], weight: g[i] }).collect();
        if !pos.assigned {
            // 首翻：隨機（對手無從預測），略偏好邊線
            let edge = [0usize, 7, 8, 15, 16, 23, 24, 31, 1, 6, 25, 30];
            let s = if self.rng.unit() < 0.5 {
                edge[self.rng.below(edge.len() as u64) as usize]
            } else {
                self.rng.below(32) as usize
            };
            let m = Move::flip(s as u8);
            return Decision { mv: m, score: 0, depth: 0, nodes: 0, time_ms: 0.0, pv: vec![m], experts, candidates: vec![] };
        }
        self.searcher.cfg.time_ms = time_ms;
        let r = self.searcher.think(pos, &game.history);
        let best_score = r.score;
        // 候選：精確分數且與最佳差距在容許範圍內
        let margin = self.searcher.cfg.root_margin as f64;
        let mut cands: Vec<(Move, f64, i32)> = vec![];
        for rm in r.root.iter() {
            if rm.score == -INF {
                continue;
            }
            if rm.mv == r.best || (rm.exact && (best_score - rm.score) as f64 <= margin) {
                let bias = self.expert_bias(pos, rm.mv, &g, &game.history);
                cands.push((rm.mv, rm.score as f64 + bias, rm.score));
            }
        }
        let mut chosen = r.best;
        if best_score.abs() < MATE - 500 && cands.len() > 1 {
            // 開局（翻子多半等價）放大隨機性；戰術 / 殘局關鍵局面收斂到最佳著
            let calm = (1.0 - (best_score - cands.iter().map(|c| c.2).min().unwrap_or(best_score)) as f64 / 40.0).clamp(0.3, 1.0);
            let temp = 4.0 * self.variety.max(0.01) * (0.25 + 0.75 * g[0] / g[0].max(0.2).max(g[1])) * calm;
            let mx = cands.iter().map(|c| c.1).fold(f64::MIN, f64::max);
            let ws: Vec<f64> = cands.iter().map(|c| ((c.1 - mx) / temp).exp()).collect();
            let tot: f64 = ws.iter().sum();
            let mut x = self.rng.unit() * tot;
            for (i, w) in ws.iter().enumerate() {
                x -= w;
                if x <= 0.0 {
                    chosen = cands[i].0;
                    break;
                }
            }
        }
        let score = r.root.iter().find(|rm| rm.mv == chosen).map(|rm| rm.score).unwrap_or(best_score);
        let pv = if chosen == r.best { r.pv.clone() } else { vec![chosen] };
        Decision {
            mv: chosen,
            score,
            depth: r.depth,
            nodes: r.nodes,
            time_ms: r.time_ms,
            pv,
            experts,
            candidates: r.root.iter().filter(|rm| rm.score > -INF).take(8).map(|rm| (rm.mv, rm.score)).collect(),
        }
    }
}
