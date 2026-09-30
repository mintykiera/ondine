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
