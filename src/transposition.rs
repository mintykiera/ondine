use cozy_chess::{Move, Piece, Square};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Result as IoResult, Write};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NodeType {
    Exact,
    LowerBound,
    UpperBound,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct TTEntry {
    pub hash: u64,
    pub depth: i32,
    pub score: i32,
    pub node_type: NodeType,
    pub best_move: Option<Move>,
    pub age: u8,
}

impl Default for TTEntry {
    fn default() -> Self {
        Self {
            hash: 0,
            depth: 0,
            score: 0,
            node_type: NodeType::Exact,
            best_move: None,
            age: 0,
        }
    }
}

pub struct LocklessEntry {
    pub data1: AtomicU64,
    pub data2: AtomicU64,
}

impl Default for LocklessEntry {
    fn default() -> Self {
        Self {
            data1: AtomicU64::new(0),
            data2: AtomicU64::new(0),
        }
    }
}

pub const BUCKET_SIZE: usize = 4;

#[derive(Default)]
pub struct LocklessBucket {
    pub entries: [LocklessEntry; BUCKET_SIZE],
}

pub fn pack_move(m: Option<Move>) -> u16 {
    match m {
        Some(mv) => {
            let from = mv.from as u16;
            let to = mv.to as u16;
            let promo = match mv.promotion {
                Some(Piece::Knight) => 1,
                Some(Piece::Bishop) => 2,
                Some(Piece::Rook) => 3,
                Some(Piece::Queen) => 4,
                _ => 0,
            };
            (promo << 12) | (from << 6) | to | 0x8000
        }
        None => 0,
    }
}

pub fn unpack_move(data: u16) -> Option<Move> {
    if (data & 0x8000) == 0 {
        return None;
    }
    let to = Square::index((data & 0x3f) as usize);
    let from = Square::index(((data >> 6) & 0x3f) as usize);
    let promo = match (data >> 12) & 0x7 {
        1 => Some(Piece::Knight),
        2 => Some(Piece::Bishop),
        3 => Some(Piece::Rook),
        4 => Some(Piece::Queen),
        _ => None,
    };
    Some(Move {
        from,
        to,
        promotion: promo,
    })
}

fn pack_data2(score: i32, depth: i32, mv: Option<Move>, node_type: NodeType, age: u8) -> u64 {
    let score_u32 = score as u32 as u64;
    let depth_u8 = depth.clamp(0, 255) as u64;
    let mv_u16 = pack_move(mv) as u64;
    let nt_u8 = (match node_type {
        NodeType::Exact => 0,
        NodeType::LowerBound => 1,
        NodeType::UpperBound => 2,
    }) as u64;
    let age_u8 = (age & 0x3f) as u64;

    (age_u8 << 58) | (nt_u8 << 56) | (mv_u16 << 40) | (depth_u8 << 32) | score_u32
}

fn unpack_data2(data: u64) -> (i32, i32, Option<Move>, NodeType, u8) {
    let score = (data & 0xffffffff) as u32 as i32;
    let depth = ((data >> 32) & 0xff) as i32;
    let mv = unpack_move(((data >> 40) & 0xffff) as u16);
    let node_type = match (data >> 56) & 0x3 {
        0 => NodeType::Exact,
        1 => NodeType::LowerBound,
        _ => NodeType::UpperBound,
    };
    let age = ((data >> 58) & 0x3f) as u8;

    (score, depth, mv, node_type, age)
}

pub struct TranspositionTable {
    table: Vec<LocklessBucket>,
    mask: usize,
    pub age: AtomicU8,
}

impl TranspositionTable {
    pub fn new(size_mb: usize) -> Self {
        let count = (size_mb * 1024 * 1024) / (std::mem::size_of::<LocklessEntry>() * BUCKET_SIZE);
        let count = if count < 2 {
            1
        } else {
            1usize << (usize::BITS - 1 - count.leading_zeros())
        };

        let mut table = Vec::with_capacity(count);
        for _ in 0..count {
            table.push(LocklessBucket::default());
        }

        Self {
            table,
            mask: count - 1,
            age: AtomicU8::new(0),
        }
    }

    #[inline]
    fn index(&self, hash: u64) -> usize {
        (hash as usize) & self.mask
    }

    pub fn get(&self, hash: u64) -> Option<TTEntry> {
        let idx = self.index(hash);
        let bucket = &self.table[idx];

        for entry in &bucket.entries {
            let hash_pre = entry.data1.load(Ordering::Acquire);
            if hash_pre != hash {
                continue;
            }

            let d2 = entry.data2.load(Ordering::Acquire);
            let raw_d2 = d2 ^ hash;

            let hash_post = entry.data1.load(Ordering::Acquire);

            if hash_post == hash {
                let (score, depth, best_move, node_type, age) = unpack_data2(raw_d2);
                return Some(TTEntry {
                    hash,
                    depth,
                    score,
                    node_type,
                    best_move,
                    age,
                });
            }
        }
        None
    }

    pub fn insert(
        &self,
        hash: u64,
        depth: i32,
        score: i32,
        node_type: NodeType,
        best_move: Option<Move>,
    ) {
        let bucket = &self.table[self.index(hash)];
        let current_age = self.age.load(Ordering::Relaxed);
        let d2 = pack_data2(score, depth, best_move, node_type, current_age);
        let xor_d2 = d2 ^ hash;

        for entry in &bucket.entries {
            if entry.data1.load(Ordering::Acquire) == hash {
                let old_d2 = entry.data2.load(Ordering::Acquire) ^ hash;
                let (_, _, old_mv, _, _) = unpack_data2(old_d2);
                let move_to_store = best_move.or(old_mv);
                let new_d2 = pack_data2(score, depth, move_to_store, node_type, current_age);
                let new_xor_d2 = new_d2 ^ hash;

                entry.data1.store(0, Ordering::Release);
                entry.data2.store(new_xor_d2, Ordering::Release);
                entry.data1.store(hash, Ordering::Release);
                return;
            }
        }

        let mut best_entry = None;
        let mut best_score = i64::MAX;

        for entry in &bucket.entries {
            let hash_old = entry.data1.load(Ordering::Acquire);
            if hash_old == 0 {
                best_entry = Some(entry);
                break;
            }

            let d2_old = entry.data2.load(Ordering::Acquire) ^ hash_old;
            let (_, old_depth, _, _, old_age) = unpack_data2(d2_old);

            let score =
                (old_depth as i64) - (((current_age.wrapping_sub(old_age) & 0x3f) as i64) * 1000);
            if score < best_score {
                best_score = score;
                best_entry = Some(entry);
            }
        }

        if let Some(entry) = best_entry {
            entry.data1.store(0, Ordering::Release);
            entry.data2.store(xor_d2, Ordering::Release);
            entry.data1.store(hash, Ordering::Release);
        }
    }

    pub fn new_search(&self) {
        self.age.fetch_add(1, Ordering::Relaxed);
    }

    pub fn clear(&self) {
        for bucket in &self.table {
            for entry in &bucket.entries {
                entry.data1.store(0, Ordering::Relaxed);
                entry.data2.store(0, Ordering::Relaxed);
            }
        }
        self.age.store(0, Ordering::Relaxed);
    }

    pub fn save_to_file(&self, path: &str) -> IoResult<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);

        for bucket in &self.table {
            for entry in &bucket.entries {
                let d1 = entry.data1.load(Ordering::Relaxed);
                let d2 = entry.data2.load(Ordering::Relaxed);
                writer.write_all(&d1.to_le_bytes())?;
                writer.write_all(&d2.to_le_bytes())?;
            }
        }
        writer.flush()?;
        Ok(())
    }

    pub fn load_from_file(&self, path: &str) -> IoResult<()> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut buf1 = [0u8; 8];
        let mut buf2 = [0u8; 8];

        for bucket in &self.table {
            for entry in &bucket.entries {
                if reader.read_exact(&mut buf1).is_ok() && reader.read_exact(&mut buf2).is_ok() {
                    let d1 = u64::from_le_bytes(buf1);
                    let d2 = u64::from_le_bytes(buf2);

                    let actual_hash = d1;
                    if actual_hash != 0 {
                        let actual_d2 = d2 ^ actual_hash;
                        let (score, depth, mv, nt, _age) = unpack_data2(actual_d2);
                        let new_d2 = pack_data2(score, depth, mv, nt, 0);
                        let new_stored_d2 = new_d2 ^ actual_hash;

                        entry.data2.store(new_stored_d2, Ordering::Relaxed);
                        entry.data1.store(actual_hash, Ordering::Relaxed);
                    } else {
                        entry.data2.store(0, Ordering::Relaxed);
                        entry.data1.store(0, Ordering::Relaxed);
                    }
                } else {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}
