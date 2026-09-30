//! 產生自我對弈棋譜：records <games> <time_ms> <out_dir> [threads]
use cdc_engine::board::*;
use cdc_engine::game::*;
use cdc_engine::moe::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const NAMES: [&str; 14] = ["帥", "仕", "相", "俥", "傌", "炮", "兵", "將", "士", "象", "車", "馬", "包", "卒"];

fn notation(m: Move, info: u8) -> String {
    if m.is_flip() {
        format!("{} 翻 {}", sq_name(m.from), NAMES[info as usize])
    } else if info < 14 {
        format!("{}x{} 吃 {}", sq_name(m.from), sq_name(m.to), NAMES[info as usize])
    } else {
        format!("{}-{}", sq_name(m.from), sq_name(m.to))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: usize = args[1].parse().unwrap();
    let time: f64 = args[2].parse().unwrap();
    let out = args[3].clone();
    let threads: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(4);
    std::fs::create_dir_all(&out).unwrap();
    let next = Arc::new(AtomicUsize::new(0));
    let mut hs = vec![];
    for _ in 0..threads {
        let (next, out) = (next.clone(), out.clone());
        hs.push(std::thread::spawn(move || loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= games {
                break;
            }
            let seed = 20260930 + i as u64;
            let mut rng = Rng::new(seed);
            let mut g = Game::new(&mut rng);
            let mut a = Brain::new(seed * 7 + 1, 21);
            let mut b = Brain::new(seed * 11 + 3, 21);
            let mut lines = vec![];
            let mut first_color = None;
            while g.outcome() == Outcome::Ongoing && g.moves.len() < 600 {
                let turn_a = g.moves.len() % 2 == 0;
                let d = if turn_a { a.decide(&g, time) } else { b.decide(&g, time) };
                let info = g.play(d.mv);
                if first_color.is_none() {
                    first_color = Some(color_of(info));
                }
                let ply = g.moves.len();
                lines.push(format!(
                    "{:3}. {:<5} {:<14} 評分 {:+6}  深度 {:2}",
                    ply,
                    if (ply % 2 == 1) == (first_color == Some(RED)) { "紅" } else { "黑" },
                    notation(d.mv, info),
                    d.score,
                    d.depth
                ));
            }
            let result = match g.outcome() {
                Outcome::Win(RED) => "紅勝",
                Outcome::Win(_) => "黑勝",
                _ => "和局",
            };
            let mut text = format!("# 自我對弈棋譜 #{:03}\n# 牌局種子 {seed}，每步 {time} ms，雙方皆為 MoE 引擎\n# 結果：{result}（{} 步）\n# 初始蓋牌配置（僅供覆盤，引擎對局時看不到）：\n", i + 1, g.moves.len());
            for r in (0..ROWS).rev() {
                text.push_str("#   ");
                for c in 0..COLS {
                    text.push_str(NAMES[g.layout[r * COLS + c] as usize]);
                }
                text.push('\n');
            }
            text.push('\n');
            text.push_str(&lines.join("\n"));
            text.push_str(&format!("\n\n最終盤面：\n{}", g.pos.to_string_board()));
            std::fs::write(format!("{}/game{:03}.txt", out, i + 1), text).unwrap();
            println!("game {} {}", i + 1, result);
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
}
