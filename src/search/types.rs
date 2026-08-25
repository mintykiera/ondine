use cozy_chess::{Board, Move, Piece, Square};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(crate) const MAX_DEPTH: i32 = 64;
pub(crate) const MATE_SCORE: i32 = 100_000;
pub(crate) const MATE_THRESHOLD: i32 = 90_000;
pub(crate) const MAX_PLY: usize = 128;
pub(crate) const MAX_EXTENSIONS: i32 = 3;

pub struct SharedHistory {
    pub history: [[[AtomicI32; 64]; 64]; 2],
    pub cont_history: [[AtomicI32; 64]; 64],
    pub counter_moves: [[[AtomicU32; 64]; 64]; 2],
    pub capture_history: [[[AtomicI32; 64]; 64]; 2],
}

impl SharedHistory {
    pub fn new() -> Self {
        let history = std::array::from_fn(|_| {
            std::array::from_fn(|_| std::array::from_fn(|_| AtomicI32::new(0)))
        });
        let cont_history = std::array::from_fn(|_| std::array::from_fn(|_| AtomicI32::new(0)));
        let counter_moves = std::array::from_fn(|_| {
            std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0)))
        });
        let capture_history = std::array::from_fn(|_| {
            std::array::from_fn(|_| std::array::from_fn(|_| AtomicI32::new(0)))
        });

        Self {
            history,
            cont_history,
            counter_moves,
            capture_history,
        }
    }

    pub fn clear(&self) {
        for c in 0..2 {
            for i in 0..64 {
                for j in 0..64 {
                    self.history[c][i][j].store(0, Ordering::Relaxed);
                    self.counter_moves[c][i][j].store(0, Ordering::Relaxed);
                    self.capture_history[c][i][j].store(0, Ordering::Relaxed);
                }
            }
        }
        for i in 0..64 {
            for j in 0..64 {
                self.cont_history[i][j].store(0, Ordering::Relaxed);
            }
        }
    }

    pub fn decay(&self) {
        for c in 0..2 {
            for i in 0..64 {
                for j in 0..64 {
                    let h = self.history[c][i][j].load(Ordering::Relaxed);
                    self.history[c][i][j].store(h / 2, Ordering::Relaxed);
                    let ch = self.capture_history[c][i][j].load(Ordering::Relaxed);
                    self.capture_history[c][i][j].store(ch / 2, Ordering::Relaxed);
                }
            }
        }
        for i in 0..64 {
            for j in 0..64 {
                let c = self.cont_history[i][j].load(Ordering::Relaxed);
                self.cont_history[i][j].store(c / 2, Ordering::Relaxed);
            }
        }
    }

    pub fn get_history(&self, color: cozy_chess::Color, from: Square, to: Square) -> i32 {
        self.history[color as usize][from as usize][to as usize].load(Ordering::Relaxed)
    }

    pub fn add_history(&self, color: cozy_chess::Color, from: Square, to: Square, bonus: i32) {
        let c = color as usize;
        let f = from as usize;
        let t = to as usize;
        let val = self.history[c][f][t].load(Ordering::Relaxed);
        let new_val = val + bonus - (val * bonus.abs()) / 16384;
        self.history[c][f][t].store(new_val.clamp(-16384, 16384), Ordering::Relaxed);
    }

    pub fn get_cont_history(&self, prev_to: Square, curr_to: Square) -> i32 {
        self.cont_history[prev_to as usize][curr_to as usize].load(Ordering::Relaxed)
    }

    pub fn add_cont_history(&self, prev_to: Square, curr_to: Square, bonus: i32) {
        let p = prev_to as usize;
        let c = curr_to as usize;
        let val = self.cont_history[p][c].load(Ordering::Relaxed);
        let new_val = val + bonus - (val * bonus.abs()) / 16384;
        self.cont_history[p][c].store(new_val.clamp(-16384, 16384), Ordering::Relaxed);
    }

    pub fn get_counter_move(&self, color: cozy_chess::Color, prev_move: Move) -> Option<Move> {
        let c = color as usize;
        let f = prev_move.from as usize;
        let t = prev_move.to as usize;
        let val = self.counter_moves[c][f][t].load(Ordering::Relaxed);
        crate::transposition::unpack_move(val as u16)
    }

    pub fn update_counter_move(
        &self,
        color: cozy_chess::Color,
        prev_move: Move,
        counter_move: Move,
    ) {
        let c = color as usize;
        let f = prev_move.from as usize;
        let t = prev_move.to as usize;
        let val = crate::transposition::pack_move(Some(counter_move)) as u32;
        self.counter_moves[c][f][t].store(val, Ordering::Relaxed);
    }

    pub fn get_capture_history(&self, color: cozy_chess::Color, from: Square, to: Square) -> i32 {
        self.capture_history[color as usize][from as usize][to as usize].load(Ordering::Relaxed)
    }

    pub fn add_capture_history(
        &self,
        color: cozy_chess::Color,
        from: Square,
        to: Square,
        bonus: i32,
    ) {
        let c = color as usize;
        let f = from as usize;
        let t = to as usize;
        let val = self.capture_history[c][f][t].load(Ordering::Relaxed);
        let new_val = val + bonus - (val * bonus.abs()) / 16384;
        self.capture_history[c][f][t].store(new_val.clamp(-16384, 16384), Ordering::Relaxed);
    }
}

