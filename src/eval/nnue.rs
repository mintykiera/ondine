use cozy_chess::{Board, Color, Piece};
use nnue_rs::{Board as NnueBoard, Color as NColor, Piece as NPiece, PieceKind as NPieceKind};

pub struct OndineBoard<'a>(pub &'a Board);

impl<'a> OndineBoard<'a> {
    pub fn side_to_move(&self) -> NColor {
        match self.0.side_to_move() {
            Color::White => NColor::White,
            Color::Black => NColor::Black,
        }
    }
}

impl<'a> NnueBoard for OndineBoard<'a> {
    fn side_to_move(&self) -> NColor {
        match self.0.side_to_move() {
            Color::White => NColor::White,
            Color::Black => NColor::Black,
        }
    }

    fn king_square(&self, color: NColor) -> u8 {
        let c = match color {
            NColor::White => Color::White,
            NColor::Black => Color::Black,
        };
        (self.0.pieces(Piece::King) & self.0.colors(c))
            .into_iter()
            .next()
            .unwrap() as u8
    }

    fn for_each_piece(&self, f: &mut dyn FnMut(u8, NPiece)) {
        for &piece in &Piece::ALL {
            let n_kind = match piece {
                Piece::Pawn => NPieceKind::Pawn,
                Piece::Knight => NPieceKind::Knight,
                Piece::Bishop => NPieceKind::Bishop,
                Piece::Rook => NPieceKind::Rook,
                Piece::Queen => NPieceKind::Queen,
                Piece::King => NPieceKind::King,
            };
            for &color in &Color::ALL {
                let n_color = match color {
                    Color::White => NColor::White,
                    Color::Black => NColor::Black,
                };
                let bb = self.0.pieces(piece) & self.0.colors(color);
                for sq in bb {
                    let p = NPiece {
                        kind: n_kind,
                        color: n_color,
                    };
                    f((sq.rank() as u8) * 8 + (sq.file() as u8), p);
                }
            }
        }
    }
}
