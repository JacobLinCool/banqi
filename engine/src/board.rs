//! 台灣暗棋規則：4x8 棋盤、32 顆蓋牌、翻子 / 走子 / 吃子。
//!
//! 棋子編碼：`color * 7 + type`，type 0..7 = 帥仕相俥傌炮兵（將士象車馬包卒）。
//! 14 = 未翻開，15 = 空格。

pub const HIDDEN: u8 = 14;
pub const EMPTY: u8 = 15;

pub const KING: u8 = 0;
pub const ADVISOR: u8 = 1;
pub const ELEPHANT: u8 = 2;
pub const CHARIOT: u8 = 3;
pub const HORSE: u8 = 4;
pub const CANNON: u8 = 5;
pub const PAWN: u8 = 6;

pub const RED: u8 = 0;
pub const BLACK: u8 = 1;

pub const ROWS: usize = 4;
pub const COLS: usize = 8;
pub const NSQ: usize = 32;

pub const INIT_COUNT: [u8; 7] = [1, 2, 2, 2, 2, 2, 5];

/// 連續多少 ply 沒有翻子或吃子即判和。
pub const DRAW_PLIES: u16 = 60;

#[inline(always)]
pub fn color_of(p: u8) -> u8 {
    (p >= 7) as u8
}
#[inline(always)]
pub fn type_of(p: u8) -> u8 {
    p % 7
}
#[inline(always)]
pub fn piece(c: u8, t: u8) -> u8 {
    c * 7 + t
}
#[inline(always)]
pub fn is_revealed(p: u8) -> bool {
    p < 14
}

/// 相鄰吃子表：`ADJ_CAP[a][v]` 代表 a 可以走一步吃掉 v。炮不能相鄰吃。
pub const ADJ_CAP: [[bool; 7]; 7] = {
    let mut t = [[false; 7]; 7];
    let mut a = 0;
    while a < 7 {
        let mut v = 0;
        while v < 7 {
            t[a][v] = if a == KING as usize {
                v != PAWN as usize
            } else if a == PAWN as usize {
                v == PAWN as usize || v == KING as usize
            } else if a == CANNON as usize {
                false
            } else {
                a <= v
            };
            v += 1;
        }
        a += 1;
    }
    t
};

/// 不論方式（含炮跳吃）a 是否可能吃 v。
pub const ANY_CAP: [[bool; 7]; 7] = {
    let mut t = ADJ_CAP;
    let mut v = 0;
    while v < 7 {
        t[CANNON as usize][v] = true;
        v += 1;
    }
    t
};

pub const NEIGH: [[u8; 4]; NSQ] = {
    let mut t = [[255u8; 4]; NSQ];
    let mut s = 0;
    while s < NSQ {
        let r = s / COLS;
        let c = s % COLS;
        let mut k = 0;
        if r > 0 {
            t[s][k] = (s - COLS) as u8;
            k += 1;
        }
        if r + 1 < ROWS {
            t[s][k] = (s + COLS) as u8;
            k += 1;
        }
        if c > 0 {
            t[s][k] = (s - 1) as u8;
            k += 1;
        }
        if c + 1 < COLS {
            t[s][k] = (s + 1) as u8;
        }
        s += 1;
    }
    t
};

/// 四個方向的射線（炮用），以 255 結尾。
pub const RAYS: [[[u8; 8]; 4]; NSQ] = {
    let mut t = [[[255u8; 8]; 4]; NSQ];
    let mut s = 0;
    while s < NSQ {
        let r = (s / COLS) as i32;
        let c = (s % COLS) as i32;
        let dirs = [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)];
        let mut d = 0;
        while d < 4 {
            let (dr, dc) = dirs[d];
            let mut rr = r + dr;
            let mut cc = c + dc;
            let mut k = 0;
            while rr >= 0 && rr < ROWS as i32 && cc >= 0 && cc < COLS as i32 {
                t[s][d][k] = (rr * COLS as i32 + cc) as u8;
                k += 1;
                rr += dr;
                cc += dc;
            }
            d += 1;
        }
        s += 1;
    }
    t
};

pub const DIST: [[u8; NSQ]; NSQ] = {
    let mut t = [[0u8; NSQ]; NSQ];
    let mut a = 0;
    while a < NSQ {
        let mut b = 0;
        while b < NSQ {
            let dr = (a / COLS) as i32 - (b / COLS) as i32;
            let dc = (a % COLS) as i32 - (b % COLS) as i32;
            let dr = if dr < 0 { -dr } else { dr };
            let dc = if dc < 0 { -dc } else { dc };
            t[a][b] = (dr + dc) as u8;
            b += 1;
        }
        a += 1;
    }
    t
};

