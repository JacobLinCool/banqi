//! 與外部引擎對弈：match <對手> <games> <time_ms> [threads] ["我方覆寫"]
//!
//! 對手：
//!   misty          MistyBanqi（UCI），每步 `go movetime <time_ms>`；執行檔路徑取自 MISTY_BIN
//!   misty:nodes=N  MistyBanqi 改用固定節點數 `go nodes N`，不受機器負載影響
//!   george         George0828Zhang 的 NTU TCG hw3（CDC 協定），每步時間由 PLY_MS 環境變數固定；路徑取自 GEORGE_BIN
//!
//! 每副牌交換先後手各下一盤。終局（吃光 / 無子可動、60 步無進展、三次重複）一律由本工具依我方規則判定，
//! 不採用對手引擎自己的判定。對手回傳不合法著法或當掉時該盤記為錯誤，不計入戰績。
//! 環境變數：SEED 起始種子（預設 1000）、REC 目錄（每盤寫一份棋譜）。
#[path = "arena.rs"]
#[allow(dead_code)]
mod arena;

use arena::{parse_spec, Player, Spec};
use cdc_engine::board::*;
use cdc_engine::game::*;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ───────────────────────── 外部引擎行程 ─────────────────────────

struct Proc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Proc {
    fn spawn(bin: &str, envs: &[(&str, String)]) -> Result<Proc, String> {
        let mut cmd = Command::new(bin);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| format!("無法啟動 {bin}: {e}"))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Ok(Proc { child, stdin, stdout })
    }
    fn send(&mut self, line: &str) -> Result<(), String> {
        writeln!(self.stdin, "{line}").and_then(|_| self.stdin.flush()).map_err(|e| format!("寫入失敗: {e}"))
    }
    /// 讀到第一個符合條件的行
    fn read_until(&mut self, pred: impl Fn(&str) -> bool) -> Result<String, String> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.stdout.read_line(&mut line).map_err(|e| format!("讀取失敗: {e}"))? == 0 {
                return Err("對手行程已結束".into());
            }
            let l = line.trim_end();
            if pred(l) {
                return Ok(l.to_string());
            }
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

trait Opponent {
    /// 雙方每走一步（含對手自己的）都會呼叫，g 為走完後的對局
    fn observe(&mut self, g: &Game, m: Move, info: u8) -> Result<(), String>;
    fn choose(&mut self, g: &Game) -> Result<Move, String>;
}

// ───────────────────────── MistyBanqi（UCI）─────────────────────────

const MISTY_ROLES: &[u8; 7] = b"GAERHCS";

/// MistyBanqi 的蓋牌 FEN：由上（第 4 列）往下，蓋牌 `X`，紅大寫黑小寫
fn misty_fen(pos: &Pos) -> String {
    let mut board = String::new();
    for r in (0..ROWS).rev() {
        let mut empty = 0;
        for c in 0..COLS {
            let p = pos.cells[r * COLS + c];
            if p == EMPTY {
                empty += 1;
                continue;
            }
            if empty > 0 {
                board.push_str(&empty.to_string());
                empty = 0;
            }
            board.push(if p == HIDDEN { 'X' } else { misty_letter(p) });
        }
        if empty > 0 {
            board.push_str(&empty.to_string());
        }
        if r > 0 {
            board.push('/');
        }
    }
    let turn = if !pos.assigned { "-" } else if pos.side == RED { "r" } else { "b" };
    let mut pool = String::new();
    for p in 0..14u8 {
        if pos.pool[p as usize] > 0 {
            pool.push(misty_letter(p));
            pool.push_str(&pos.pool[p as usize].to_string());
        }
    }
    if pool.is_empty() {
        pool.push('-');
    }
    format!("{board} {turn} {pool} {} 1", pos.nocap)
}

fn misty_letter(p: u8) -> char {
    let l = MISTY_ROLES[type_of(p) as usize] as char;
    if color_of(p) == RED {
        l
    } else {
        l.to_ascii_lowercase()
    }
}

/// UCI 格子：file a–h + rank 0–3，索引與我方相同
fn misty_sq(s: u8) -> String {
    format!("{}{}", (b'a' + s % COLS as u8) as char, s / COLS as u8)
}

