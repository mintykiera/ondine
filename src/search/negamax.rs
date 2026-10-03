use cozy_chess::{Board, Move, Piece};
use nnue_rs::{Accumulator, Network};
use shakmaty::{CastlingMode, Chess};
use shakmaty_syzygy::{Tablebase, Wdl};

use super::ordering::lmr_reduction;
use super::see::see;
use super::types::{
    ACC_STACK_SIZE, MATE_SCORE, MATE_THRESHOLD, MAX_EXTENSIONS, MAX_PLY, MoveStack, SearchInfo,
    SharedHistory, StagedMovePicker, score_from_tt, score_to_tt,
};
use crate::eval::{normalize_eval, piece_value};
use crate::transposition::{NodeType, TranspositionTable};

#[derive(Clone, Copy, Debug)]
pub(crate) struct SearchResult {
    pub score: i32,
    pub best_move: Option<Move>,
}

impl SearchResult {
    pub const fn new(score: i32, best_move: Option<Move>) -> Self {
        Self { score, best_move }
    }
}

#[inline(always)]
fn acc_split(stack: &mut [Accumulator], ply: usize) -> (&Accumulator, &mut Accumulator) {
    let (left, right) = stack.split_at_mut(ply + 1);
    (&left[ply], &mut right[0])
}