#[inline]
pub(crate) fn score_to_tt(score: i32, ply: i32) -> i32 {
    if score > MATE_THRESHOLD {
        score + ply
    } else if score < -MATE_THRESHOLD {
        score - ply
    } else {
        score
    }
}

pub(crate) struct MoveStack {
    moves: [Move; 256],
    len: usize,
}

impl MoveStack {
    pub fn new() -> Self {
        Self {
            moves: [Move {
                from: Square::A1,
                to: Square::A1,
                promotion: None,
            }; 256],
            len: 0,
        }
    }

    #[inline(always)]
    pub fn push(&mut self, m: Move) {
        if self.len < 256 {
            self.moves[self.len] = m;
            self.len += 1;
        }
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [Move] {
        &mut self.moves[..self.len]
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    TtMove,
    GoodCaptures,
    Killers,
    Countermove,
    Quiets,
    BadCaptures,
    Done,
}

pub(crate) struct StagedMovePicker<'a> {
    stack: &'a MoveStack,
    tt_move: Option<Move>,
    killers: [Option<Move>; 2],
    counter_move: Option<Move>,
    stage: Stage,
    current_index: usize,
    scores: [i32; 256],
    scored_captures: bool,
    scored_quiets: bool,
    bad_captures: MoveStack,
    bad_capture_scores: [i32; 256],
}

impl<'a> StagedMovePicker<'a> {
    pub fn new(
        stack: &'a MoveStack,
        tt_move: Option<Move>,
        killers: [Option<Move>; 2],
        counter_move: Option<Move>,
    ) -> Self {
        Self {
            stack,
            tt_move,
            killers,
            counter_move,
            stage: Stage::TtMove,
            current_index: 0,
            scores: [i32::MIN; 256],
            scored_captures: false,
            scored_quiets: false,
            bad_captures: MoveStack::new(),
            bad_capture_scores: [i32::MIN; 256],
        }
    }

