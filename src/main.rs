mod eval;
mod search;
mod transposition;
mod uci;

use cozy_chess::Board;
use nnue_rs::Network;
use polyglot_book_rs::PolyglotBook;
use shakmaty_syzygy::Tablebase;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::search::SharedHistory;
use uci::{
    SearchHandle, format_uci_move, get_book_path, get_memory_path, load_syzygy, parse_go_time,
    parse_total_clock, parse_uci_move,
};

fn main() {
    let board = Arc::new(Mutex::new(Board::default()));
    let mut tt = Arc::new(transposition::TranspositionTable::new(32));
    let shared_history = Arc::new(SharedHistory::new());
    let mut handle = SearchHandle::new();
    let mut num_threads = 1;
    let mut game_history: Vec<u64> = Vec::new();

    let nnue_path = uci::get_nnue_path();
    let network = Arc::new(
        Network::from_file(nnue_path.to_str().unwrap()).expect("Failed to load ondine.nnue"),
    );

    let mem_path = get_memory_path();
    let sig_path = uci::get_nnue_sig_path();
    let current_sig = uci::nnue_fingerprint(&nnue_path);
    if mem_path.exists() {
        let stored_sig = std::fs::read_to_string(&sig_path).ok();
        if stored_sig.as_deref() == current_sig.as_deref() {
            if tt.load_from_file(mem_path.to_str().unwrap()).is_ok() {
                println!("info string Transposition table memory restored from disk");
            }
        } else {
            let _ = std::fs::remove_file(&mem_path);
            let _ = std::fs::remove_file(&sig_path);
            println!("info string Network changed — stale transposition cache discarded");
        }
    }

    let book_path = get_book_path();
    let opening_book: Arc<Option<PolyglotBook>> = if book_path.exists() {
        match PolyglotBook::load(book_path.to_str().unwrap()) {
            Ok(book) => {
                println!(
                    "info string Opening book loaded from {}",
                    book_path.display()
                );
                Arc::new(Some(book))
            }
            Err(_) => {
                println!("info string Failed to load opening book, continuing without it");
                Arc::new(None)
            }
        }
    } else {
        Arc::new(None)
    };

    let default_syzygy_path = std::path::Path::new("syzygy");
    let mut syzygy: Arc<Option<Tablebase<shakmaty::Chess>>> = if default_syzygy_path.exists() {
        Arc::new(load_syzygy("syzygy"))
    } else {
        Arc::new(None)
    };

    let stdin = io::stdin();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => {
                break;
            }
        };

        let tokens: Vec<&str> = line.trim().split_whitespace().collect();

        if tokens.is_empty() {
            continue;
        }

        match tokens[0] {
            "setoption" => {
                if let Some(name_idx) = tokens.iter().position(|&r| r == "name") {
                    if let Some(value_idx) = tokens.iter().position(|&r| r == "value") {
                        if name_idx + 1 < tokens.len() {
                            let opt_name = tokens[name_idx + 1].to_lowercase();
                            if opt_name == "threads" {
                                if value_idx + 1 < tokens.len() {
                                    if let Ok(t) = tokens[value_idx + 1].parse::<usize>() {
                                        num_threads = t.max(1);
                                    }
                                }
                            } else if opt_name == "syzygypath" {
                                if value_idx + 1 < tokens.len() {
                                    let path = tokens[value_idx + 1..].join(" ");
                                    syzygy = Arc::new(load_syzygy(&path));
                                }
                            } else if opt_name == "hash" {
                                if value_idx + 1 < tokens.len() {
                                    if let Ok(h) = tokens[value_idx + 1].parse::<usize>() {
                                        let h_clamped = h.clamp(1, 65536);
                                        tt = Arc::new(transposition::TranspositionTable::new(
                                            h_clamped,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "uci" => {
                println!("id name Ondine");
                println!("id author kieraesque");
                println!("option name Threads type spin default 1 min 1 max 256");
                println!("option name SyzygyPath type string default syzygy");
                println!("option name Hash type spin default 32 min 1 max 65536");
                println!("uciok");
            }
            "isready" => {
                println!("readyok");
            }
            "ucinewgame" => {
                handle.stop_and_join();
                *board.lock().unwrap() = Board::default();
                game_history.clear();
                tt.clear();
                shared_history.clear();
            }
            "position" => {
                handle.stop_and_join();
                let mut b = board.lock().unwrap();
                game_history.clear();

                if tokens.contains(&"startpos") {
                    *b = Board::default();
                    game_history.push(b.hash());
                    if let Some(moves_idx) = tokens.iter().position(|&r| r == "moves") {
                        for m_str in &tokens[moves_idx + 1..] {
                            if let Some(m) = parse_uci_move(&b, m_str) {
                                b.play_unchecked(m);
                                game_history.push(b.hash());
                            }
                        }
                    }
                } else if let Some(fen_idx) = tokens.iter().position(|&r| r == "fen") {
                    let fen_start = fen_idx + 1;
                    if let Some(moves_idx) = tokens.iter().position(|&r| r == "moves") {
                        let fen_str = tokens[fen_start..moves_idx].join(" ");
                        if let Ok(parsed) = fen_str.parse::<Board>() {
                            *b = parsed;
                            game_history.push(b.hash());
                            for m_str in &tokens[moves_idx + 1..] {
                                if let Some(m) = parse_uci_move(&b, m_str) {
                                    b.play_unchecked(m);
                                    game_history.push(b.hash());
                                }
                            }
                        }
                    } else {
                        let fen_str = tokens[fen_start..].join(" ");
                        if let Ok(parsed) = fen_str.parse::<Board>() {
                            *b = parsed;
                            game_history.push(b.hash());
                        }
                    }
                }
            }
            "go" => {
                handle.stop_and_join();

                let ponder = tokens.contains(&"ponder");

                let board_snapshot = board.lock().unwrap().clone();
                let mut root_moves = Vec::new();
                board_snapshot.generate_moves(|moves| {
                    root_moves.extend(moves);
                    false
                });

                if root_moves.len() == 1 {
                    println!(
                        "bestmove {}",
                        format_uci_move(&board_snapshot, root_moves[0])
                    );
                    continue;
                }

                let time_limit = parse_go_time(&tokens, board_snapshot.side_to_move());
                let total_clock = parse_total_clock(&tokens, board_snapshot.side_to_move());
                let max_depth = uci::time::parse_uci_param(&tokens, "depth").map(|d| d as i32);

                let stop_flag = Arc::new(AtomicBool::new(false));
                let is_pondering = Arc::new(AtomicBool::new(ponder));
                let time_limit_ms = Arc::new(AtomicU64::new(time_limit.as_millis() as u64));

                handle.stop_flag = Arc::clone(&stop_flag);
                handle.is_pondering = Arc::clone(&is_pondering);
                handle.time_limit_ms = Arc::clone(&time_limit_ms);

                tt.new_search();

                for i in 0..num_threads {
                    let tt_clone = Arc::clone(&tt);
                    let board_clone = board_snapshot.clone();
                    let sf_clone = Arc::clone(&stop_flag);
                    let ip_clone = Arc::clone(&is_pondering);
                    let tl_clone = Arc::clone(&time_limit_ms);
                    let network_clone = Arc::clone(&network);
                    let mut history_clone = game_history.clone();
                    let book_clone = Arc::clone(&opening_book);
                    let syzygy_clone = Arc::clone(&syzygy);
                    let shared_clone = Arc::clone(&shared_history);

                    handle.threads.push(thread::spawn(move || {
                        let (best, ponder_mv, _) = search::get_best_move(
                            &board_clone,
                            time_limit,
                            total_clock,
                            max_depth,
                            &tt_clone,
                            &shared_clone,
                            sf_clone.clone(),
                            ip_clone,
                            tl_clone,
                            i == 0,
                            i,
                            &network_clone,
                            &mut history_clone,
                            &book_clone,
                            &syzygy_clone,
                        );

                        if i == 0 {
                            sf_clone.store(true, Ordering::Relaxed);

                            let final_move = best.or_else(|| {
                                let mut fallback = None;
                                board_clone.generate_moves(|moves| {
                                    if fallback.is_none() {
                                        fallback = moves.into_iter().next();
                                    }
                                    false
                                });
                                fallback
                            });

                            let best_str = final_move
                                .map_or("0000".to_string(), |m| format_uci_move(&board_clone, m));

                            if let (Some(bm), Some(pm)) = (final_move, ponder_mv) {
                                let mut after = board_clone.clone();
                                after.play_unchecked(bm);
                                println!(
                                    "bestmove {} ponder {}",
                                    best_str,
                                    format_uci_move(&after, pm)
                                );
                            } else {
                                println!("bestmove {}", best_str);
                            }
                        }
                    }));
                }
            }
            "ponderhit" => {
                if handle.is_searching() {
                    handle.is_pondering.store(false, Ordering::Relaxed);
                }
            }
            "stop" => {
                handle.stop_and_join();
            }
            "perft" => {
                let depth: i32 = tokens.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
                let b = board.lock().unwrap().clone();
                let start = Instant::now();
                let nodes = uci::perft::perft(&b, depth);
                let elapsed = start.elapsed();
                let ms = elapsed.as_millis().max(1);
                let nps = ((nodes as u128) * 1000) / ms;
                println!(
                    "perft({}) = {} nodes in {}ms ({} nps)",
                    depth, nodes, ms, nps
                );
            }
            "perftsuite" => {
                let positions = vec![
                    (
                        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
                        5,
                        4865609,
                    ),
                    (
                        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
                        4,
                        4085603,
                    ),
                    ("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", 5, 674624),
                    (
                        "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
                        4,
                        422333,
                    ),
                    (
                        "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
                        4,
                        2103487,
                    ),
                ];
                let mut all_correct = true;
                for (fen, depth, expected) in positions {
                    let b: Board = fen.parse().unwrap();
                    let nodes = uci::perft::perft(&b, depth);
                    if nodes == expected {
                        println!("PASS: {} perft({}) = {}", fen, depth, nodes);
                    } else {
                        println!(
                            "FAIL: {} perft({}) = {} (expected {})",
                            fen, depth, nodes, expected
                        );
                        all_correct = false;
                    }
                }
                if all_correct {
                    println!("perftsuite: ALL PASS");
                } else {
                    println!("perftsuite: FAIL");
                }
            }
            "tactics" => {
                handle.stop_and_join();
                uci::tactics::run_tactics(&tt, &network);
            }
            "savememory" => {
                let mem_path = get_memory_path();
                if tt.save_to_file(mem_path.to_str().unwrap()).is_ok() {
                    if let Some(sig) = &current_sig {
                        let _ = std::fs::write(&sig_path, sig);
                    }
                    println!("info string Transposition table saved to disk");
                }
            }
            "bench" => {
                handle.stop_and_join();
                let depth: i32 = tokens.get(1).and_then(|s| s.parse().ok()).unwrap_or(8);
                uci::bench::run_bench(&tt, &network, depth);
            }
            "quit" => {
                handle.stop_and_join();
                let mem_path = get_memory_path();
                let _ = tt.save_to_file(mem_path.to_str().unwrap());
                if let Some(sig) = &current_sig {
                    let _ = std::fs::write(&sig_path, sig);
                }
                break;
            }
            _ => {}
        }
    }
}