pub(crate) fn negamax(
    board: &Board,
    depth: i32,
    mut alpha: i32,
    mut beta: i32,
    ply: i32,
    extensions: i32,
    info: &mut SearchInfo,
    tt: &TranspositionTable,
    shared: &SharedHistory,
    history_hashes: &mut Vec<u64>,
    prev_move: Option<Move>,
    prev_prev_move: Option<Move>,
    network: &Network,
    acc_stack: &mut [Accumulator],
    syzygy: &Option<Tablebase<Chess>>,
    excluded_move: Option<Move>,
) -> SearchResult {
    info.check_time();
    if info.aborted {
        return SearchResult::new(0, None);
    }

    info.nodes += 1;
    let ply_idx = ply as usize;

    if ply >= (MAX_PLY as i32) - 1 {
        return SearchResult::new(
            normalize_eval(network.evaluate_accumulator(
                &acc_stack[ply_idx],
                crate::eval::OndineBoard(board).side_to_move(),
            )),
            None,
        );
    }

    let hash = board.hash();
    if ply > 0 {
        if board.halfmove_clock() >= 100 {
            return SearchResult::new(0, None);
        }

        let hc = board.halfmove_clock() as usize;
        let scan_limit = hc.min(history_hashes.len());
        let mut is_repetition = false;
        let mut i = 2;
        while i <= scan_limit {
            let idx = history_hashes.len() - i;
            if history_hashes[idx] == hash {
                is_repetition = true;
                break;
            }
            i += 2;
        }
        if is_repetition {
            return SearchResult::new(0, None);
        }
    }

    if board.occupied().len() <= 6 && ply > 0 && depth >= 8 && excluded_move.is_none() {
        if let Some(tb) = syzygy {
            let mut s_board = shakmaty::Board::empty();
            for &color in &[cozy_chess::Color::White, cozy_chess::Color::Black] {
                let s_color = if color == cozy_chess::Color::White {
                    shakmaty::Color::White
                } else {
                    shakmaty::Color::Black
                };
                for &piece in &[
                    cozy_chess::Piece::Pawn,
                    cozy_chess::Piece::Knight,
                    cozy_chess::Piece::Bishop,
                    cozy_chess::Piece::Rook,
                    cozy_chess::Piece::Queen,
                    cozy_chess::Piece::King,
                ] {
                    let bb = board.colored_pieces(color, piece);
                    let s_role = match piece {
                        cozy_chess::Piece::Pawn => shakmaty::Role::Pawn,
                        cozy_chess::Piece::Knight => shakmaty::Role::Knight,
                        cozy_chess::Piece::Bishop => shakmaty::Role::Bishop,
                        cozy_chess::Piece::Rook => shakmaty::Role::Rook,
                        cozy_chess::Piece::Queen => shakmaty::Role::Queen,
                        cozy_chess::Piece::King => shakmaty::Role::King,
                    };
                    for sq in bb {
                        s_board.set_piece_at(
                            shakmaty::Square::new(sq as u32),
                            shakmaty::Piece {
                                color: s_color,
                                role: s_role,
                            },
                        );
                    }
                }
            }
            let s_turn = if board.side_to_move() == cozy_chess::Color::White {
                shakmaty::Color::White
            } else {
                shakmaty::Color::Black
            };
            let mut castling = shakmaty::Bitboard::default();
            let w_cr = board.castle_rights(cozy_chess::Color::White);
            if w_cr.short.is_some() {
                castling.add(shakmaty::Square::H1);
            }
            if w_cr.long.is_some() {
                castling.add(shakmaty::Square::A1);
            }
            let b_cr = board.castle_rights(cozy_chess::Color::Black);
            if b_cr.short.is_some() {
                castling.add(shakmaty::Square::H8);
            }
            if b_cr.long.is_some() {
                castling.add(shakmaty::Square::A8);
            }

            let ep_square = board
                .en_passant()
                .map(|sq| shakmaty::Square::new(sq as u32));
            use shakmaty::FromSetup;
            let setup = shakmaty::Setup {
                board: s_board,
                promoted: shakmaty::Bitboard::default(),
                turn: s_turn,
                castling_rights: castling,
                ep_square,
                halfmoves: board.halfmove_clock() as u32,
                fullmoves: std::num::NonZeroU32::new((board.fullmove_number() as u32).max(1))
                    .unwrap(),
                pockets: None,
                remaining_checks: None,
            };
            if let Ok(pos) = shakmaty::Chess::from_setup(setup, CastlingMode::Standard) {
                if let Ok(wdl) = tb.probe_wdl_after_zeroing(&pos) {
                    let egtb_score = match wdl {
                        Wdl::Win => (MATE_SCORE - 1000) - ply,
                        Wdl::Loss => -(MATE_SCORE - 1000) + ply,
                        _ => 0,
                    };
                    return SearchResult::new(egtb_score, None);
                }
            }
        }
    }

    let is_pv = beta - alpha > 1;
    let original_alpha = alpha;
    let tt_entry = tt.get(hash);

    if excluded_move.is_none() && ply > 0 && !is_pv {
        if let Some(entry) = tt_entry {
            if entry.depth >= depth {
                let score = score_from_tt(entry.score, ply);
                match entry.node_type {
                    NodeType::Exact => {
                        return SearchResult::new(score, entry.best_move);
                    }
                    NodeType::LowerBound => {
                        alpha = alpha.max(score);
                    }
                    NodeType::UpperBound => {
                        beta = beta.min(score);
                    }
                }
                if alpha >= beta {
                    return SearchResult::new(score, entry.best_move);
                }
            }
        }
    }

    if depth <= 0 {
        return SearchResult::new(
            quiescence_search(
                board, alpha, beta, ply, info, tt, shared, network, acc_stack,
            ),
            None,
        );
    }

    let in_check = !board.checkers().is_empty();

    let (depth, extensions) = if in_check && extensions < MAX_EXTENSIONS {
        (depth + 1, extensions + 1)
    } else {
        (depth, extensions)
    };

    let mut static_eval = if !in_check {
        normalize_eval(network.evaluate_accumulator(
            &acc_stack[ply_idx],
            crate::eval::OndineBoard(board).side_to_move(),
        ))
    } else {
        -MATE_SCORE
    };

    if !in_check {
        if let Some(entry) = tt_entry {
            let tt_score = score_from_tt(entry.score, ply);
            if tt_score.abs() < MATE_THRESHOLD {
                match entry.node_type {
                    NodeType::Exact => static_eval = tt_score,
                    NodeType::LowerBound if tt_score > static_eval => static_eval = tt_score,
                    NodeType::UpperBound if tt_score < static_eval => static_eval = tt_score,
                    _ => {}
                }
            }
        }
    }

    let ply_idx_eval = ply as usize;
    info.set_eval(ply_idx_eval, static_eval);

    let prev_eval = if ply >= 2 {
        info.get_eval((ply - 2) as usize)
    } else {
        -MATE_SCORE
    };
    let improving = !in_check && ply >= 2 && prev_eval > -MATE_THRESHOLD && static_eval > prev_eval;

    let rfp_margin = if improving { 60 } else { 80 };
    if !in_check && ply > 0 && depth <= 7 && excluded_move.is_none() {
        if static_eval - rfp_margin * depth >= beta {
            return SearchResult::new(beta, None);
        }
    }

    if !in_check && ply > 0 && depth <= 3 && excluded_move.is_none() && !is_pv {
        let razor_margin = 150 * depth;
        if static_eval + razor_margin <= alpha {
            let razor_score = quiescence_search(
                board, alpha, beta, ply, info, tt, shared, network, acc_stack,
            );
            if razor_score <= alpha {
                return SearchResult::new(razor_score, None);
            }
        }
    }

    // ProbCut: shallow capture search to quickly prune tactical positions.
    if !in_check
        && depth >= 5
        && beta.abs() < MATE_THRESHOLD
        && excluded_move.is_none()
    {
        let probcut_beta = beta + 200;
        let probcut_depth = depth - 4;

        let mut pc_stack = MoveStack::new();
        board.generate_moves(|move_list| {
            for m in move_list {
                let is_ep = board.piece_on(m.from) == Some(Piece::Pawn)
                    && m.from.file() != m.to.file()
                    && board.color_on(m.to).is_none();
                let is_cap = board.color_on(m.to).is_some() || is_ep;
                let is_promo = m.promotion.is_some();
                if (is_cap || is_promo) && see(board, m) >= 0 {
                    pc_stack.push(m);
                }
            }
            false
        });

        for &m in pc_stack.as_slice() {
            let mut next_board = board.clone();
            next_board.play_unchecked(m);

            {
                let (parent_acc, child_acc) = acc_split(acc_stack, ply_idx);
                network.update(
                    &crate::eval::OndineBoard(board),
                    &crate::eval::OndineBoard(&next_board),
                    parent_acc,
                    child_acc,
                );
            }

            history_hashes.push(hash);
            let res = negamax(
                &next_board,
                probcut_depth,
                -probcut_beta,
                -probcut_beta + 1,
                ply + 1,
                extensions,
                info,
                tt,
                shared,
                history_hashes,
                Some(m),
                prev_move,
                network,
                acc_stack,
                syzygy,
                None,
            );
            history_hashes.pop();

            if info.aborted {
                return SearchResult::new(0, None);
            }

            if -res.score >= probcut_beta {
                return SearchResult::new(probcut_beta, None);
            }
        }
    }

    if !in_check
        && ply > 0
        && depth >= 3
        && excluded_move.is_none()
        && prev_move.is_some()
        && static_eval >= beta
    {
        let our_pieces = (board.pieces(Piece::Knight)
            | board.pieces(Piece::Bishop)
            | board.pieces(Piece::Rook)
            | board.pieces(Piece::Queen))
            & board.colors(board.side_to_move());
        if our_pieces.len() > 0 {
            if let Some(null_board) = board.null_move() {
                let r = 3 + depth / 3 + ((static_eval - beta) / 200).clamp(0, 3);
                {
                    let (parent, child) = acc_split(acc_stack, ply_idx);
                    child.clone_from(parent);
                }
                history_hashes.push(hash);
                let res = negamax(
                    &null_board,
                    depth - 1 - r,
                    -beta,
                    -beta + 1,
                    ply + 1,
                    extensions,
                    info,
                    tt,
                    shared,
                    history_hashes,
                    None,
                    prev_move,
                    network,
                    acc_stack,
                    syzygy,
                    None,
                );
                history_hashes.pop();
                if info.aborted {
                    return SearchResult::new(0, None);
                }
                let null_score = -res.score;
                if null_score >= beta {
                    if depth >= 12 {
                        let verify_res = negamax(
                            board,
                            depth - r - 1,
                            beta - 1,
                            beta,
                            ply,
                            extensions,
                            info,
                            tt,
                            shared,
                            history_hashes,
                            None,
                            prev_move,
                            network,
                            acc_stack,
                            syzygy,
                            None,
                        );
                        if info.aborted {
                            return SearchResult::new(0, None);
                        }
                        if verify_res.score >= beta {
                            return SearchResult::new(beta, None);
                        }
                    } else {
                        return SearchResult::new(beta, None);
                    }
                }
            }
        }
    }

    let mut is_singular = false;
    let mut singular_move = None;

    if excluded_move.is_none() && depth >= 8 {
        if let Some(entry) = tt_entry {
            if entry.depth >= depth - 3
                && (entry.node_type == NodeType::Exact || entry.node_type == NodeType::LowerBound)
                && entry.best_move.is_some()
            {
                let tt_m = entry.best_move.unwrap();
                let tt_score = score_from_tt(entry.score, ply);
                if tt_score.abs() < MATE_THRESHOLD {
                    let margin = depth * 2;
                    let singular_beta = tt_score - margin;
                    let singular_alpha = singular_beta - 1;
                    let singular_depth = (depth - 1) / 2;

                    let sing_res = negamax(
                        board,
                        singular_depth,
                        singular_alpha,
                        singular_beta,
                        ply,
                        extensions,
                        info,
                        tt,
                        shared,
                        history_hashes,
                        prev_move,
                        prev_prev_move,
                        network,
                        acc_stack,
                        syzygy,
                        Some(tt_m),
                    );

                    if info.aborted {
                        return SearchResult::new(0, None);
                    }

                    if sing_res.score <= singular_alpha {
                        is_singular = true;
                        singular_move = Some(tt_m);
                    }
                }
            }
        }
    }

    let tt_move = tt_entry.and_then(|e| e.best_move);

    let depth = if depth >= 4 && tt_move.is_none() && !in_check && excluded_move.is_none() {
        depth - 1
    } else {
        depth
    };

    let mut move_stack = MoveStack::new();
    board.generate_moves(|move_list| {
        for m in move_list {
            move_stack.push(m);
        }
        false
    });

    if move_stack.is_empty() {
        if in_check {
            return SearchResult::new(-MATE_SCORE + (ply as i32), None);
        } else {
            return SearchResult::new(0, None);
        }
    }

    let mut futility_pruning = false;
    if !in_check && depth <= 4 && ply > 0 {
        if static_eval + (depth * 100) <= alpha {
            futility_pruning = true;
        }
    }

    let killers = if ply_idx < MAX_PLY {
        [info.get_killer(ply_idx, 0), info.get_killer(ply_idx, 1)]
    } else {
        [None, None]
    };

    let counter_move = prev_move.and_then(|pm| shared.get_counter_move(board.side_to_move(), pm));

    let mut picker = StagedMovePicker::new(&move_stack, tt_move, killers, counter_move);
    let mut best_move: Option<Move> = None;
    let mut best_score = -MATE_SCORE;
    let mut move_index = 0usize;
    let mut quiet_moves_searched = 0i32;
    let mut searched_quiets = MoveStack::new();

    while let Some((m, _)) = picker.pick_next(board, shared, prev_move, prev_prev_move) {
        if excluded_move == Some(m) {
            continue;
        }

        let is_ep = board.piece_on(m.from) == Some(Piece::Pawn)
            && m.from.file() != m.to.file()
            && board.color_on(m.to).is_none();
        let is_capture = board.color_on(m.to).is_some() || is_ep;
        let is_quiet = !is_capture && m.promotion.is_none();
        let is_killer = ply_idx < MAX_PLY && (killers[0] == Some(m) || killers[1] == Some(m));
        let is_counter = Some(m) == counter_move;

        let lmp_threshold = (4 + depth * depth * 2) / (2 - improving as i32);
        if !in_check && depth <= 4 && is_quiet && !is_killer && !is_counter && quiet_moves_searched > lmp_threshold {
            continue;
        }

        if !in_check
            && is_quiet
            && !is_killer
            && !is_counter
            && depth <= 6
            && move_index > 0
            && see(board, m) < -depth * 80
        {
            continue;
        }

        if !in_check && is_capture && depth <= 5 && move_index > 0 && see(board, m) < -depth * 80 {
            continue;
        }

        if !in_check && is_quiet && depth <= 5 && quiet_moves_searched >= 3 {
            let hist = shared.get_history(board.side_to_move(), m.from, m.to);
            let cont = prev_move
                .map(|pm| shared.get_cont_history(pm.to, m.to))
                .unwrap_or(0);
            if hist + cont < -2048 * depth {
                continue;
            }
        }

        let mut next_board = board.clone();
        next_board.play_unchecked(m);

        let gives_check = !next_board.checkers().is_empty();

        if futility_pruning && is_quiet && !gives_check && !is_killer && !is_counter && quiet_moves_searched > 0 {
            continue;
        }

        if is_quiet {
            quiet_moves_searched += 1;
            searched_quiets.push(m);
        }

        {
            let (parent_acc, child_acc) = acc_split(acc_stack, ply_idx);
            network.update(
                &crate::eval::OndineBoard(board),
                &crate::eval::OndineBoard(&next_board),
                parent_acc,
                child_acc,
            );
        }

        let singular_ext = if is_singular && Some(m) == singular_move && extensions < MAX_EXTENSIONS
        {
            1
        } else {
            0
        };

        let mut score;

        if move_index == 0 {
            history_hashes.push(hash);
            let res = negamax(
                &next_board,
                depth - 1 + singular_ext,
                -beta,
                -alpha,
                ply + 1,
                extensions + singular_ext,
                info,
                tt,
                shared,
                history_hashes,
                Some(m),
                prev_move,
                network,
                acc_stack,
                syzygy,
                None,
            );
            history_hashes.pop();
            score = -res.score;
        } else {
            let mut reduced_depth = depth - 1 + singular_ext;
            let do_lmr = move_index >= 3
                && depth >= 3
                && !is_capture
                && !gives_check
                && !in_check
                && !is_killer;
            if do_lmr {
                let r = lmr_reduction(depth, move_index as i32);
                let history_score = shared.get_history(board.side_to_move(), m.from, m.to);
                let history_adjustment = history_score / 4096;
                let improving_adj = if improving { 1 } else { 0 };
                let final_r = (r - history_adjustment - improving_adj).max(0);
                reduced_depth = (depth - 1 + singular_ext - final_r).max(1);
            }

            history_hashes.push(hash);
            let res = negamax(
                &next_board,
                reduced_depth,
                -alpha - 1,
                -alpha,
                ply + 1,
                extensions + singular_ext,
                info,
                tt,
                shared,
                history_hashes,
                Some(m),
                prev_move,
                network,
                acc_stack,
                syzygy,
                None,
            );
            score = -res.score;

            if do_lmr && reduced_depth < depth - 1 + singular_ext && score > alpha {
                let zw_res = negamax(
                    &next_board,
                    depth - 1 + singular_ext,
                    -alpha - 1,
                    -alpha,
                    ply + 1,
                    extensions + singular_ext,
                    info,
                    tt,
                    shared,
                    history_hashes,
                    Some(m),
                    prev_move,
                    network,
                    acc_stack,
                    syzygy,
                    None,
                );
                score = -zw_res.score;
            }

            if score > alpha && score < beta {
                let pv_res = negamax(
                    &next_board,
                    depth - 1 + singular_ext,
                    -beta,
                    -alpha,
                    ply + 1,
                    extensions + singular_ext,
                    info,
                    tt,
                    shared,
                    history_hashes,
                    Some(m),
                    prev_move,
                    network,
                    acc_stack,
                    syzygy,
                    None,
                );
                score = -pv_res.score;
            }
            history_hashes.pop();
        }

        if info.aborted {
            return SearchResult::new(0, None);
        }

        if score > best_score {
            best_score = score;
            best_move = Some(m);
        }
        if score > alpha {
            alpha = score;
        }
        if alpha >= beta {
            let color = board.side_to_move();
            let bonus = (depth * 32).min(1800);

            if is_quiet {
                if ply_idx < MAX_PLY {
                    info.update_killer(ply_idx, m);
                }

                if let Some(pm) = prev_move {
                    shared.update_counter_move(color, pm, m);
                }

                shared.add_history(color, m.from, m.to, bonus);

                for &qm in searched_quiets.as_slice() {
                    if qm != m {
                        shared.add_history(color, qm.from, qm.to, -bonus);
                    }
                }

                if let Some(pm) = prev_move {
                    shared.add_cont_history(pm.to, m.to, bonus);
                    for &qm in searched_quiets.as_slice() {
                        if qm != m {
                            shared.add_cont_history(pm.to, qm.to, -bonus);
                        }
                    }
                }

                if let Some(ppm) = prev_prev_move {
                    shared.add_cont_2ply(ppm.to, m.to, bonus);
                    for &qm in searched_quiets.as_slice() {
                        if qm != m {
                            shared.add_cont_2ply(ppm.to, qm.to, -bonus);
                        }
                    }
                }
            } else {
                shared.add_capture_history(color, m.from, m.to, bonus);
            }
            break;
        }

        move_index += 1;
    }

    if best_score == -MATE_SCORE && !in_check {
        best_score = alpha;
    }

    if excluded_move.is_none() {
        let node_type = if best_score <= original_alpha {
            NodeType::UpperBound
        } else if best_score >= beta {
            NodeType::LowerBound
        } else {
            NodeType::Exact
        };
        tt.insert(
            hash,
            depth,
            score_to_tt(best_score, ply),
            node_type,
            best_move,
        );
    }

    SearchResult::new(best_score, best_move)
}

