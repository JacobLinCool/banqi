//! 評估函數：動態子力（依可吃 / 被吃的敵子數量）+ 追殺 / 危險距離 + 機動性。
//! 所有權重都在 `Params` 裡，方便自我對弈調參（SPSA）。

use crate::board::*;

pub const EVAL_BOUND: i32 = 9000;

pub mod idx {
    pub const BASE: usize = 0; // 0..7
    pub const CAPW: usize = 7;
    pub const THRW: usize = 8;
    pub const INVULN: usize = 9;
    pub const CHASE: usize = 10;
    pub const DANGER: usize = 11;
    pub const MOBILITY: usize = 12;
    pub const TEMPO: usize = 13;
    pub const KING_PAWN: usize = 14;
    pub const PROTECT: usize = 15;
    pub const TRAPPED: usize = 16;
    pub const LATE_CHASE: usize = 17;
    pub const NOCAP_DECAY: usize = 18;
    pub const LAST_PIECES: usize = 19;
    pub const CANNON_LINE: usize = 20;
    pub const HIDDEN_NEAR: usize = 21;
    pub const MAT_MODE: usize = 22;
    pub const OBS_TOP: usize = 23;
    pub const OBS_RATIO: usize = 24;
    pub const OBS_CANNON: usize = 25;
    pub const OBS_KING: usize = 26;
    pub const PARITY: usize = 27;
    pub const FLIP_ADV: usize = 28;
    pub const FLIP_ELE: usize = 29;
    pub const FLIP_KING: usize = 30;
    pub const CONFINE: usize = 31;
    pub const TYPE_ADD: usize = 32; // 32..39：各兵種的額外加成
    pub const HIDDEN_PCT: usize = 39;
    pub const NP: usize = 40;
}

pub const PARAM_NAMES: [&str; idx::NP] = [
    "base_king", "base_advisor", "base_elephant", "base_chariot", "base_horse", "base_cannon", "base_pawn",
    "capw", "thrw", "invuln", "chase", "danger", "mobility", "tempo", "king_pawn", "protect", "trapped",
    "late_chase", "nocap_decay", "last_pieces", "cannon_line", "hidden_near", "mat_mode", "obs_top", "obs_ratio",
    "obs_cannon", "obs_king", "parity", "flip_adv", "flip_ele", "flip_king", "confine", "add_king", "add_advisor", "add_elephant", "add_chariot", "add_horse",
    "add_cannon", "add_pawn", "hidden_pct",
];

#[derive(Clone, Debug)]
pub struct Params {
    /// 一般（開局 / 中局）專家
    pub w: [i32; idx::NP],
    /// 殘局專家：未翻子少時由 gating 逐步接手
    pub e: [i32; idx::NP],
}

impl Params {
    pub fn set(&mut self, key: &str, v: i32) -> bool {
        let (arr, name) = match key.strip_prefix("e.") {
            Some(n) => (&mut self.e, n),
            None => (&mut self.w, key),
        };
        match PARAM_NAMES.iter().position(|&n| n == name) {
            Some(i) => {
                arr[i] = v;
                true
            }
            None => false,
        }
    }
    pub fn get(&self, key: &str) -> Option<i32> {
        let (arr, name) = match key.strip_prefix("e.") {
            Some(n) => (&self.e, n),
            None => (&self.w, key),
        };
        PARAM_NAMES.iter().position(|&n| n == name).map(|i| arr[i])
    }
    /// 殘局 gating 權重（0..64）：未翻子 ≥ 8 時為 0，全翻開時為 64
    #[inline(always)]
    pub fn endgame_gate(hidden: u8) -> i32 {
        ((8 - hidden.min(8) as i32) * 8).min(64)
    }
    /// 依 gating 混合兩位專家的參數
    pub fn blended(&self, hidden: u8) -> Params {
        let g = Self::endgame_gate(hidden);
        let mut w = self.w;
        if g > 0 {
            for i in 0..idx::NP {
                if i != idx::MAT_MODE {
                    w[i] = (self.w[i] * (64 - g) + self.e[i] * g) / 64;
                }
            }
        }
        Params { w, e: self.e }
    }
}