fn misty_parse_sq(b: &[u8]) -> Option<u8> {
    let f = b[0].checked_sub(b'a')?;
    let r = b[1].checked_sub(b'0')?;
    (f < COLS as u8 && r < ROWS as u8).then_some(r * COLS as u8 + f)
}

struct Misty {
    proc: Proc,
    go: String,
    /// 最後一次翻子或吃子後的局面，以及之後的安靜步（讓對手看得到重複局面）
    start: Pos,
    window: Vec<Move>,
}

impl Misty {
    fn new(bin: &str, go: String) -> Result<Misty, String> {
        let mut proc = Proc::spawn(bin, &[])?;
        proc.send("uci")?;
        proc.read_until(|l| l == "uciok")?;
        proc.send("ucinewgame")?;
        Ok(Misty { proc, go, start: Pos::new(), window: vec![] })
    }
}

impl Opponent for Misty {
    fn observe(&mut self, g: &Game, m: Move, info: u8) -> Result<(), String> {
        if m.is_flip() || info < HIDDEN {
            self.start = g.pos.clone();
            self.window.clear();
        } else {
            self.window.push(m);
        }
        Ok(())
    }
    fn choose(&mut self, _g: &Game) -> Result<Move, String> {
        let mut cmd = format!("position fen {}", misty_fen(&self.start));
        if !self.window.is_empty() {
            cmd.push_str(" moves");
            for m in &self.window {
                cmd.push_str(&format!(" {}{}", misty_sq(m.from), misty_sq(m.to)));
            }
        }
        self.proc.send(&cmd)?;
        self.proc.send(&self.go)?;
        let line = self.proc.read_until(|l| l.starts_with("bestmove"))?;
        let mv = line.split_whitespace().nth(1).unwrap_or("").as_bytes();
        if mv.len() != 4 {
            return Err(format!("無法解析：{line}"));
        }
        match (misty_parse_sq(&mv[0..2]), misty_parse_sq(&mv[2..4])) {
            (Some(from), Some(to)) => Ok(Move { from, to }),
            _ => Err(format!("無法解析：{line}")),
        }
    }
}

// ───────────────────────── George hw3（CDC 協定）─────────────────────────

const GEORGE_ROLES: &[u8; 7] = b"KGMRNCP";

/// 對方棋盤是 4 行 × 8 列（a–d × 1–8），轉置對應：我方列 r → 行 a+r，我方行 c → 列 c+1。相鄰與直線關係不變。
fn george_sq(s: u8) -> String {
    format!("{}{}", (b'a' + s / COLS as u8) as char, s % COLS as u8 + 1)
}

fn george_parse_sq(t: &str) -> Option<u8> {
    let b = t.as_bytes();
    if b.len() != 2 {
        return None;
    }
    let r = b[0].checked_sub(b'a')?;
    let c = b[1].checked_sub(b'1')?;
    (r < ROWS as u8 && c < COLS as u8).then_some(r * COLS as u8 + c)
}

struct George {
    proc: Proc,
}

impl George {
    fn new(bin: &str, time_ms: f64) -> Result<George, String> {
        let mut g = George { proc: Proc::spawn(bin, &[("PLY_MS", format!("{time_ms}"))])? };
        g.cmd(7, "reset_board")?;
        Ok(g)
    }
    /// 指令編號必須等於指令種類（對方以編號分派）；回覆為 `=<id> ...`，其餘輸出略過
    fn cmd(&mut self, id: u32, body: &str) -> Result<String, String> {
        self.proc.send(&format!("{id} {body}"))?;
        let ok = format!("={id}");
        let bad = format!("?{id}");
        let head = |l: &str, p: &str| l == p || l.starts_with(&format!("{p} "));
        let line = self.proc.read_until(|l| head(l, &ok) || head(l, &bad))?;
        if line.starts_with('?') {
            return Err(format!("指令失敗：{id} {body} → {line}"));
        }
        Ok(line[ok.len()..].trim().to_string())
    }
}

