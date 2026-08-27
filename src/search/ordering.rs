use std::sync::LazyLock;

const LMR_MAX_DEPTH: usize = 65;
const LMR_MAX_MOVES: usize = 65;

static LMR_TABLE: LazyLock<[[i32; LMR_MAX_MOVES]; LMR_MAX_DEPTH]> = LazyLock::new(|| {
    let mut table = [[0i32; LMR_MAX_MOVES]; LMR_MAX_DEPTH];
    for d in 1..LMR_MAX_DEPTH {
        for m in 1..LMR_MAX_MOVES {
            table[d][m] = (0.75 + ((d as f64).ln() * (m as f64).ln()) / 2.25) as i32;
        }
    }
    table
});

#[inline]
pub(crate) fn lmr_reduction(depth: i32, move_index: i32) -> i32 {
    let d = (depth as usize).clamp(1, LMR_MAX_DEPTH - 1);
    let m = (move_index as usize).clamp(1, LMR_MAX_MOVES - 1);
    LMR_TABLE[d][m]
}
