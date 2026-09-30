//! 殘局資料庫（retrograde analysis）：全翻開、總子數 ≤ 4 的局面即時計算完美解。
//!
//! 每個「子力組合」一張表，索引 = 輪走方 + 各子位置。值記錄「距離下一次吃子（或終局）」的
//! ply 數，因為吃子會重置 60 步無進展計數：勝方必須在 60 − nocap 步內完成吃子才算真正的勝。

use crate::board::*;
use std::collections::HashMap;

pub const TB_MAX_PIECES: usize = 4;
const MAX_DIST: u8 = DRAW_PLIES as u8;

/// 0 = 和 / 無法在時限內分出勝負；1..=60 = 輪走方勝（d 步內吃子）；128+d = 輪走方負
pub type TbVal = u8;
const LOSS: u8 = 128;

pub struct Table {
    pieces: Vec<u8>,
    vals: Vec<TbVal>,
}

#[derive(Default)]
pub struct Tablebase {
    tables: HashMap<Vec<u8>, Table>,
}

#[inline]
fn index(sqs: &[u8], side: u8) -> usize {
    let mut i = 0usize;
    for &s in sqs.iter().rev() {
        i = i * 32 + s as usize;
    }
    i * 2 + side as usize
}

fn decode(mut i: usize, k: usize, sqs: &mut [u8]) -> u8 {
    let side = (i & 1) as u8;
    i >>= 1;
    for s in sqs.iter_mut().take(k) {
        *s = (i % 32) as u8;
        i /= 32;
    }
    side
}

pub enum Probe {
    Win(u8),
    Loss(u8),
    Draw,
}

impl Tablebase {
    pub fn new() -> Tablebase {
        Tablebase { tables: HashMap::new() }
    }

    pub fn eligible(pos: &Pos) -> bool {
        pos.hidden_n == 0
            && pos.assigned
            && (pos.alive_color(0) + pos.alive_color(1)) as usize <= TB_MAX_PIECES
            && pos.alive_color(0) > 0
            && pos.alive_color(1) > 0
    }

    fn key_of(pos: &Pos) -> (Vec<u8>, Vec<u8>) {
        let mut v: Vec<(u8, u8)> = vec![];
        for s in 0..NSQ {
            let p = pos.cells[s];
            if p < 14 {
                v.push((p, s as u8));
            }
        }
        v.sort();
        (v.iter().map(|x| x.0).collect(), v.iter().map(|x| x.1).collect())
    }

    pub fn has(&self, pos: &Pos) -> bool {
        Self::eligible(pos) && self.tables.contains_key(&Self::key_of(pos).0)
    }

    pub fn probe(&self, pos: &Pos) -> Option<Probe> {
        if !Self::eligible(pos) {
            return None;
        }
        let (key, sqs) = Self::key_of(pos);
        let t = self.tables.get(&key)?;
        let v = t.vals[index(&sqs, pos.side)];
        Some(if v == 0 {
            Probe::Draw
        } else if v >= LOSS {
            Probe::Loss(v - LOSS)
        } else {
            Probe::Win(v)
        })
    }

    /// 確保此局面的子力組合（與所有可達的子組合）都已計算。
    pub fn ensure(&mut self, pos: &Pos) {
        if !Self::eligible(pos) {
            return;
        }
        let (key, _) = Self::key_of(pos);
        self.build(&key);
    }

    /// 5 子時預先建立吃掉任一子後的 4 子表
    pub fn ensure_next(&mut self, pos: &Pos) {
        if pos.hidden_n != 0 || !pos.assigned {
            return;
        }
        let total = (pos.alive_color(0) + pos.alive_color(1)) as usize;
        if total != TB_MAX_PIECES + 1 {
            return;
        }
        let mut v: Vec<u8> = pos.cells.iter().copied().filter(|&p| p < 14).collect();
        v.sort();
        for i in 0..v.len() {
            let mut sub = v.clone();
            sub.remove(i);
            self.build(&sub);
        }
    }