impl Opponent for George {
    fn observe(&mut self, _g: &Game, m: Move, info: u8) -> Result<(), String> {
        if m.is_flip() {
            let l = GEORGE_ROLES[type_of(info) as usize] as char;
            let l = if color_of(info) == RED { l } else { l.to_ascii_lowercase() };
            self.cmd(11, &format!("flip {} {l}", george_sq(m.from)))?;
        } else {
            self.cmd(10, &format!("move {} {}", george_sq(m.from), george_sq(m.to)))?;
        }
        Ok(())
    }
    fn choose(&mut self, g: &Game) -> Result<Move, String> {
        let color = if !g.pos.assigned { "unknown" } else if g.pos.side == RED { "red" } else { "black" };
        let r = self.cmd(12, &format!("genmove {color}"))?;
        let mut it = r.split_whitespace();
        match (it.next().and_then(george_parse_sq), it.next().and_then(george_parse_sq)) {
            (Some(from), Some(to)) => Ok(Move { from, to }),
            _ => Err(format!("無法解析：{r}")),
        }
    }
}

// ───────────────────────── 對局 ─────────────────────────

#[derive(Clone)]
struct Config {
    opp: String,
    time: f64,
    ours: Spec,
    misty_bin: Option<String>,
    george_bin: Option<String>,
}

fn spawn_opponent(cfg: &Config) -> Result<Box<dyn Opponent>, String> {
    let (name, arg) = cfg.opp.split_once(':').unwrap_or((cfg.opp.as_str(), ""));
    match name {
        "misty" => {
            let bin = cfg.misty_bin.as_deref().ok_or("請設定 MISTY_BIN")?;
            let go = match arg.strip_prefix("nodes=") {
                Some(n) => format!("go nodes {n}"),
                None => format!("go movetime {}", cfg.time as u64),
            };
            Ok(Box::new(Misty::new(bin, go)?))
        }
        "george" => {
            let bin = cfg.george_bin.as_deref().ok_or("請設定 GEORGE_BIN")?;
            Ok(Box::new(George::new(bin, cfg.time)?))
        }
        _ => Err(format!("未知的對手 {}", cfg.opp)),
    }
}

struct GameResult {
    /// 我方得分：1 勝、0.5 和、0 負
    score: f64,
    plies: usize,
    reason: &'static str,
    record: String,
}

fn play_game(cfg: &Config, seed: u64, ours_first: bool) -> Result<GameResult, String> {
    let mut rng = Rng::new(seed);
    let mut g = Game::new(&mut rng);
    let mut opp = spawn_opponent(cfg)?;
    let mut ours = Player::new(&cfg.ours, cfg.time, seed * 3 + 1, 22);
    let mut our_color: Option<u8> = None;
    let mut our_turn = ours_first;
    let mut record = String::new();
    loop {
        let o = g.outcome();
        if o != Outcome::Ongoing || g.moves.len() >= 1000 {
            let (score, reason) = match (o, our_color) {
                (Outcome::Win(c), Some(oc)) => {
                    let reason = if g.pos.alive_color(g.pos.side) == 0 { "吃光" } else { "無子可動" };
                    (if c == oc { 1.0 } else { 0.0 }, reason)
                }
                (Outcome::Draw, _) => (0.5, if g.pos.nocap >= DRAW_PLIES { "60 步無進展" } else { "三次重複" }),
                _ => (0.5, "超過 1000 步"),
            };
            return Ok(GameResult { score, plies: g.moves.len(), reason, record });
        }
        let m = if our_turn { ours.choose(&g, &mut rng) } else { opp.choose(&g)? };
        if !g.legal().contains(&m) {
            return Err(format!("{}回傳不合法著法 {m}\n{}", if our_turn { "我方" } else { "對手" }, g.pos.to_string_board()));
        }
        let first = !g.pos.assigned;
        let info = g.play(m);
        if first {
            let c = color_of(info);
            our_color = Some(if our_turn { c } else { c ^ 1 });
        }
        let what = if m.is_flip() {
            format!(" 翻 {}", PIECE_CHARS[info as usize])
        } else if info < HIDDEN {
            format!(" 吃 {}", PIECE_CHARS[info as usize])
        } else {
            String::new()
        };
        record.push_str(&format!("{:>3} {} {m}{what}\n", g.moves.len(), if our_turn { "我方" } else { "對手" }));
        opp.observe(&g, m, info)?;
        our_turn = !our_turn;
    }
}

