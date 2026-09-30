//! 給網頁用的 WebAssembly 介面。所有棋局狀態都留在 Rust 端。

use crate::board::*;
use crate::game::*;
use crate::moe::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct CdcGame {
    game: Game,
    seed: u64,
    /// 每個 slot 一顆獨立的大腦（各自的置換表與亂數），AI 對戰時雙方互不共享搜尋結果
    brains: Vec<Brain>,
}

fn mv_json(m: Move) -> String {
    format!("{{\"from\":{},\"to\":{},\"flip\":{}}}", m.from, m.to, m.is_flip())
}

#[wasm_bindgen]
impl CdcGame {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: f64) -> CdcGame {
        let s = seed as u64;
        let mut rng = Rng::new(s);
        let game = Game::new(&mut rng);
        CdcGame { game, seed: s, brains: vec![Self::make_brain(s, 0)] }
    }

    fn make_brain(seed: u64, slot: usize) -> Brain {
        Brain::new(seed.wrapping_mul(31).wrapping_add(7 + slot as u64 * 0x9E37_79B9), 22)
    }

    /// 取得 slot 的大腦，第一次用到時才配置（置換表約 64MB）
    fn brain(&mut self, slot: u8) -> &mut Brain {
        let slot = slot as usize;
        while self.brains.len() <= slot {
            let b = Self::make_brain(self.seed, self.brains.len());
            self.brains.push(b);
        }
        &mut self.brains[slot]
    }

    /// 32 格：0..13 棋子、14 蓋牌、15 空格。index = row*8 + col，row 0 在最下方。
    pub fn cells(&self) -> Vec<u8> {
        self.game.pos.cells.to_vec()
    }
    pub fn side(&self) -> u8 {
        self.game.pos.side
    }
    pub fn assigned(&self) -> bool {
        self.game.pos.assigned
    }
    pub fn nocap(&self) -> u16 {
        self.game.pos.nocap
    }
    pub fn draw_plies(&self) -> u16 {
        DRAW_PLIES
    }
    pub fn ply(&self) -> usize {
        self.game.moves.len()
    }
    /// 被吃掉的子數（14）
    pub fn captured(&self) -> Vec<u8> {
        self.game.captured.to_vec()
    }
    /// 尚未翻開的子數（14）
    pub fn pool(&self) -> Vec<u8> {
        self.game.pos.pool.to_vec()
    }
    /// 合法著法，編碼 from | to << 5（翻子 from == to）
    pub fn legal_moves(&self) -> Vec<u16> {
        self.game.legal().iter().map(|m| m.encode()).collect()
    }
    /// 走棋；回傳翻出或吃到的子（15 = 無），非法回傳 255
    pub fn play(&mut self, from: u8, to: u8) -> u8 {
        let m = Move { from, to };
        if !self.game.legal().contains(&m) || self.game.outcome() != Outcome::Ongoing {
            return 255;
        }
        self.game.play(m)
    }
    /// -1 進行中，0 紅勝，1 黑勝，2 和棋
    pub fn outcome(&self) -> i32 {
        match self.game.outcome() {
            Outcome::Ongoing => -1,
            Outcome::Win(c) => c as i32,
            Outcome::Draw => 2,
        }
    }
    /// 終局原因：""（進行中）、"annihilated"（子被吃光）、"blocked"（無子可動）、"no_progress"、"repetition"
    pub fn outcome_reason(&self) -> String {
        let pos = &self.game.pos;
        match self.game.outcome() {
            Outcome::Ongoing => "",
            Outcome::Win(_) => {
                if pos.alive_color(pos.side) == 0 {
                    "annihilated"
                } else {
                    "blocked"
                }
            }
            Outcome::Draw => {
                if pos.nocap >= DRAW_PLIES {
                    "no_progress"
                } else {
                    "repetition"
                }
            }
        }
        .to_string()
    }
    /// 悔棋 n 步（重新播放）
    pub fn undo(&mut self, n: usize) {
        let keep = self.game.moves.len().saturating_sub(n);
        let moves: Vec<Move> = self.game.moves[..keep].iter().map(|x| x.0).collect();
        let mut rng = Rng::new(self.seed);
        let mut g = Game::new(&mut rng);
        for m in moves {
            g.play(m);
        }
        self.game = g;
    }
    /// 著法紀錄 JSON：[{from,to,flip,info}]
    pub fn history_json(&self) -> String {
        let items: Vec<String> = self
            .game
            .moves
            .iter()
            .map(|(m, info)| format!("{{\"from\":{},\"to\":{},\"flip\":{},\"info\":{}}}", m.from, m.to, m.is_flip(), info))
            .collect();
        format!("[{}]", items.join(","))
    }
    pub fn set_variety(&mut self, slot: u8, v: f64) {
        self.brain(slot).variety = v;
    }
    /// 以 slot 的大腦思考（不落子），回傳 JSON
    pub fn think(&mut self, slot: u8, time_ms: f64) -> String {
        self.brain(slot);
        let d = self.brains[slot as usize].decide(&self.game, time_ms);
        let experts: Vec<String> =
            d.experts.iter().map(|e| format!("{{\"name\":\"{}\",\"weight\":{:.4}}}", e.name, e.weight)).collect();
        let cands: Vec<String> =
            d.candidates.iter().map(|(m, s)| format!("{{\"move\":{},\"score\":{}}}", mv_json(*m), s)).collect();
        let pv: Vec<String> = d.pv.iter().map(|m| mv_json(*m)).collect();
        format!(
            "{{\"move\":{},\"score\":{},\"depth\":{},\"nodes\":{},\"timeMs\":{:.0},\"pv\":[{}],\"experts\":[{}],\"candidates\":[{}]}}",
            mv_json(d.mv),
            d.score,
            d.depth,
            d.nodes,
            d.time_ms,
            pv.join(","),
            experts.join(","),
            cands.join(",")
        )
    }
}
