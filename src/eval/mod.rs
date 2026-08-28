pub mod nnue;
pub use nnue::OndineBoard;

/// nn-1c0000000000.nnue (HalfKAv2Hm) outputs ~400-500 internal units per pawn.
/// Divide by this to normalize to centipawn scale (~100 per pawn).
pub const NNUE_SCALE: i32 = 4;

/// Normalize raw NNUE output to centipawns.
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