const fn splitmix(mut x: u64) -> (u64, u64) {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    (x, z ^ (z >> 31))
}

pub const Z_CELL: [[u64; 16]; NSQ] = {
    let mut t = [[0u64; 16]; NSQ];
    let mut st = 0x1234_5678_9abc_def0u64;
    let mut s = 0;
    while s < NSQ {
        let mut p = 0;
        while p < 16 {
            let (ns, v) = splitmix(st);
            st = ns;
            t[s][p] = if p == EMPTY as usize { 0 } else { v };
            p += 1;
        }
        s += 1;
    }
    t
};

pub const Z_POOL: [[u64; 6]; 14] = {
    let mut t = [[0u64; 6]; 14];
    let mut st = 0x0fed_cba9_8765_4321u64;
    let mut p = 0;
    while p < 14 {
        let mut n = 0;
        while n < 6 {
            let (ns, v) = splitmix(st);
            st = ns;
            t[p][n] = v;
            n += 1;
        }
        p += 1;
    }
    t
};

pub const Z_SIDE: u64 = 0x5bd1_e995_7f4a_7c15;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, Hash)]
pub struct Move {
    pub from: u8,
    pub to: u8,
}

impl Move {
    pub const NONE: Move = Move { from: 255, to: 255 };
    #[inline(always)]
    pub fn flip(sq: u8) -> Move {
        Move { from: sq, to: sq }
    }
    #[inline(always)]
    pub fn is_flip(self) -> bool {
        self.from == self.to
    }
    #[inline(always)]
    pub fn is_none(self) -> bool {
        self.from == 255
    }
    pub fn encode(self) -> u16 {
        if self.is_none() {
            0xFFFF
        } else {
            self.from as u16 | ((self.to as u16) << 5)
        }
    }
    pub fn decode(v: u16) -> Move {
        if v == 0xFFFF {
            Move::NONE
        } else {
            Move { from: (v & 31) as u8, to: ((v >> 5) & 31) as u8 }
        }
    }
}

pub fn sq_name(s: u8) -> String {
    let r = s as usize / COLS;
    let c = s as usize % COLS;
    format!("{}{}", (b'a' + c as u8) as char, r + 1)
}

impl std::fmt::Display for Move {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_none() {
            write!(f, "none")
        } else if self.is_flip() {
            write!(f, "{}({})", sq_name(self.from), "flip")
        } else {
            write!(f, "{}-{}", sq_name(self.from), sq_name(self.to))
        }
    }
}

pub struct MoveList {
    pub moves: [Move; 160],
    pub len: usize,
}

impl MoveList {
    #[inline(always)]
    pub fn new() -> Self {
        MoveList { moves: [Move::NONE; 160], len: 0 }
    }
    #[inline(always)]
    pub fn push(&mut self, m: Move) {
        self.moves[self.len] = m;
        self.len += 1;
    }
    #[inline(always)]
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

/// 公開資訊的局面（引擎只看得到這個）。
#[derive(Clone, Debug)]
pub struct Pos {
    pub cells: [u8; NSQ],
    /// 尚未翻開的各子數量
    pub pool: [u8; 14],
    /// 仍存活（翻開 + 未翻）的各子數量
    pub alive: [u8; 14],
    /// 輪到的顏色（assigned 之前無意義）
    pub side: u8,
    /// 雙方顏色是否已決定（第一次翻子後）
    pub assigned: bool,
    /// 連續未翻子/未吃子 ply 數
    pub nocap: u16,
    pub hash: u64,
    pub hidden_n: u8,
}

impl Default for Pos {
    fn default() -> Self {
        Self::new()
    }
}

impl Pos {
    pub fn new() -> Pos {
        let mut pool = [0u8; 14];
        for c in 0..2u8 {
            for t in 0..7u8 {
                pool[piece(c, t) as usize] = INIT_COUNT[t as usize];
            }
        }
        let mut p = Pos {
            cells: [HIDDEN; NSQ],
            pool,
            alive: pool,
            side: RED,
            assigned: false,
            nocap: 0,
            hash: 0,
            hidden_n: 32,
        };
        p.hash = p.compute_hash();
        p
    }

    pub fn compute_hash(&self) -> u64 {
        let mut h = 0u64;
        for s in 0..NSQ {
            h ^= Z_CELL[s][self.cells[s] as usize];
        }
        for p in 0..14 {
            h ^= Z_POOL[p][self.pool[p] as usize];
        }
        if self.assigned && self.side == BLACK {
            h ^= Z_SIDE;
        }
        h
    }