impl Default for Params {
    fn default() -> Self {
        let w = [
                252, 210, 150, 110, 90, 198, 84, // base
                6,   // capw
                4,   // thrw
                120, // invuln
                4,  // chase
                4,  // danger
                12,   // mobility
                11,  // tempo
                6,   // king_pawn
                14,  // protect
                24,  // trapped
                11,  // late_chase
                56,  // nocap_decay (百分比：在和棋邊緣時保留多少)
                68,  // last_pieces
                18,   // cannon_line
                10,   // hidden_near
                1,   // mat_mode：0 線性、1 天敵數指數表
                579, // obs_top：無天敵時的價值
                49,  // obs_ratio：每多一個天敵價值乘上的百分比
                276, // obs_cannon
                78,  // obs_king：帥/將每多一隻敵兵的價值百分比
                15,  // parity：殘局一對一追殺的奇偶性（/64）
                43,  // flip_adv：翻己方仕旁邊的加分
                21,  // flip_ele：翻己方相旁邊的加分
                59,  // flip_king：敵兵未翻時翻己方帥旁邊的扣分
                25,  // confine：被圍困（安全逃生格少）的懲罰
                -5, -4, 9, 7, 1, 12, 29, // add_*：各兵種額外加成
                83, // hidden_pct：未翻開棋子的價值百分比
            ];
        #[allow(unused_mut)]
        let mut e = w;
        e[10] = 7; // chase
        e[11] = 5; // danger
        e[12] = 15; // mobility
        e[13] = 10; // tempo
        e[15] = 16; // protect
        e[16] = 21; // trapped
        e[18] = 43; // nocap_decay
        e[19] = 78; // last_pieces
        e[20] = 13; // cannon_line
        e[24] = 40; // obs_ratio
        e[25] = 386; // obs_cannon
        e[27] = 12; // parity
        e[31] = 28; // confine
        e[26] = 66; // obs_king
        e[39] = 104; // hidden_pct
        Params { w, e }
    }
}

/// 距離權重（索引為曼哈頓距離），/64
const NEAR: [i32; 12] = [0, 64, 40, 26, 17, 11, 7, 4, 2, 1, 0, 0];

pub struct PieceVals {
    pub v: [[i32; 7]; 2],
}

pub fn piece_values(pos: &Pos, p: &Params) -> PieceVals {
    if p.w[idx::MAT_MODE] == 1 {
        return piece_values_obs(pos, p);
    }
    let w = &p.w;
    let mut v = [[0i32; 7]; 2];
    for c in 0..2usize {
        let o = 1 - c;
        let ob = o * 7;
        for t in 0..7usize {
            let mut caps = 0i32;
            let mut thr = 0i32;
            for u in 0..7usize {
                let n = pos.alive[ob + u] as i32;
                if ANY_CAP[t][u] {
                    caps += n;
                }
                if ANY_CAP[u][t] {
                    thr += n;
                }
            }
            let mut val = w[idx::BASE + t] + w[idx::CAPW] * caps - w[idx::THRW] * thr;
            if t == KING as usize {
                val -= w[idx::KING_PAWN] * pos.alive[ob + PAWN as usize] as i32;
            }
            if thr == 0 && caps > 0 {
                val += w[idx::INVULN];
            }
            if caps == 0 {
                // 什麼都吃不到的棋子只剩下擋路 / 當炮架的價值
                val = val / 3;
            }
            v[c][t] = val.max(10);
        }
    }
    PieceVals { v }
}

/// 天敵數模型（參考 Observer）：價值隨「能吃掉它的敵方非炮棋子數」指數遞減。
fn piece_values_obs(pos: &Pos, p: &Params) -> PieceVals {
    let w = &p.w;
    let top = w[idx::OBS_TOP] as f64;
    let r = w[idx::OBS_RATIO] as f64 / 100.0;
    let rk = w[idx::OBS_KING] as f64 / 100.0;
    let mut v = [[0i32; 7]; 2];
    for c in 0..2usize {
        let ob = (1 - c) * 7;
        for t in 0..7usize {
            let mut caps = 0;
            for u in 0..7usize {
                if ANY_CAP[t][u] {
                    caps += pos.alive[ob + u] as i32;
                }
            }
            let val = if t == CANNON as usize {
                w[idx::OBS_CANNON] as f64
            } else if t == KING as usize {
                top * rk.powi(pos.alive[ob + PAWN as usize] as i32)
            } else {
                let mut n = 0;
                for u in 0..7usize {
                    if u != CANNON as usize && u != t && ADJ_CAP[u][t] {
                        n += pos.alive[ob + u] as i32;
                    }
                }
                top * r.powi(n)
            };
            let mut val = val as i32 + w[idx::TYPE_ADD + t];
            if caps == 0 {
                val /= 3;
            }
            v[c][t] = val.max(2);
        }
    }
    PieceVals { v }
}