    #[inline]
    pub fn pick_next(
        &mut self,
        board: &Board,
        shared: &SharedHistory,
        prev_move: Option<Move>,
    ) -> Option<(Move, usize)> {
        loop {
            match self.stage {
                Stage::TtMove => {
                    self.stage = Stage::GoodCaptures;
                    if let Some(tm) = self.tt_move {
                        if let Some(idx) = self.stack.as_slice().iter().position(|&m| m == tm) {
                            return Some((tm, idx));
                        }
                    }
                }
                Stage::GoodCaptures => {
                    if !self.scored_captures {
                        self.scored_captures = true;
                        let color = board.side_to_move();
                        for (i, &m) in self.stack.as_slice().iter().enumerate() {
                            if Some(m) == self.tt_move {
                                continue;
                            }
                            let is_ep = board.piece_on(m.from) == Some(Piece::Pawn)
                                && m.from.file() != m.to.file()
                                && board.color_on(m.to).is_none();
                            let is_cap = board.color_on(m.to).is_some() || is_ep;
                            let is_promo = m.promotion.is_some();
                            if is_cap || is_promo {
                                let victim_val = if is_ep {
                                    crate::eval::piece_value(Piece::Pawn)
                                } else {
                                    board
                                        .piece_on(m.to)
                                        .map(crate::eval::piece_value)
                                        .unwrap_or(0)
                                };
                                let attacker_val = board
                                    .piece_on(m.from)
                                    .map(crate::eval::piece_value)
                                    .unwrap_or(100);
                                let promo_val =
                                    m.promotion.map(crate::eval::piece_value).unwrap_or(0);
                                let mut score =
                                    100_000 + victim_val * 10 - attacker_val + promo_val * 10;
                                if is_cap {
                                    score += shared.get_capture_history(color, m.from, m.to) / 32;
                                }
                                self.scores[i] = score;
                            }
                        }
                    }

                    let mut best_idx = None;
                    let mut best_score = i32::MIN;
                    for (i, &score) in self.scores[..self.stack.len].iter().enumerate() {
                        if score > best_score {
                            best_score = score;
                            best_idx = Some(i);
                        }
                    }

                    if let Some(idx) = best_idx {
                        self.scores[idx] = i32::MIN;
                        let m = self.stack.moves[idx];

                        let is_ep = board.piece_on(m.from) == Some(Piece::Pawn)
                            && m.from.file() != m.to.file()
                            && board.color_on(m.to).is_none();
                        let victim_val = if is_ep {
                            crate::eval::piece_value(Piece::Pawn)
                        } else {
                            board
                                .piece_on(m.to)
                                .map(crate::eval::piece_value)
                                .unwrap_or(0)
                        };
                        let attacker_val = board
                            .piece_on(m.from)
                            .map(crate::eval::piece_value)
                            .unwrap_or(100);

                        if victim_val > attacker_val || crate::search::see::see(board, m) >= 0 {
                            return Some((m, idx));
                        } else {
                            self.bad_captures.push(m);
                            let bc_idx = self.bad_captures.len - 1;
                            self.bad_capture_scores[bc_idx] = best_score;
                        }
                    } else {
                        self.stage = Stage::Killers;
                        self.current_index = 0;
                    }
                }
                Stage::Killers => {
                    if self.current_index < 2 {
                        let km = self.killers[self.current_index];
                        self.current_index += 1;
                        if let Some(km) = km {
                            if Some(km) != self.tt_move {
                                if self.current_index == 2 && self.killers[0] == Some(km) {
                                    continue;
                                }
                                if let Some(idx) =
                                    self.stack.as_slice().iter().position(|&m| m == km)
                                {
                                    let is_cap = board.color_on(km.to).is_some()
                                        || (board.piece_on(km.from) == Some(Piece::Pawn)
                                            && km.from.file() != km.to.file()
                                            && board.color_on(km.to).is_none());
                                    let is_promo = km.promotion.is_some();
                                    if !is_cap && !is_promo {
                                        return Some((km, idx));
                                    }
                                }
                            }
                        }
                    } else {
                        self.stage = Stage::Countermove;
                    }
                }
                Stage::Countermove => {
                    self.stage = Stage::Quiets;
                    if let Some(cm) = self.counter_move {
                        if Some(cm) != self.tt_move
                            && Some(cm) != self.killers[0]
                            && Some(cm) != self.killers[1]
                        {
                            if let Some(idx) = self.stack.as_slice().iter().position(|&m| m == cm) {
                                let is_cap = board.color_on(cm.to).is_some()
                                    || (board.piece_on(cm.from) == Some(Piece::Pawn)
                                        && cm.from.file() != cm.to.file()
                                        && board.color_on(cm.to).is_none());
                                let is_promo = cm.promotion.is_some();
                                if !is_cap && !is_promo {
                                    return Some((cm, idx));
                                }
                            }
                        }
                    }
                }
                Stage::Quiets => {
                    if !self.scored_quiets {
                        self.scored_quiets = true;
                        let color = board.side_to_move();
                        for (i, &m) in self.stack.as_slice().iter().enumerate() {
                            if Some(m) == self.tt_move
                                || self.killers.contains(&Some(m))
                                || Some(m) == self.counter_move
                            {
                                continue;
                            }
                            let is_cap = board.color_on(m.to).is_some()
                                || (board.piece_on(m.from) == Some(Piece::Pawn)
                                    && m.from.file() != m.to.file()
                                    && board.color_on(m.to).is_none());
                            let is_promo = m.promotion.is_some();
                            if !is_cap && !is_promo {
                                let mut score = shared.get_history(color, m.from, m.to);
                                if let Some(pm) = prev_move {
                                    score += shared.get_cont_history(pm.to, m.to);
                                }
                                self.scores[i] = score;
                            }
                        }
                    }

                    let mut best_idx = None;
                    let mut best_score = i32::MIN;
                    for (i, &score) in self.scores[..self.stack.len].iter().enumerate() {
                        if score != i32::MIN {
                            if best_idx.is_none() || score > best_score {
                                best_score = score;
                                best_idx = Some(i);
                            }
                        }
                    }

                    if let Some(idx) = best_idx {
                        self.scores[idx] = i32::MIN;
                        return Some((self.stack.moves[idx], idx));
                    } else {
                        self.stage = Stage::BadCaptures;
                        self.current_index = 0;
                    }
                }
                Stage::BadCaptures => {
                    let mut best_bc_idx = None;
                    let mut best_score = i32::MIN;
                    for (i, &score) in self.bad_capture_scores[..self.bad_captures.len]
                        .iter()
                        .enumerate()
                    {
                        if score > best_score {
                            best_score = score;
                            best_bc_idx = Some(i);
                        }
                    }

                    if let Some(bc_idx) = best_bc_idx {
                        self.bad_capture_scores[bc_idx] = i32::MIN;
                        let m = self.bad_captures.moves[bc_idx];
                        if let Some(idx) = self.stack.as_slice().iter().position(|&sm| sm == m) {
                            return Some((m, idx));
                        }
                    } else {
                        self.stage = Stage::Done;
                    }
                }
                Stage::Done => return None,
            }
        }
    }
}

#[inline]
pub(crate) fn score_from_tt(score: i32, ply: i32) -> i32 {
    if score > MATE_THRESHOLD {
        score - ply
    } else if score < -MATE_THRESHOLD {
        score + ply
    } else {
        score
    }
}

pub struct SearchInfo {
    pub start_time: Instant,
    pub time_limit: Duration,
    pub deadline: Option<Instant>,
    pub nodes: u64,
    pub aborted: bool,
    pub stop_flag: Arc<AtomicBool>,
    pub is_pondering: Arc<AtomicBool>,
    pub time_limit_ms: Arc<AtomicU64>,
    pub was_pondering: bool,
    pub killers: [[Option<Move>; 2]; MAX_PLY],
    pub eval_stack: [i32; MAX_PLY],
}

impl SearchInfo {
    pub fn new(
        time_limit: Duration,
        stop_flag: Arc<AtomicBool>,
        is_pondering: Arc<AtomicBool>,
        time_limit_ms: Arc<AtomicU64>,
    ) -> Self {
        let safe_limit = if time_limit > Duration::from_millis(25) {
            time_limit - Duration::from_millis(15)
        } else {
            (time_limit * 8) / 10
        };
        let deadline = Some(Instant::now() + safe_limit);
        Self {
            start_time: Instant::now(),
            time_limit,
            deadline,
            nodes: 0,
            aborted: false,
            stop_flag,
            is_pondering,
            time_limit_ms,
            was_pondering: false,
            killers: [[None; 2]; MAX_PLY],
            eval_stack: [0; MAX_PLY],
        }
    }