fn elo(score: f64) -> f64 {
    let s = score.clamp(1e-6, 1.0 - 1e-6);
    -400.0 * (1.0 / s - 1.0).log10()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("用法：match <misty|misty:nodes=N|george> <games> <time_ms> [threads] [\"我方覆寫\"]");
        std::process::exit(2);
    }
    let cfg = Config {
        opp: args[1].clone(),
        time: args[3].parse().expect("time_ms"),
        ours: parse_spec(args.get(5).map(|s| s.as_str()).unwrap_or("")),
        misty_bin: std::env::var("MISTY_BIN").ok(),
        george_bin: std::env::var("GEORGE_BIN").ok(),
    };
    let games: usize = args[2].parse().expect("games");
    let threads: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(4);
    let seed0: u64 = std::env::var("SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1000);
    let rec_dir = std::env::var("REC").ok();
    if let Some(d) = &rec_dir {
        std::fs::create_dir_all(d).expect("REC 目錄");
    }
    // 先確認對手能啟動
    if let Err(e) = spawn_opponent(&cfg) {
        eprintln!("{e}");
        std::process::exit(1);
    }

    let next = Arc::new(AtomicUsize::new(0));
    // 勝、和、負、錯誤、總步數
    let res = Arc::new(Mutex::new((0usize, 0usize, 0usize, 0usize, 0usize)));
    let mut hs = vec![];
    for _ in 0..threads {
        let (next, res, cfg, rec_dir) = (next.clone(), res.clone(), cfg.clone(), rec_dir.clone());
        hs.push(std::thread::spawn(move || loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= games {
                break;
            }
            // 同一副牌兩邊各先手一次
            let seed = seed0 + (i / 2) as u64;
            let ours_first = i % 2 == 0;
            let r = play_game(&cfg, seed, ours_first);
            let mut t = res.lock().unwrap();
            match &r {
                Ok(gr) => {
                    match gr.score {
                        s if s == 1.0 => t.0 += 1,
                        s if s == 0.0 => t.2 += 1,
                        _ => t.1 += 1,
                    }
                    t.4 += gr.plies;
                    let n = t.0 + t.1 + t.2;
                    let sc = (t.0 as f64 + 0.5 * t.1 as f64) / n as f64;
                    eprintln!(
                        "#{i:<4} seed={seed} {} {} {:>3} 步（{}）  累計 {}/{}/{} score={sc:.3}",
                        if ours_first { "我先" } else { "對先" },
                        ["負", "和", "勝"][(gr.score * 2.0) as usize],
                        gr.plies,
                        gr.reason,
                        t.0,
                        t.1,
                        t.2
                    );
                }
                Err(e) => {
                    t.3 += 1;
                    eprintln!("#{i:<4} seed={seed} 錯誤：{e}");
                }
            }
            drop(t);
            if let (Some(d), Ok(gr)) = (&rec_dir, &r) {
                let head = format!(
                    "對手 {}，每步 {}ms，seed {seed}，{}，結果（我方）{}，{}\n",
                    cfg.opp,
                    cfg.time,
                    if ours_first { "我方先手" } else { "對手先手" },
                    gr.score,
                    gr.reason
                );
                let _ = std::fs::write(format!("{d}/game{i:04}.txt"), head + &gr.record);
            }
        }));
    }
    for h in hs {
        h.join().unwrap();
    }
    let r = res.lock().unwrap();
    let n = (r.0 + r.1 + r.2) as f64;
    if n == 0.0 {
        println!("沒有完成任何對局（錯誤 {}）", r.3);
        return;
    }
    let sc = (r.0 as f64 + 0.5 * r.1 as f64) / n;
    let var = (r.0 as f64 * (1.0 - sc).powi(2) + r.1 as f64 * (0.5 - sc).powi(2) + r.2 as f64 * sc.powi(2)) / n;
    let ci = 1.96 * (var / n).sqrt();
    println!(
        "vs {} @ {}ms：我方 W/D/L = {}/{}/{}  score={:.3}±{:.3}  elo={:+.0} [{:+.0}, {:+.0}]  avglen={:.0}  錯誤={}",
        cfg.opp,
        cfg.time,
        r.0,
        r.1,
        r.2,
        sc,
        ci,
        elo(sc),
        elo(sc - ci),
        elo(sc + ci),
        r.4 as f64 / n,
        r.3
    );
}