/// 從輪到方的角度評估。
pub fn evaluate(pos: &Pos, p0: &Params) -> i32 {
    if !pos.assigned {
        return 0;
    }
    let blended;
    let p = if pos.hidden_n < 8 {
        blended = p0.blended(pos.hidden_n);
        &blended
    } else {
        p0
    };
    let w = &p.w;
    let pv = piece_values(pos, p);
    let mut score = [0i32; 2];

    // 子力
    let mut total = [0i32; 2];
    for c in 0..2usize {
        for t in 0..7usize {
            let n = pos.alive[c * 7 + t] as i32;
            let h = pos.pool[c * 7 + t] as i32;
            total[c] += n;
            score[c] += (n - h) * pv.v[c][t] + h * pv.v[c][t] * w[idx::HIDDEN_PCT] / 100;
        }
    }
    // 棋子越少，每顆越珍貴（避免被清空）
    for c in 0..2usize {
        if total[c] <= 3 {
            score[c] -= w[idx::LAST_PIECES] * (4 - total[c]);
        }
    }

    // 已翻開棋子的位置關係
    let mut sqs: [[u8; 16]; 2] = [[0; 16]; 2];
    let mut ns = [0usize; 2];
    for s in 0..NSQ {
        let q = pos.cells[s];
        if q < 14 {
            let c = color_of(q) as usize;
            if ns[c] < 16 {
                sqs[c][ns[c]] = s as u8;
                ns[c] += 1;
            }
        }
    }
    let hidden = pos.hidden_n as i32;
    // 翻子越少，追殺越重要
    let chase_w = w[idx::CHASE] + w[idx::LATE_CHASE] * (32 - hidden) / 32;

    // 每種子是否有敵方（非炮）天敵存活
    let mut hunted = [[false; 7]; 2];
    for c in 0..2usize {
        let ob = (1 - c) * 7;
        for t in 0..7usize {
            for u in 0..7usize {
                if u != CANNON as usize && ADJ_CAP[u][t] && pos.alive[ob + u] > 0 {
                    hunted[c][t] = true;
                }
            }
        }
    }
    let late = 32 - hidden;
    let am = pos.attack_maps();
    for c in 0..2usize {
        let o = 1 - c;
        let mut confine = 0i32;
        let mut chase = 0i32;
        let mut danger = 0i32;
        let mut mob = 0i32;
        let mut prot = 0i32;
        let mut trapped = 0i32;
        let mut hid_near = 0i32;
        let mut par = 0i32;
        for i in 0..ns[c] {
            let s = sqs[c][i] as usize;
            let tp = type_of(pos.cells[s]) as usize;
            let myv = pv.v[c][tp];
            // 機動性與周圍
            let mut free = 0;
            let mut escape = 0;
            for &n in NEIGH[s].iter() {
                if n == 255 {
                    break;
                }
                let q = pos.cells[n as usize];
                if q == EMPTY {
                    free += 1;
                    if !am.attacked(n as usize, tp as u8, o) {
                        escape += 1;
                    }
                } else if q == HIDDEN {
                    hid_near += 1;
                } else if color_of(q) as usize == c {
                    // 有同伴且同伴能反吃攻擊者：簡化為同伴強度 >= 自己
                    let tq = type_of(q) as usize;
                    if tq <= tp && tq != CANNON as usize {
                        prot += 1;
                    }
                }
            }
            mob += free;
            if hunted[c][tp] {
                const CONF: [i32; 5] = [16, 8, 3, 1, 0];
                confine += myv * CONF[escape.min(4) as usize];
            }
            let threatened = am.attacked(s, tp as u8, o);
            if threatened && escape == 0 {
                trapped += myv;
            }
            for j in 0..ns[o] {
                let e = sqs[o][j] as usize;
                let te = type_of(pos.cells[e]) as usize;
                let d = DIST[s][e] as usize;
                let pc = ADJ_CAP[tp][te];
                let ec = ADJ_CAP[te][tp];
                if pc && !ec {
                    chase += pv.v[o][te] * NEAR[d];
                    // 一對一追殺：輪到追方時距離為奇數才追得到
                    if (d & 1 == 1) == (c == pos.side as usize) {
                        par += pv.v[o][te];
                    }
                } else if ec && !pc {
                    danger += myv * NEAR[d];
                }
            }
            if tp == CANNON as usize {
                // 炮：瞄著可吃的敵子（有炮架但未必能馬上吃）的潛力
                for d in 0..4 {
                    let ray = &RAYS[s][d];
                    let mut cnt = 0;
                    for &n in ray.iter() {
                        if n == 255 {
                            break;
                        }
                        let q = pos.cells[n as usize];
                        if q == EMPTY {
                            continue;
                        }
                        cnt += 1;
                        if cnt >= 2 && q < 14 && color_of(q) as usize == o {
                            score[c] += w[idx::CANNON_LINE] * pv.v[o][type_of(q) as usize] / 64;
                            break;
                        }
                        if cnt >= 3 {
                            break;
                        }
                    }
                }
            }
        }
        score[c] += chase * chase_w / (64 * 64);
        score[c] -= danger * w[idx::DANGER] / (64 * 64);
        score[c] += mob * w[idx::MOBILITY];
        score[c] -= confine * w[idx::CONFINE] * late / (64 * 64 * 32);
        score[c] += prot * w[idx::PROTECT];
        score[c] -= trapped * w[idx::TRAPPED] / 64;
        score[c] += hid_near * w[idx::HIDDEN_NEAR] * hidden / 32;
        if hidden <= 2 {
            score[c] += par * w[idx::PARITY] / (64 * (1 + ns[c] as i32));
        }
    }

    let side = pos.side as usize;
    let mut s = score[side] - score[1 - side] + w[idx::TEMPO];
    // 接近和棋上限時，評估值縮向 0，逼優勢方進取
    if pos.nocap > 10 {
        let keep = w[idx::NOCAP_DECAY].clamp(0, 100);
        let frac = ((pos.nocap - 10) as i32 * 100) / (DRAW_PLIES as i32 - 10);
        let factor = 100 - (100 - keep) * frac.min(100) / 100;
        s = s * factor / 100;
    }
    s.clamp(-EVAL_BOUND, EVAL_BOUND)
}