    #[inline(always)]
    pub fn get_killer(&self, ply: usize, slot: usize) -> Option<Move> {
        if ply < MAX_PLY && slot < 2 {
            self.killers[ply][slot]
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn update_killer(&mut self, ply: usize, m: Move) {
        if ply < MAX_PLY {
            if self.killers[ply][0] != Some(m) {
                self.killers[ply][1] = self.killers[ply][0];
                self.killers[ply][0] = Some(m);
            }
        }
    }

    #[inline(always)]
    pub fn set_eval(&mut self, ply: usize, eval: i32) {
        if ply < MAX_PLY {
            self.eval_stack[ply] = eval;
        }
    }

    #[inline(always)]
    pub fn get_eval(&self, ply: usize) -> i32 {
        if ply < MAX_PLY {
            self.eval_stack[ply]
        } else {
            0
        }
    }

    #[inline]
    pub fn check_time(&mut self) {
        if (self.nodes & 1023) == 0 {
            if self.stop_flag.load(Ordering::Relaxed) {
                self.aborted = true;
                return;
            }

            let currently_pondering = self.is_pondering.load(Ordering::Relaxed);

            if !currently_pondering && self.was_pondering {
                self.start_time = Instant::now();
                let sl = if self.time_limit > Duration::from_millis(25) {
                    info_safe_limit(self.time_limit)
                } else {
                    (self.time_limit * 8) / 10
                };
                self.deadline = Some(self.start_time + sl);
                self.was_pondering = false;
            }

            if currently_pondering {
                return;
            }

            if let Some(dl) = self.deadline {
                if Instant::now() >= dl {
                    self.aborted = true;
                    return;
                }
            }
        }
    }
}

#[inline(always)]
fn info_safe_limit(time_limit: Duration) -> Duration {
    if time_limit > Duration::from_millis(25) {
        time_limit - Duration::from_millis(15)
    } else {
        (time_limit * 8) / 10
    }
}