    fn build(&mut self, key: &[u8]) {
        if self.tables.contains_key(key) {
            return;
        }
        let k = key.len();
        let has = |c: u8| key.iter().any(|&p| color_of(p) == c);
        if !has(0) || !has(1) {
            return;
        }
        // 先建所有少一子的組合
        for i in 0..k {
            let mut sub = key.to_vec();
            sub.remove(i);
            self.build(&sub);
        }
        let n = 2usize << (5 * k);
        let mut vals = vec![0u8; n];
        let mut cnt = vec![0u8; n]; // 尚未確定會輸的走子數
        let mut valid = vec![false; n];
        let mut frontier: Vec<usize> = vec![];
        let mut sqs = [0u8; 4];
        let mut pos = Pos::from_parts([EMPTY; NSQ], [0; 14], 0, true, 0);
        pos.pool = [0; 14];
        pos.hidden_n = 0;
        for i in 0..n {
            let side = decode(i, k, &mut sqs);
            // 重疊位置無效
            let mut occ = 0u32;
            let mut ok = true;
            for &s in &sqs[..k] {
                if occ >> s & 1 == 1 {
                    ok = false;
                    break;
                }
                occ |= 1 << s;
            }
            if !ok {
                continue;
            }
            valid[i] = true;
            pos.cells = [EMPTY; NSQ];
            for j in 0..k {
                pos.cells[sqs[j] as usize] = key[j];
            }
            pos.side = side;
            let mut ml = MoveList::new();
            pos.gen_moves(&mut ml);
            let mut quiet = 0u8;
            let mut win = false;
            let mut escape = false; // 吃子後和棋
            for &m in ml.as_slice() {
                let victim = pos.cells[m.to as usize];
                if victim < 14 {
                    // 吃子 → 子組合改變，計數重置
                    let vi = sqs[..k].iter().position(|&s| s == m.to).unwrap();
                    let mut sub_key = key.to_vec();
                    sub_key.remove(vi);
                    let mut sub_sqs: Vec<u8> = sqs[..k].to_vec();
                    sub_sqs.remove(vi);
                    let mi = if vi < k { sqs[..k].iter().position(|&s| s == m.from).unwrap() } else { 0 };
                    let mi = if mi > vi { mi - 1 } else { mi };
                    sub_sqs[mi] = m.to;
                    let opp = side ^ 1;
                    if !sub_key.iter().any(|&p| color_of(p) == opp) {
                        win = true;
                        break;
                    }
                    let st = &self.tables[&sub_key];
                    let cv = st.vals[index(&sub_sqs, opp)];
                    let _ = &st.pieces;
                    if cv >= LOSS {
                        win = true;
                        break;
                    } else if cv == 0 {
                        escape = true;
                    }
                } else {
                    quiet += 1;
                }
            }
            if win {
                vals[i] = 1;
                frontier.push(i);
            } else if escape {
                cnt[i] = 255; // 永遠不會輸
            } else if quiet == 0 {
                // 無子可動（或只剩會輸的吃子）→ 負
                vals[i] = LOSS + if ml.len == 0 { 0 } else { 1 };
                frontier.push(i);
            } else {
                cnt[i] = quiet;
            }
        }
        // 逆推 BFS（依距離分層）
        let mut dist: u8 = 1;
        while !frontier.is_empty() && dist < MAX_DIST {
            let mut next = vec![];
            for &i in &frontier {
                let v = vals[i];
                let side = decode(i, k, &mut sqs);
                let d = if v >= LOSS { v - LOSS } else { v };
                // 前一手是 side^1 走的：找 side^1 的子往回走一步到空格
                let prev_side = side ^ 1;
                let occ: u32 = sqs[..k].iter().fold(0, |a, &s| a | 1 << s);
                for j in 0..k {
                    if color_of(key[j]) != prev_side {
                        continue;
                    }
                    let from = sqs[j];
                    for &nb in NEIGH[from as usize].iter() {
                        if nb == 255 {
                            break;
                        }
                        if occ >> nb & 1 == 1 {
                            continue;
                        }
                        let mut ps = sqs;
                        ps[j] = nb;
                        let pi = index(&ps[..k], prev_side);
                        if !valid[pi] || vals[pi] != 0 || cnt[pi] == 255 {
                            continue;
                        }
                        if v >= LOSS {
                            // 子節點輪走方輸 → 前一局面勝
                            if d + 1 <= MAX_DIST {
                                vals[pi] = d + 1;
                                next.push(pi);
                            }
                        } else {
                            cnt[pi] -= 1;
                            if cnt[pi] == 0 && d + 1 <= MAX_DIST {
                                vals[pi] = LOSS + d + 1;
                                next.push(pi);
                            }
                        }
                    }
                }
            }
            frontier = next;
            dist += 1;
        }
        self.tables.insert(key.to_vec(), Table { pieces: key.to_vec(), vals });
    }
}
