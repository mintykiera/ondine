use cozy_chess::Board;
use nnue_rs::Network;
use polyglot_book_rs::PolyglotBook;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::Duration;

use super::move_fmt::format_uci_move;
use crate::transposition::TranspositionTable;

pub(crate) fn run_tactics(tt: &TranspositionTable, network: &Network) {
    let positions = vec![
        ("M1 Back Rank", "6k1/5ppp/8/8/8/8/8/4R1K1 w - - 0 1", "e1e8"),
        (
            "M1 Scholar",
            "r1bqkb1r/pppp1ppp/2n2n2/4p2Q/2B1P3/8/PPPP1PPP/RNB1K1NR w KQkq - 4 4",
            "h5f7",
        ),
        (
            "M2 Discovered",
            "r5rk/5p1p/5R2/4B3/8/8/7P/7K w - - 0 1",
            "f6a6",
        ),
        ("Hanging Queen", "4k3/8/8/3q4/8/8/3R4/4K3 w - - 0 1", "d2d5"),
        ("Knight Fork", "8/3k1q2/8/8/8/3N4/8/4K3 w - - 0 1", "d3e5"),
        ("Rook Pin", "4k3/4q3/8/8/8/8/P7/4R1K1 w - - 0 1", "e1e7"),
        ("Bishop Skewer", "7q/8/8/4k3/8/8/8/2B3K1 w - - 0 1", "c1b2"),
        ("Pawn Fork", "4k3/8/8/8/3p4/2N1R3/8/4K3 b - - 0 1", "d4e3"),
    ];
    let mut passed = 0;
    for (name, fen, expected) in &positions {
        let b: Board = fen.parse().unwrap();

        let stop_flag = Arc::new(AtomicBool::new(false));
        let is_pondering = Arc::new(AtomicBool::new(false));
        let time_limit_ms = Arc::new(AtomicU64::new(1000));

        tt.new_search();
        println!("Testing {}...", name);
        let mut hist = Vec::new();
        let no_book: Option<PolyglotBook> = None;
        let shared = crate::search::SharedHistory::new();
        let (best, _, _) = crate::search::get_best_move(
            &b,
            Duration::from_millis(1000),
            Some(Duration::from_millis(1000)),
            None,
            &tt,
            &shared,
            stop_flag,
            is_pondering,
            time_limit_ms,
            true,
            0,
            network,
            &mut hist,
            &no_book,
            &None,
        );

        if let Some(m) = best {
            let best_str = format_uci_move(&b, m);
            if best_str == *expected {
                println!("PASS: {} -> found {} as expected", name, best_str);
                passed += 1;
            } else {
                println!(
                    "FAIL: {} -> expected {}, found {}",
                    name, expected, best_str
                );
            }
        } else {
            println!("FAIL: {} -> found no move", name);
        }
        println!("---");
    }
    println!("Tactics score: {}/{}", passed, positions.len());
}