pub(crate) fn quiescence_search(
    board: &Board,
    mut alpha: i32,
    beta: i32,
    ply: i32,
    info: &mut SearchInfo,
    tt: &TranspositionTable,
    shared: &SharedHistory,
    network: &Network,
    acc_stack: &mut [Accumulator],
) -> i32 {
    info.check_time();
    if info.aborted {
        return 0;
    }

    info.nodes += 1;
    let ply_idx = ply as usize;

    if ply_idx >= ACC_STACK_SIZE - 1 {
        let in_check = !board.checkers().is_empty();
        return if in_check {
            -MATE_SCORE + ply
        } else {
            normalize_eval(network.evaluate_accumulator(
                &acc_stack[ply_idx],
                crate::eval::OndineBoard(board).side_to_move(),
            ))
        };
    }

    let in_check = !board.checkers().is_empty();

    let hash = board.hash();
    let tt_entry = tt.get(hash);
    if !in_check {
        if let Some(entry) = tt_entry {
            if entry.depth >= 0 {
                let tt_score = score_from_tt(entry.score, ply);
                match entry.node_type {
                    NodeType::Exact => return tt_score,
                    NodeType::LowerBound if tt_score >= beta => return tt_score,
                    NodeType::UpperBound if tt_score <= alpha => return tt_score,
                    _ => {}
                }
            }
        }
    }

    let stand_pat = if in_check {
        -MATE_SCORE + ply
    } else {
        normalize_eval(network.evaluate_accumulator(
            &acc_stack[ply_idx],
            crate::eval::OndineBoard(board).side_to_move(),
        ))
    };

    if !in_check {
        if stand_pat >= beta {
            return beta;
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }

        const BIG_DELTA: i32 = 1800;
        if stand_pat + BIG_DELTA < alpha {
            return alpha;
        }
    }

    let mut move_stack = MoveStack::new();
    board.generate_moves(|move_list| {
        for m in move_list {
            let is_ep = board.piece_on(m.from) == Some(Piece::Pawn)
                && m.from.file() != m.to.file()
                && board.color_on(m.to).is_none();
            if in_check || board.color_on(m.to).is_some() || is_ep || m.promotion.is_some() {
                move_stack.push(m);
            }
        }
        false
    });

    if in_check && move_stack.is_empty() {
        return -MATE_SCORE + ply;
    }

    let tt_move = tt_entry.and_then(|e| e.best_move);
    let color = board.side_to_move();
    let moves = move_stack.as_mut_slice();
    moves.sort_unstable_by(|a, b| {
        let score_move = |m: &Move| -> i32 {
            if Some(*m) == tt_move {
                return 1_000_000;
            }
            let is_ep = board.piece_on(m.from) == Some(Piece::Pawn)
                && m.from.file() != m.to.file()
                && board.color_on(m.to).is_none();
            let victim_val = if is_ep {
                piece_value(Piece::Pawn)
            } else {
                board.piece_on(m.to).map(piece_value).unwrap_or(0)
            };
            let attacker_val = board.piece_on(m.from).map(piece_value).unwrap_or(100);
            let promo_val = m.promotion.map(piece_value).unwrap_or(0);
            let cap_hist = shared.get_capture_history(color, m.from, m.to) / 32;
            (victim_val * 10 - attacker_val + promo_val * 10) * 100 + cap_hist
        };
        score_move(b).cmp(&score_move(a))
    });

    let mut best_score = if in_check {
        -MATE_SCORE + ply
    } else {
        stand_pat
    };

    const DELTA_MARGIN: i32 = 200;
    for m in moves.iter() {
        if !in_check {
            let is_ep = board.piece_on(m.from) == Some(Piece::Pawn)
                && m.from.file() != m.to.file()
                && board.color_on(m.to).is_none();
            let victim_val = if is_ep {
                piece_value(Piece::Pawn)
            } else {
                board.piece_on(m.to).map(piece_value).unwrap_or(0)
            };
            let promo_val = m.promotion.map(piece_value).unwrap_or(0);
            if stand_pat + victim_val + promo_val + DELTA_MARGIN < alpha {
                continue;
            }

            if see(board, *m) < 0 {
                continue;
            }
        }

        let mut next_board = board.clone();
        next_board.play_unchecked(*m);

        {
            let (parent_acc, child_acc) = acc_split(acc_stack, ply_idx);
            network.update(
                &crate::eval::OndineBoard(board),
                &crate::eval::OndineBoard(&next_board),
                parent_acc,
                child_acc,
            );
        }

        let score = -quiescence_search(
            &next_board,
            -beta,
            -alpha,
            ply + 1,
            info,
            tt,
            shared,
            network,
            acc_stack,
        );

        if info.aborted {
            return 0;
        }

        if score > best_score {
            best_score = score;
        }
        if score >= beta {
            return score;
        }
        if score > alpha {
            alpha = score;
        }
    }

    best_score
}
