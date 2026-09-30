//! 自我對弈擂台：arena <games> <time_ms> "A 覆寫" "B 覆寫" [threads]
//! 覆寫格式：key=val,key=val（參數名見 eval::PARAM_NAMES，或 fr / iflips / rqflips / lmr / variety / tmul）
use cdc_engine::board::*;
use cdc_engine::eval::*;
use cdc_engine::game::*;
use cdc_engine::moe::*;
use cdc_engine::search::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Spec {
    pub params: Params,
    pub cfg: SearchConfig,
    pub variety: f64,
    pub tmul: f64,
    pub brain: bool,
    pub use_tb: bool,
}

pub fn parse_spec(s: &str) -> Spec {
    let mut sp = Spec { params: Params::default(), cfg: SearchConfig::default(), variety: 0.0, tmul: 1.0, brain: false, use_tb: true };
    for kv in s.split(',').filter(|x| !x.is_empty()) {
        let mut it = kv.splitn(2, '=');
        let k = it.next().unwrap().trim();
        let v: f64 = it.next().unwrap_or("0").trim().parse().expect("bad value");
        match k {
            "fr" => sp.cfg.flip_reduction = v as i32,
            "iflips" => sp.cfg.interior_flips = v as usize,
            "rqflips" => sp.cfg.root_quiet_flips = v as usize,
            "lmr" => sp.cfg.use_lmr = v != 0.0,
            "variety" => {
                sp.variety = v;
                sp.brain = true;
            }
            "brain" => sp.brain = v != 0.0,
            "margin" => sp.cfg.root_margin = v as i32,
            "tmul" => sp.tmul = v,
            "depth" => sp.cfg.max_depth = v as i32,
            "contempt" => sp.cfg.contempt = v as i32,
            "nmp" => sp.cfg.null_move = v != 0.0,
            "fmd" => sp.cfg.flip_min_depth = v as i32,
            "tb" => sp.use_tb = v != 0.0,
            "idstop" => sp.cfg.id_stop = v,
            _ => {
                if !sp.params.set(k, v.round() as i32) {
                    panic!("unknown key {k}");
                }
            }
        }
    }
    sp
}

struct Player {
    brain: Brain,
    use_brain: bool,
    time: f64,
}

impl Player {
    fn new(sp: &Spec, time: f64, seed: u64) -> Player {
        let mut brain = Brain::new(seed, 18);
        brain.searcher.params = sp.params.clone();
        let margin = brain.searcher.cfg.root_margin;
        brain.searcher.cfg = sp.cfg.clone();
        if sp.brain {
            brain.searcher.cfg.root_margin = margin.max(sp.cfg.root_margin);
        }
        brain.variety = sp.variety;
        brain.searcher.use_tb = sp.use_tb;
        Player { brain, use_brain: sp.brain, time: time * sp.tmul }
    }
    fn choose(&mut self, g: &Game, rng: &mut Rng) -> Move {
        if !g.pos.assigned {
            return Move::flip(rng.below(32) as u8);
        }
        if self.use_brain {
            self.brain.decide(g, self.time).mv
        } else {
            self.brain.searcher.cfg.time_ms = self.time;
            let r = self.brain.searcher.think(&g.pos, &g.history);
            if std::env::var("VS").is_ok() {
                eprintln!("  score={} depth={}", r.score, r.depth);
            }
            r.best
        }
    }
}

pub fn play_game(seed: u64, a_first: bool, time: f64, a: &Spec, b: &Spec, verbose: bool) -> (f64, usize) {
    let mut rng = Rng::new(seed);
    let mut g = Game::new(&mut rng);
    let mut pa = Player::new(a, time, seed * 3 + 1);
    let mut pb = Player::new(b, time, seed * 5 + 2);
    let mut a_color: Option<u8> = None;
    let mut turn_a = a_first;
    loop {
        let o = g.outcome();
        if o != Outcome::Ongoing || g.moves.len() > 600 {
            let s = match (o, a_color) {
                (Outcome::Win(c), Some(ac)) => {
                    if c == ac {
                        1.0
                    } else {
                        0.0
                    }
                }
                _ => 0.5,
            };
            if verbose {
                eprintln!("result for A: {s}");
            }
            if s == 0.5 && std::env::var("SHOWDRAW").is_ok() {
                let e = evaluate(&g.pos, &Params::default());
                eprintln!("DRAW nocap={} eval(stm)={} pieces={}/{} \n{}", g.pos.nocap, e, g.pos.alive_color(g.pos.side), g.pos.alive_color(g.pos.side ^ 1), g.pos.to_string_board());
            }
            return (s, g.moves.len());
        }
        let m = if turn_a { pa.choose(&g, &mut rng) } else { pb.choose(&g, &mut rng) };
        let first = !g.pos.assigned;
        let info = g.play(m);
        if first {
            let c = color_of(info);
            a_color = Some(if turn_a { c } else { c ^ 1 });
        }
        if verbose {
            eprintln!("{} {} nocap={}\n{}", if turn_a { "A" } else { "B" }, m, g.pos.nocap, g.pos.to_string_board());
        }
        turn_a = !turn_a;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let time: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50.0);
    let a = parse_spec(args.get(3).map(|s| s.as_str()).unwrap_or(""));
    let b = parse_spec(args.get(4).map(|s| s.as_str()).unwrap_or(""));
    let threads: usize = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(16);
    let seed0: u64 = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let verbose = std::env::var("V").is_ok();
    let next = Arc::new(AtomicUsize::new(0));
    let res = Arc::new(Mutex::new((0usize, 0usize, 0usize, 0usize)));
    let mut hs = vec![];
    for _ in 0..threads {
        let (next, res, a, b) = (next.clone(), res.clone(), a.clone(), b.clone());
        hs.push(std::thread::spawn(move || loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= games {
                break;
            }
            // 同一個牌局兩邊各先手一次
            let (s, len) = play_game(seed0 + (i / 2) as u64, i % 2 == 0, time, &a, &b, verbose);
            let mut r = res.lock().unwrap();
            if s == 1.0 {
                r.0 += 1
            } else if s == 0.0 {
                r.2 += 1
            } else {
                r.1 += 1
            }
            r.3 += len;
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    let r = res.lock().unwrap();
    let n = (r.0 + r.1 + r.2) as f64;
    let sc = (r.0 as f64 + 0.5 * r.1 as f64) / n;
    let elo = if sc > 0.0 && sc < 1.0 { -400.0 * (1.0 / sc - 1.0).log10() } else { f64::NAN };
    let var = (r.0 as f64 * (1.0 - sc).powi(2) + r.1 as f64 * (0.5 - sc).powi(2) + r.2 as f64 * sc.powi(2)) / n;
    let se = (var / n).sqrt();
    println!(
        "A W/D/L = {}/{}/{}  score={:.3}±{:.3} elo={:.0} avglen={:.0}",
        r.0,
        r.1,
        r.2,
        sc,
        1.96 * se,
        elo,
        r.3 as f64 / n
    );
}
