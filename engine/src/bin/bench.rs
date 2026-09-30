use cdc_engine::board::*;
use cdc_engine::eval::*;
use cdc_engine::game::*;
use cdc_engine::search::*;

fn main() {
    let t: f64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2000.0);
    for seed in [3u64, 9] {
        let mut rng = Rng::new(seed);
        let mut g = Game::new(&mut rng);
        let mut fast = Searcher::new(Params::default(), SearchConfig { time_ms: 15.0, ..Default::default() }, 18);
        let mut slow = Searcher::new(Params::default(), SearchConfig { time_ms: t, ..Default::default() }, 22);
        g.play(Move::flip(rng.below(32) as u8));
        while g.outcome() == Outcome::Ongoing && g.moves.len() < 200 {
            if [10usize, 40, 80, 120, 160].contains(&g.moves.len()) {
                let r = slow.think(&g.pos, &g.history);
                println!("seed {seed} ply {:3} hidden {:2} depth {:2} nodes {:9} nps {:.1}M score {} best {}", g.moves.len(), g.pos.hidden_n, r.depth, r.nodes, r.nodes as f64 / r.time_ms / 1000.0, r.score, r.best);
            }
            let m = fast.think(&g.pos, &g.history).best;
            g.play(m);
        }
    }
}