    /// 從盤面 + 死子推出 pool / alive。`captured[p]` = 已被吃掉數量。
    pub fn from_parts(cells: [u8; NSQ], captured: [u8; 14], side: u8, assigned: bool, nocap: u16) -> Pos {
        let mut revealed = [0u8; 14];
        let mut hidden_n = 0u8;
        for &c in cells.iter() {
            if c < 14 {
                revealed[c as usize] += 1;
            } else if c == HIDDEN {
                hidden_n += 1;
            }
        }
        let mut pool = [0u8; 14];
        let mut alive = [0u8; 14];
        for p in 0..14 {
            let init = INIT_COUNT[p % 7];
            let a = init.saturating_sub(captured[p]);
            alive[p] = a;
            pool[p] = a.saturating_sub(revealed[p]);
        }
        let mut pos = Pos { cells, pool, alive, side, assigned, nocap, hash: 0, hidden_n };
        pos.hash = pos.compute_hash();
        pos
    }

    #[inline(always)]
    pub fn pool_total(&self) -> u32 {
        self.pool.iter().map(|&x| x as u32).sum()
    }

    pub fn alive_color(&self, c: u8) -> u32 {
        let b = (c * 7) as usize;
        self.alive[b..b + 7].iter().map(|&x| x as u32).sum()
    }

    /// 產生所有合法著法。
    pub fn gen_moves(&self, out: &mut MoveList) {
        if !self.assigned {
            for s in 0..NSQ as u8 {
                if self.cells[s as usize] == HIDDEN {
                    out.push(Move::flip(s));
                }
            }
            return;
        }
        let side = self.side;
        for s in 0..NSQ {
            let p = self.cells[s];
            if p == HIDDEN {
                out.push(Move::flip(s as u8));
            } else if p != EMPTY && color_of(p) == side {
                self.gen_piece(s, p, out, false);
            }
        }
    }

    /// 只產生吃子著法（靜態搜尋用）。
    pub fn gen_captures(&self, out: &mut MoveList) {
        if !self.assigned {
            return;
        }
        let side = self.side;
        for s in 0..NSQ {
            let p = self.cells[s];
            if p < 14 && color_of(p) == side {
                self.gen_piece(s, p, out, true);
            }
        }
    }

    #[inline(always)]
    fn gen_piece(&self, s: usize, p: u8, out: &mut MoveList, caps_only: bool) {
        let t = type_of(p) as usize;
        let side = color_of(p);
        for &n in NEIGH[s].iter() {
            if n == 255 {
                break;
            }
            let q = self.cells[n as usize];
            if q == EMPTY {
                if !caps_only {
                    out.push(Move { from: s as u8, to: n });
                }
            } else if q < 14 && color_of(q) != side && ADJ_CAP[t][type_of(q) as usize] {
                out.push(Move { from: s as u8, to: n });
            }
        }
        if t == CANNON as usize {
            for d in 0..4 {
                let ray = &RAYS[s][d];
                let mut screen = false;
                for &n in ray.iter() {
                    if n == 255 {
                        break;
                    }
                    let q = self.cells[n as usize];
                    if q == EMPTY {
                        continue;
                    }
                    if !screen {
                        screen = true;
                        continue;
                    }
                    if q < 14 && color_of(q) != side {
                        out.push(Move { from: s as u8, to: n });
                    }
                    break;
                }
            }
        }
    }

    pub fn has_moves(&self) -> bool {
        let mut ml = MoveList::new();
        self.gen_moves(&mut ml);
        ml.len > 0
    }

    /// 走一步非翻子著法，回傳新局面。
    #[inline(always)]
    pub fn make_move(&self, m: Move) -> Pos {
        debug_assert!(!m.is_flip());
        let mut p = self.clone();
        let from = m.from as usize;
        let to = m.to as usize;
        let mover = p.cells[from];
        let victim = p.cells[to];
        p.hash ^= Z_CELL[from][mover as usize] ^ Z_CELL[to][victim as usize] ^ Z_CELL[to][mover as usize];
        p.cells[to] = mover;
        p.cells[from] = EMPTY;
        if victim < 14 {
            p.alive[victim as usize] -= 1;
            p.nocap = 0;
        } else {
            p.nocap += 1;
        }
        p.side ^= 1;
        p.hash ^= Z_SIDE;
        p
    }

