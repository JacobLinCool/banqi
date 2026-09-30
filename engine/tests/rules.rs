use cdc_engine::board::*;
use cdc_engine::game::*;
use cdc_engine::search::*;
use cdc_engine::eval::*;

fn pos_with(cells: &[(usize, u8)], side: u8) -> Pos {
    let mut c = [EMPTY; NSQ];
    for &(s, p) in cells { c[s] = p; }
    // 其他子視為已被吃
    let mut captured = [0u8; 14];
    let mut on = [0u8; 14];
    for &(_, p) in cells { if p < 14 { on[p as usize] += 1; } }
    for p in 0..14 { captured[p] = INIT_COUNT[p % 7] - on[p]; }
    Pos::from_parts(c, captured, side, true, 0)
}

fn moves(p: &Pos) -> Vec<(u8, u8)> {
    let mut ml = MoveList::new();
    p.gen_moves(&mut ml);
    ml.as_slice().iter().map(|m| (m.from, m.to)).collect()
}

#[test]
fn king_cannot_take_pawn_pawn_takes_king() {
    let p = pos_with(&[(0, piece(RED, KING)), (1, piece(BLACK, PAWN))], RED);
    assert!(!moves(&p).contains(&(0, 1)));
    let p = pos_with(&[(0, piece(RED, KING)), (1, piece(BLACK, PAWN))], BLACK);
    assert!(moves(&p).contains(&(1, 0)));
}

#[test]
fn pawn_cannot_take_cannon() {
    let p = pos_with(&[(0, piece(RED, PAWN)), (1, piece(BLACK, CANNON))], RED);
    assert!(!moves(&p).contains(&(0, 1)));
}

#[test]
fn cannon_jumps() {
    // a1 炮, b1 蓋牌（炮架）, e1 敵帥 → 可吃；c1 敵兵 在旁不可相鄰吃
    let p = pos_with(&[(0, piece(RED, CANNON)), (1, HIDDEN), (4, piece(BLACK, KING)), (8, piece(BLACK, PAWN))], RED);
    let m = moves(&p);
    assert!(m.contains(&(0, 4)));
    assert!(!m.contains(&(0, 8)));
    // 兩個炮架不可吃
    let p = pos_with(&[(0, piece(RED, CANNON)), (1, HIDDEN), (2, HIDDEN), (4, piece(BLACK, KING))], RED);
    assert!(!moves(&p).contains(&(0, 4)));
}

#[test]
fn equal_rank_capture() {
    let p = pos_with(&[(0, piece(RED, HORSE)), (1, piece(BLACK, HORSE)), (8, piece(BLACK, CHARIOT))], RED);
    let m = moves(&p);
    assert!(m.contains(&(0, 1)));
    assert!(!m.contains(&(0, 8)));
}

#[test]
fn finds_simple_win() {
    // 紅仕 a1 旁邊黑卒 b1 是黑方最後一子 → 立刻吃掉獲勝
    let p = pos_with(&[(0, piece(RED, ADVISOR)), (1, piece(BLACK, PAWN))], RED);
    let mut s = Searcher::new(Params::default(), SearchConfig { time_ms: 100.0, ..Default::default() }, 16);
    let r = s.think(&p, &[]);
    assert_eq!((r.best.from, r.best.to), (0, 1));
    assert!(r.score > MATE - 100);
}

#[test]
fn game_plays_out() {
    let mut rng = Rng::new(5);
    let mut g = Game::new(&mut rng);
    let mut n = 0;
    while g.outcome() == Outcome::Ongoing && n < 400 {
        let l = g.legal();
        let m = l[rng.below(l.len() as u64) as usize];
        g.play(m);
        assert_eq!(g.pos.hash, g.pos.compute_hash());
        n += 1;
    }
}

#[test]
fn attack_maps_match() {
    let mut rng = Rng::new(11);
    for _ in 0..50 {
        let mut g = Game::new(&mut rng);
        for _ in 0..80 {
            if g.outcome() != Outcome::Ongoing { break; }
            let l = g.legal();
            g.play(l[rng.below(l.len() as u64) as usize]);
            let am = g.pos.attack_maps();
            for s in 0..NSQ {
                let q = g.pos.cells[s];
                if q < 14 {
                    let by = (color_of(q) ^ 1) as usize;
                    assert_eq!(am.attacked(s, type_of(q), by), g.pos.attacked_by(s, type_of(q), by as u8));
                }
            }
        }
    }
}

#[test]
fn tablebase_basic() {
    use cdc_engine::tb::*;
    // 紅仕 + 紅兵 vs 黑將：帥/將 不能吃兵，兵可吃將
    let p = pos_with(&[(0, piece(RED, ADVISOR)), (9, piece(RED, PAWN)), (20, piece(BLACK, KING))], RED);
    let mut tb = Tablebase::new();
    let t0 = std::time::Instant::now();
    tb.ensure(&p);
    eprintln!("3-piece build {:?}", t0.elapsed());
    match tb.probe(&p) { Some(Probe::Win(d)) => eprintln!("win in {d}"), Some(Probe::Loss(_)) => panic!("loss?"), Some(Probe::Draw) => eprintln!("draw"), None => panic!("none") }
    // 4 子
    let p = pos_with(&[(0, piece(RED, ADVISOR)), (9, piece(RED, HORSE)), (20, piece(BLACK, CHARIOT)), (30, piece(BLACK, HORSE))], RED);
    let t0 = std::time::Instant::now();
    tb.ensure(&p);
    eprintln!("4-piece build {:?}", t0.elapsed());
    // 單子吃不到對方唯一的子（帥 vs 兵，帥不能吃兵、兵在距離偶數時追不到）
    let p = pos_with(&[(0, piece(RED, ADVISOR)), (31, piece(BLACK, ADVISOR))], RED);
    tb.ensure(&p);
    // 距離 奇/偶 由 parity 決定：a1 與 h4 距離 10（偶數）→ 紅方走時追不到 → 和或對方有利
    match tb.probe(&p) { Some(Probe::Win(_)) => panic!("should not win with even distance"), _ => {} }
    let p2 = pos_with(&[(0, piece(RED, ADVISOR)), (30, piece(BLACK, ADVISOR))], RED);
    tb.ensure(&p2);
    match tb.probe(&p2) { Some(v) => eprintln!("odd distance: {}", match v { Probe::Win(d) => format!("win {d}"), Probe::Loss(d)=>format!("loss {d}"), Probe::Draw=>"draw".into() }), None => panic!() }
}
