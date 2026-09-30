//! 非同步 SPSA 自我對弈調參：spsa <iterations> <time_ms> [threads] [start_spec]
//! 每回合以 θ+cΔ 與 θ−cΔ 對下一對（同牌局、交換先手）棋局，依勝負更新 θ。
use cdc_engine::board::*;
use cdc_engine::eval::*;
use cdc_engine::game::*;
use cdc_engine::search::*;
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const TUNE: [&str; 43] = [
    "chase", "danger", "mobility", "tempo", "protect", "trapped", "late_chase", "nocap_decay", "last_pieces",
    "cannon_line", "hidden_near", "obs_top", "obs_ratio", "obs_cannon", "obs_king", "parity", "flip_adv", "flip_ele",
    "flip_king", "confine",
    // 殘局專家
    "e.chase", "e.danger", "e.mobility", "e.tempo", "e.protect", "e.trapped", "e.nocap_decay", "e.last_pieces",
    "e.cannon_line", "e.obs_ratio", "e.obs_cannon", "e.parity", "e.confine",
    // 兵種加成、蓋牌價值
    "add_king", "add_advisor", "add_elephant", "add_chariot", "add_horse", "add_cannon", "add_pawn", "hidden_pct",
    "e.obs_king", "e.hidden_pct",
];

fn play(seed: u64, plus_first: bool, time: f64, pp: &Params, pm: &Params) -> f64 {
    let mut rng = Rng::new(seed);
    let mut g = Game::new(&mut rng);
    let cfg = SearchConfig { time_ms: time, ..Default::default() };
    let mut sp = Searcher::new(pp.clone(), cfg.clone(), 17);
    let mut sm = Searcher::new(pm.clone(), cfg, 17);
    let mut plus_color = None;
    let mut turn_plus = plus_first;
    loop {
        let o = g.outcome();
        if o != Outcome::Ongoing || g.moves.len() > 500 {
            return match (o, plus_color) {
                (Outcome::Win(c), Some(pc)) => {
                    if c == pc {
                        1.0
                    } else {
                        -1.0
                    }
                }
                _ => 0.0,
            };
        }
        let m = if !g.pos.assigned {
            Move::flip(rng.below(32) as u8)
        } else if turn_plus {
            sp.think(&g.pos, &g.history).best
        } else {
            sm.think(&g.pos, &g.history).best
        };
        let first = !g.pos.assigned;
        let info = g.play(m);
        if first {
            let c = color_of(info);
            plus_color = Some(if turn_plus { c } else { c ^ 1 });
        }
        turn_plus = !turn_plus;
    }
}

fn to_params(base: &Params, theta: &[f64]) -> Params {
    let mut p = base.clone();
    for (i, name) in TUNE.iter().enumerate() {
        assert!(p.set(name, theta[i].round() as i32));
    }
    p
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let iters: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let time: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let threads: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(16);
    let mut base = Params::default();
    if let Some(spec) = args.get(4) {
        // 未指定殘局版本時，殘局專家沿用一般值
        let mut explicit_e = vec![];
        for kv in spec.split(',').filter(|x| !x.is_empty()) {
            let mut it = kv.splitn(2, '=');
            let k = it.next().unwrap();
            let v: f64 = it.next().unwrap().parse().unwrap();
            assert!(base.set(k, v.round() as i32), "unknown {k}");
            if k.starts_with("e.") {
                explicit_e.push(k.to_string());
            }
        }
        for kv in spec.split(',').filter(|x| !x.is_empty()) {
            let k = kv.split('=').next().unwrap();
            let ek = format!("e.{k}");
            if !k.starts_with("e.") && !explicit_e.contains(&ek) {
                let v = base.get(k).unwrap();
                base.set(&ek, v);
            }
        }
    }
    let theta0: Vec<f64> = TUNE.iter().map(|n| base.get(n).unwrap() as f64).collect();
    let cs: Vec<f64> = TUNE
        .iter()
        .zip(&theta0)
        .map(|(n, &v)| if n.starts_with("add_") { 20.0 } else { (v.abs() * 0.15).max(3.0) })
        .collect();
    let theta = Arc::new(Mutex::new(theta0));
    let next = Arc::new(AtomicUsize::new(0));
    let score_sum = Arc::new(Mutex::new((0.0f64, 0usize)));
    let r_lr = 0.006f64;
    let mut hs = vec![];
    for t in 0..threads {
        let (theta, next, cs, base, score_sum) = (theta.clone(), next.clone(), cs.clone(), base.clone(), score_sum.clone());
        hs.push(std::thread::spawn(move || {
            let mut rng = Rng::new(0xABCDEF + t as u64 * 7919);
            loop {
                let k = next.fetch_add(1, Ordering::SeqCst);
                if k >= iters {
                    break;
                }
                let th = theta.lock().unwrap().clone();
                let delta: Vec<f64> = (0..TUNE.len()).map(|_| if rng.below(2) == 0 { -1.0 } else { 1.0 }).collect();
                let tp: Vec<f64> = th.iter().zip(&delta).zip(&cs).map(|((v, d), c)| v + c * d).collect();
                let tm: Vec<f64> = th.iter().zip(&delta).zip(&cs).map(|((v, d), c)| v - c * d).collect();
                let pp = to_params(&base, &tp);
                let pm = to_params(&base, &tm);
                let seed = 777_000 + k as u64;
                let r = play(seed, true, time, &pp, &pm) + play(seed, false, time, &pp, &pm);
                {
                    let mut ss = score_sum.lock().unwrap();
                    ss.0 += r.abs();
                    ss.1 += 1;
                }
                if r != 0.0 {
                    // 學習率隨時間緩降
                    let lr = r_lr * (1.0 - 0.5 * k as f64 / iters as f64);
                    let mut th = theta.lock().unwrap();
                    for i in 0..TUNE.len() {
                        th[i] += lr * cs[i] * r * delta[i];
                        if th[i] < 0.0 && !TUNE[i].starts_with("add_") {
                            th[i] = 0.0;
                        }
                    }
                    // obs_ratio / obs_king 為百分比
                    for (i, n) in TUNE.iter().enumerate() {
                        if n.ends_with("obs_ratio") || n.ends_with("obs_king") || n.ends_with("nocap_decay") {
                            th[i] = th[i].clamp(5.0, 95.0);
                        }
                    }
                }
                if k % 100 == 99 {
                    let th = theta.lock().unwrap().clone();
                    let spec: Vec<String> =
                        TUNE.iter().zip(&th).map(|(n, v)| format!("{}={}", n, v.round() as i64)).collect();
                    let line = spec.join(",");
                    println!("iter {} {}", k + 1, line);
                    let _ = std::fs::write(format!("tuned{}.txt", std::env::var("TAG").unwrap_or_default()), &line);
                    let mut f = std::fs::OpenOptions::new().create(true).append(true).open("spsa_log.txt").unwrap();
                    let _ = writeln!(f, "iter {} {}", k + 1, line);
                }
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    let th = theta.lock().unwrap().clone();
    let spec: Vec<String> = TUNE.iter().zip(&th).map(|(n, v)| format!("{}={}", n, v.round() as i64)).collect();
    println!("FINAL {}", spec.join(","));
    let _ = std::fs::write(format!("tuned{}_final.txt", std::env::var("TAG").unwrap_or_default()), spec.join(","));
}