/// 翻子啟發分（給排序 / 剪枝用），從翻子方角度，單位與子力相同。
pub fn flip_heuristic(pos: &Pos, sq: usize, pv: &PieceVals, params: &Params) -> i32 {
    let me = pos.side;
    let opp = me ^ 1;
    let total = pos.pool_total() as i32;
    if total == 0 {
        return 0;
    }
    // 鄰近已翻開棋子 & 炮線
    let mut neigh: [(u8, u8); 4] = [(0, 0); 4];
    let mut nn = 0;
    for &n in NEIGH[sq].iter() {
        if n == 255 {
            break;
        }
        let q = pos.cells[n as usize];
        if q < 14 {
            neigh[nn] = (color_of(q), type_of(q));
            nn += 1;
        }
    }
    // 會被敵炮打到的（翻出己子時）/ 被我炮打到的（翻出敵子時）
    let mut cannon_by = [false; 2];
    for d in 0..4 {
        let ray = &RAYS[sq][d];
        let mut screen = false;
        for &n in ray.iter() {
            if n == 255 {
                break;
            }
            let q = pos.cells[n as usize];
            if q == EMPTY {
                continue;
            }
            if !screen {
                screen = true;
                continue;
            }
            if q < 14 && type_of(q) == CANNON {
                cannon_by[color_of(q) as usize] = true;
            }
            break;
        }
    }
    if nn == 0 && !cannon_by[0] && !cannon_by[1] {
        return 0;
    }
    // 經驗法則：翻己方仕 / 相旁邊較安全；敵兵未翻完時避免翻己方帥旁邊
    let mut prior = 0i32;
    for &(c, t) in neigh[..nn].iter() {
        if c == me {
            match t {
                ADVISOR => prior += params.w[idx::FLIP_ADV],
                ELEPHANT => prior += params.w[idx::FLIP_ELE],
                KING if pos.pool[piece(opp, PAWN) as usize] > 0 => prior -= params.w[idx::FLIP_KING],
                _ => {}
            }
        }
    }
    let mut acc = 0i32;
    for k in 0..14u8 {
        let cnt = pos.pool[k as usize] as i32;
        if cnt == 0 {
            continue;
        }
        let kc = color_of(k);
        let kt = type_of(k) as usize;
        let kval = pv.v[kc as usize][kt];
        let mut g = 0i32;
        if kc == me {
            // 翻出己子，對手先走：會不會被吃？
            let mut lost = cannon_by[opp as usize];
            let mut threat = 0;
            for &(c, t) in neigh[..nn].iter() {
                if c == opp {
                    if ADJ_CAP[t as usize][kt] {
                        lost = true;
                    }
                    if ADJ_CAP[kt][t as usize] {
                        threat = threat.max(pv.v[opp as usize][t as usize]);
                    }
                }
            }
            if lost {
                g -= kval;
            } else {
                g += threat / 3;
            }
        } else {
            // 翻出敵子，對手先走：它可能吃我旁邊的子，或逃走
            let mut best_loss = 0;
            let mut can_hit = cannon_by[me as usize];
            for &(c, t) in neigh[..nn].iter() {
                if c == me {
                    if ADJ_CAP[kt][t as usize] {
                        best_loss = best_loss.max(pv.v[me as usize][t as usize]);
                    }
                    if ADJ_CAP[t as usize][kt] {
                        can_hit = true;
                    }
                }
            }
            g -= best_loss;
            if best_loss == 0 && can_hit {
                g += kval / 4;
            }
        }
        acc += g * cnt;
    }
    acc / total + prior
}