    /// 翻子，翻出 `result`。
    #[inline(always)]
    pub fn make_flip(&self, sq: u8, result: u8) -> Pos {
        let mut p = self.clone();
        let s = sq as usize;
        debug_assert!(p.cells[s] == HIDDEN && p.pool[result as usize] > 0);
        p.hash ^= Z_CELL[s][HIDDEN as usize] ^ Z_CELL[s][result as usize];
        let r = result as usize;
        p.hash ^= Z_POOL[r][p.pool[r] as usize];
        p.pool[r] -= 1;
        p.hash ^= Z_POOL[r][p.pool[r] as usize];
        p.cells[s] = result;
        p.hidden_n -= 1;
        p.nocap = 0;
        if !p.assigned {
            p.assigned = true;
            p.side = color_of(result) ^ 1;
            if p.side == BLACK {
                p.hash ^= Z_SIDE;
            }
        } else {
            p.side ^= 1;
            p.hash ^= Z_SIDE;
        }
        p
    }

    pub fn is_capture(&self, m: Move) -> bool {
        !m.is_flip() && self.cells[m.to as usize] < 14
    }

    /// 某格是否被 `by` 方攻擊（可被吃）。只看已翻開的棋子。
    pub fn attacked_by(&self, sq: usize, target_type: u8, by: u8) -> bool {
        for &n in NEIGH[sq].iter() {
            if n == 255 {
                break;
            }
            let q = self.cells[n as usize];
            if q < 14 && color_of(q) == by && ADJ_CAP[type_of(q) as usize][target_type as usize] {
                return true;
            }
        }
        // 炮
        for d in 0..4 {
            let ray = &RAYS[sq][d];
            let mut screen = false;
            for &n in ray.iter() {
                if n == 255 {
                    break;
                }
                let q = self.cells[n as usize];
                if q == EMPTY {
                    continue;
                }
                if !screen {
                    screen = true;
                    continue;
                }
                if q < 14 && color_of(q) == by && type_of(q) == CANNON {
                    return true;
                }
                break;
            }
        }
        false
    }

    /// 攻擊圖：`adj[c][sq]` = c 方相鄰於 sq 的棋子種類位元遮罩；`cannon[c]` = c 方炮可打到的格子位元板。
    pub fn attack_maps(&self) -> AttackMaps {
        let mut am = AttackMaps { adj: [[0u8; NSQ]; 2], cannon: [0u32; 2] };
        for s in 0..NSQ {
            let p = self.cells[s];
            if p >= 14 {
                continue;
            }
            let c = color_of(p) as usize;
            let t = type_of(p);
            for &n in NEIGH[s].iter() {
                if n == 255 {
                    break;
                }
                am.adj[c][n as usize] |= 1 << t;
            }
            if t == CANNON {
                for d in 0..4 {
                    let mut screen = false;
                    for &n in RAYS[s][d].iter() {
                        if n == 255 {
                            break;
                        }
                        let q = self.cells[n as usize];
                        if screen {
                            am.cannon[c] |= 1 << n;
                            if q != EMPTY {
                                break;
                            }
                        } else if q != EMPTY {
                            screen = true;
                        }
                    }
                }
            }
        }
        am
    }

    pub fn to_string_board(&self) -> String {
        const NAMES: [&str; 16] = [
            "帥", "仕", "相", "俥", "傌", "炮", "兵", "將", "士", "象", "車", "馬", "包", "卒", "Ｘ", "．",
        ];
        let mut s = String::new();
        for r in (0..ROWS).rev() {
            s.push_str(&format!("{} ", r + 1));
            for c in 0..COLS {
                s.push_str(NAMES[self.cells[r * COLS + c] as usize]);
            }
            s.push('\n');
        }
        s.push_str("  ａｂｃｄｅｆｇｈ\n");
        s
    }
}

pub struct AttackMaps {
    pub adj: [[u8; NSQ]; 2],
    pub cannon: [u32; 2],
}

/// `CAPTURERS[t]` = 能相鄰吃掉 t 的種類遮罩
pub const CAPTURERS: [u8; 7] = {
    let mut m = [0u8; 7];
    let mut t = 0;
    while t < 7 {
        let mut u = 0;
        while u < 7 {
            if ADJ_CAP[u][t] {
                m[t] |= 1 << u;
            }
            u += 1;
        }
        t += 1;
    }
    m
};

impl AttackMaps {
    /// sq 上的 t 種棋子是否會被 by 方吃（炮線包含空格與棋子）
    #[inline(always)]
    pub fn attacked(&self, sq: usize, t: u8, by: usize) -> bool {
        (self.adj[by][sq] & CAPTURERS[t as usize]) != 0 || (self.cannon[by] >> sq) & 1 != 0
    }
}

pub const PIECE_CHARS: [char; 16] =
    ['帥', '仕', '相', '俥', '傌', '炮', '兵', '將', '士', '象', '車', '馬', '包', '卒', 'X', '.'];
