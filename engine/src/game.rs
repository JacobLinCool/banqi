//! 真實對局（持有蓋牌下的真實棋子，引擎看不到）。

use crate::board::*;

#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // splitmix64 打散種子，避免相鄰種子產生相同序列
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        Rng(z | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Ongoing,
    Win(u8), // 勝方顏色
    Draw,
}

#[derive(Clone)]
pub struct Game {
    pub layout: [u8; NSQ],
    pub pos: Pos,
    pub history: Vec<u64>,
    pub moves: Vec<(Move, u8)>, // 著法與（翻子時）翻出的棋子 / 吃到的棋子
    pub captured: [u8; 14],
}

impl Game {
    pub fn new(rng: &mut Rng) -> Game {
        let mut bag: Vec<u8> = Vec::with_capacity(32);
        for c in 0..2u8 {
            for t in 0..7u8 {
                for _ in 0..INIT_COUNT[t as usize] {
                    bag.push(piece(c, t));
                }
            }
        }
        for i in (1..bag.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            bag.swap(i, j);
        }
        let mut layout = [0u8; NSQ];
        layout.copy_from_slice(&bag);
        let pos = Pos::new();
        let h = pos.hash;
        Game { layout, pos, history: vec![h], moves: vec![], captured: [0; 14] }
    }

    pub fn legal(&self) -> Vec<Move> {
        let mut ml = MoveList::new();
        self.pos.gen_moves(&mut ml);
        ml.as_slice().to_vec()
    }

    /// 執行著法；回傳翻出 / 吃到的棋子（無則 EMPTY）
    pub fn play(&mut self, m: Move) -> u8 {
        let info;
        if m.is_flip() {
            let p = self.layout[m.from as usize];
            self.pos = self.pos.make_flip(m.from, p);
            info = p;
        } else {
            let v = self.pos.cells[m.to as usize];
            if v < 14 {
                self.captured[v as usize] += 1;
            }
            self.pos = self.pos.make_move(m);
            info = v;
        }
        self.history.push(self.pos.hash);
        self.moves.push((m, info));
        info
    }

    pub fn repetition_count(&self) -> usize {
        let h = self.pos.hash;
        let lim = (self.pos.nocap as usize + 1).min(self.history.len());
        self.history[self.history.len() - lim..].iter().filter(|&&x| x == h).count()
    }

    pub fn outcome(&self) -> Outcome {
        let pos = &self.pos;
        if !pos.assigned {
            return Outcome::Ongoing;
        }
        if pos.alive_color(pos.side) == 0 || !pos.has_moves() {
            return Outcome::Win(pos.side ^ 1);
        }
        if pos.nocap >= DRAW_PLIES {
            return Outcome::Draw;
        }
        if self.repetition_count() >= 3 {
            return Outcome::Draw;
        }
        Outcome::Ongoing
    }
}
