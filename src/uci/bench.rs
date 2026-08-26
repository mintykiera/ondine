use cozy_chess::Board;
use nnue_rs::Network;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::{Duration, Instant};
use crate::transposition::TranspositionTable;

const BENCH_POSITIONS: &[&str] = &[
    "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
    "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
    "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
    "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
];

pub(crate) fn run_bench(tt: &TranspositionTable, network: &Network, target_depth: i32) {
    let mut total_nodes: u64 = 0;
    let start_total = Instant::now();

    for (i, fen) in BENCH_POSITIONS.iter().enumerate() {
        let b: Board = fen.parse().expect("Invalid FEN in benchmark suite");
        let stop_flag = Arc::new(AtomicBool::new(false));
        let is_pondering = Arc::new(AtomicBool::new(false));
        let time_limit_ms = Arc::new(AtomicU64::new(0));

        tt.new_search();
        let mut hist = Vec::new();
        let shared = crate::search::SharedHistory::new();

        println!("Position {}/{}: {}", i + 1, BENCH_POSITIONS.len(), fen);
        let start = Instant::now();

        let (best, _, nodes) = crate::search::get_best_move(
            &b,
            Duration::from_secs(3600),
            None,
            Some(target_depth),
            tt,
            &shared,
            stop_flag,
            is_pondering,
            time_limit_ms,
            false,
            0,
            network,
            &mut hist,
            &None,
            &None,
        );

        total_nodes += nodes;
        let elapsed = start.elapsed().as_millis().max(1);
        let nps = ((nodes as u128) * 1000) / elapsed;
        println!(
            "  -> Move: {:?} | Nodes: {} | Time: {}ms | NPS: {}",
            best.map(|m| crate::uci::format_uci_move(&b, m)),
            nodes,
            elapsed,
            nps
        );
    }

    let total_time_ms = start_total.elapsed().as_millis().max(1);
    let total_nps = ((total_nodes as u128) * 1000) / total_time_ms;
    println!("===========================");
    println!("Total nodes: {}", total_nodes);
    println!("Total time:  {}ms", total_time_ms);
    println!("Overall NPS: {}", total_nps);
}