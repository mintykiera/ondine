mod negamax;
mod ordering;
mod pv;
mod see;
mod types;

pub use types::SearchInfo;
pub use types::SharedHistory;

use cozy_chess::{Board, Move};
use nnue_rs::Network;
use polyglot_book_rs::PolyglotBook;
use shakmaty::Chess;
use shakmaty_syzygy::{Tablebase, Wdl};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use crate::transposition::TranspositionTable;
use crate::uci::{format_uci_move, parse_uci_move};
use negamax::negamax;
use pv::extract_pv;
use types::{MATE_SCORE, MATE_THRESHOLD, MAX_DEPTH};

pub fn get_best_move(
    board: &Board,
    time_limit: Duration,
    total_clock: Option<Duration>,
    tt: &TranspositionTable,
    shared: &SharedHistory,
    stop_flag: Arc<AtomicBool>,
    is_pondering: Arc<AtomicBool>,
    time_limit_ms: Arc<AtomicU64>,
    is_main_thread: bool,
    thread_id: usize,
    network: &Network,
    history_hashes: &mut Vec<u64>,
    opening_book: &Option<PolyglotBook>,
    syzygy: &Option<Tablebase<Chess>>,
) -> (Option<Move>, Option<Move>) {
    if let Some(book) = opening_book {
        let fen = board.to_string();
        if let Some(entry) = book.get_best_move_from_fen(&fen) {
            if let Some(book_move) = parse_uci_move(board, &entry.move_string) {
                let mut is_legal = false;
                board.generate_moves(|move_list| {
                    for m in move_list {
                        if m == book_move {
                            is_legal = true;
                        }
                    }
                    false
                });
                if is_legal {
                    if is_main_thread {
                        println!(
                            "info string Book move: {}",
                            format_uci_move(board, book_move)
                        );
                    }
                    return (Some(book_move), None);
                }
            }
        }
    }

    let mut info = SearchInfo::new(
        time_limit,
        stop_flag,
        is_pondering.clone(),
        time_limit_ms.clone(),
    );

    let mut best_move: Option<Move> = None;
    let mut best_score = 0i32;
    let mut total_nodes: u64 = 0;
    let root_acc = network.accumulator(&crate::eval::OmoBoard(board));

    // Immediate 1-ply mate check: if any root move delivers checkmate, return it instantly.
    {
        let mut mate_move: Option<Move> = None;
        board.generate_moves(|moves| {
            for m in moves {
                if mate_move.is_some() {
                    return false;
                }
                let mut next_board = board.clone();
                next_board.play_unchecked(m);
                // Opponent must be in check AND have no legal responses
                if !next_board.checkers().is_empty() {
                    let mut has_response = false;
                    next_board.generate_moves(|_responses| {
                        has_response = true;
                        false
                    });
                    if !has_response {
                        mate_move = Some(m);
                        return false;
                    }
                }
            }
            false
        });
        if let Some(m) = mate_move {
            if is_main_thread {
                println!("info depth 1 score mate 1 pv {}", format_uci_move(board, m));
            }
            return (Some(m), None);
        }
    }

    if let Some(tb) = syzygy {
        let piece_count =
            (board.colors(cozy_chess::Color::White) | board.colors(cozy_chess::Color::Black)).len();
        if piece_count <= 6 {
            let mut best_syz_move = None;
            let mut best_wdl: Option<shakmaty_syzygy::AmbiguousWdl> = None;
            let mut best_dtz = i32::MAX;

            board.generate_moves(|moves| {
                for m in moves {
                    let mut next_board = board.clone();
                    next_board.play_unchecked(m);

                    // Repetition-aware: skip moves that repeat a prior board state
                    let next_hash = next_board.hash();
                    if history_hashes.contains(&next_hash) {
                        if is_main_thread {
                            println!(
                                "info string Syzygy skip repetition: {}",
                                format_uci_move(board, m)
                            );
                        }
                        continue;
                    }

                    let fen = next_board.to_string();
                    if let Ok(epd) = fen.parse::<shakmaty::fen::Epd>() {
                        if let Ok(shak_board) =
                            epd.into_position::<shakmaty::Chess>(shakmaty::CastlingMode::Standard)
                        {
                            if let Ok(wdl) = tb.probe_wdl(&shak_board) {
                                if wdl <= Wdl::BlessedLoss.into() {
                                    let dtz_val: i32 = if let Ok(dtz) = tb.probe_dtz(&shak_board) {
                                        let dtz_exact = match dtz {
                                            shakmaty_syzygy::MaybeRounded::Precise(d) => d,
                                            shakmaty_syzygy::MaybeRounded::Rounded(d) => d,
                                        };
                                        i32::from(dtz_exact).abs()
                                    } else {
                                        i32::MAX
                                    };

                                    let is_better = match best_wdl {
                                        None => true,
                                        Some(bw) => {
                                            if wdl < bw {
                                                true
                                            } else if wdl == bw && dtz_val < best_dtz {
                                                true
                                            } else {
                                                false
                                            }
                                        }
                                    };

                                    if is_better {
                                        best_wdl = Some(wdl);
                                        best_dtz = dtz_val;
                                        best_syz_move = Some(m);
                                    }
                                }
                            }
                        }
                    }
                }
                false
            });

            if let Some(m) = best_syz_move {
                if is_main_thread {
                    println!(
                        "info string Root Syzygy probe hit: WDL {:?} DTZ {}",
                        best_wdl.unwrap(),
                        best_dtz
                    );
                }
                return (Some(m), None);
            }
        }
    }

    let mut prev_score: i32 = 0;

    let start_depth = if thread_id > 0 {
        ((thread_id % 2) as i32) + 1
    } else {
        1
    };

    if is_main_thread {
        shared.decay();
    }

    if history_hashes.last() == Some(&board.hash()) {
        history_hashes.pop();
    }

    for depth in start_depth..=MAX_DEPTH {
        info.nodes = 0;

        if !is_pondering.load(Ordering::Relaxed) {
            let current_ms = time_limit_ms.load(Ordering::Relaxed);
            if current_ms > 0 {
                info.time_limit = Duration::from_millis(current_ms);
                let sl = if info.time_limit > Duration::from_millis(25) {
                    info.time_limit - Duration::from_millis(15)
                } else {
                    (info.time_limit * 8) / 10
                };
                info.deadline = Some(info.start_time + sl);
            }
        }

        let mut window = if depth >= 4 { 20 } else { MATE_SCORE };
        let mut alpha = if depth >= 4 {
            (best_score - window).max(-MATE_SCORE)
        } else {
            -MATE_SCORE
        };
        let mut beta = if depth >= 4 {
            (best_score + window).min(MATE_SCORE)
        } else {
            MATE_SCORE
        };

        let (score, current_move) = loop {
            let res = negamax(
                board,
                depth,
                alpha,
                beta,
                0,
                0,
                &mut info,
                tt,
                shared,
                history_hashes,
                None,
                network,
                &root_acc,
                syzygy,
                None,
            );

            if info.aborted {
                break (res.score, res.best_move);
            }

            let score = res.score;
            let mv = res.best_move;

            if score <= alpha && alpha > -MATE_SCORE {
                alpha = (score - window).max(-MATE_SCORE);
                window = window.saturating_mul(2);
                if window > 400 || alpha <= -MATE_SCORE {
                    alpha = -MATE_SCORE;
                    beta = MATE_SCORE;
                }
            } else if score >= beta && beta < MATE_SCORE {
                beta = (score + window).min(MATE_SCORE);
                window = window.saturating_mul(2);
                if window > 400 || beta >= MATE_SCORE {
                    alpha = -MATE_SCORE;
                    beta = MATE_SCORE;
                }
            } else {
                break (score, mv);
            }
        };
        total_nodes += info.nodes;

        if info.aborted {
            break;
        }

        if let Some(m) = current_move {
            best_move = Some(m);
            best_score = score;
        }

        let elapsed_ms = info.start_time.elapsed().as_millis().max(1);
        let nps = ((total_nodes as u128) * 1000) / elapsed_ms;

        if is_main_thread {
            let score_str = if best_score > MATE_THRESHOLD {
                let mate_moves = (MATE_SCORE - best_score + 1) / 2;
                format!("score mate {}", mate_moves)
            } else if best_score < -MATE_THRESHOLD {
                let mate_moves = (MATE_SCORE + best_score + 1) / 2;
                format!("score mate -{}", mate_moves)
            } else {
                format!("score cp {}", best_score)
            };

            println!(
                "info depth {} time {} nodes {} nps {} {} pv {}",
                depth,
                elapsed_ms,
                total_nodes,
                nps,
                score_str,
                extract_pv(board, tt)
            );
        }

        if depth > 1 && score < prev_score - 150 {
            let scaled_ms = ((info.time_limit.as_millis() as f64) * 2.5) as u64;
            let mut new_limit = Duration::from_millis(scaled_ms);
            if let Some(clock) = total_clock {
                let max_clock = Duration::from_millis(((clock.as_millis() as f64) * 0.20) as u64);
                if new_limit > max_clock {
                    new_limit = max_clock;
                }
            }
            if new_limit > info.time_limit {
                info.time_limit = new_limit;
                let sl = if info.time_limit > Duration::from_millis(25) {
                    info.time_limit - Duration::from_millis(15)
                } else {
                    (info.time_limit * 8) / 10
                };
                info.deadline = Some(info.start_time + sl);
            }
        }
        prev_score = score;

        if score.abs() > MATE_SCORE - MAX_DEPTH {
            break;
        }

        if !info.is_pondering.load(Ordering::Relaxed) {
            let current_limit = Duration::from_millis(info.time_limit_ms.load(Ordering::Relaxed));
            let effective_limit = if current_limit.is_zero() {
                info.time_limit
            } else {
                current_limit
            };
            let soft_limit = (effective_limit * 6) / 10;
            if info.start_time.elapsed() >= soft_limit {
                break;
            }
        }
    }

    let ponder_move = best_move.and_then(|bm| {
        let mut after = board.clone();
        after.play_unchecked(bm);
        let hash = after.hash();
        tt.get(hash).and_then(|entry| entry.best_move)
    });

    (best_move, ponder_move)
}
