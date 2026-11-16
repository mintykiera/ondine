pub mod nnue;
pub use nnue::OndineBoard;

pub const NNUE_SCALE: i32 = 4;

#[inline(always)]
pub fn normalize_eval(raw: i32) -> i32 {
    raw / NNUE_SCALE
}

pub fn piece_value(p: cozy_chess::Piece) -> i32 {
    match p {
        cozy_chess::Piece::Pawn => 100,
        cozy_chess::Piece::Knight => 300,
        cozy_chess::Piece::Bishop => 320,
        cozy_chess::Piece::Rook => 500,
        cozy_chess::Piece::Queen => 900,
        cozy_chess::Piece::King => 20_000,
    }
}

/// Mop-up evaluation for converting decisive endgames into rapid checkmates.
/// Drives the enemy king to the edges/corners and brings our king into proximity.
pub fn mop_up_eval(board: &cozy_chess::Board, eval: i32) -> i32 {
    if eval.abs() < 300 || eval.abs() >= 90_000 {
        return eval;
    }

    let (winning_color, losing_color) = if eval > 0 {
        (board.side_to_move(), !board.side_to_move())
    } else {
        (!board.side_to_move(), board.side_to_move())
    };

    let losing_pawns = (board.pieces(cozy_chess::Piece::Pawn) & board.colors(losing_color)).len();
    if losing_pawns > 0 {
        return eval;
    }

    let losing_non_king = (board.colors(losing_color) & !board.pieces(cozy_chess::Piece::King)).len();
    if losing_non_king > 1 {
        return eval;
    }

    let winning_king = match (board.pieces(cozy_chess::Piece::King) & board.colors(winning_color)).into_iter().next() {
        Some(k) => k,
        None => return eval,
    };
    let losing_king = match (board.pieces(cozy_chess::Piece::King) & board.colors(losing_color)).into_iter().next() {
        Some(k) => k,
        None => return eval,
    };

    let wk_file = winning_king.file() as i32;
    let wk_rank = winning_king.rank() as i32;
    let lk_file = losing_king.file() as i32;
    let lk_rank = losing_king.rank() as i32;

    // Center distance of losing king (0 at center, up to 6 on corners)
    let center_dist = (lk_file - 3).abs().max((lk_file - 4).abs())
        + (lk_rank - 3).abs().max((lk_rank - 4).abs());

    // Proximity between kings (14 - distance, higher is closer)
    let king_dist = (wk_file - lk_file).abs() + (wk_rank - lk_rank).abs();
    let proximity = 14 - king_dist;

    let bonus = center_dist * 8 + proximity * 4;

    if eval > 0 {
        eval + bonus
    } else {
        eval - bonus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cozy_chess::Board;

    #[test]
    fn test_mop_up_corner_gradient() {
        // White K on e1, R on a1, Black K on e4 (center)
        let center_board: Board = "8/8/8/8/4k3/8/8/R3K3 w - - 0 1".parse().unwrap();
        let center_eval = mop_up_eval(&center_board, 500);

        // White K on e1, R on a1, Black K on h8 (corner, not in check)
        let corner_board: Board = "7k/8/8/8/8/8/8/R3K3 w - - 0 1".parse().unwrap();
        let corner_eval = mop_up_eval(&corner_board, 500);

        // Pushing the king to the corner must increase the evaluation score
        assert!(corner_eval > center_eval, "Corner eval {} must be > center eval {}", corner_eval, center_eval);
    }
}
