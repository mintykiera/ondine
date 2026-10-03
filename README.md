<h1 align="center">Ondine</h1>

<p align="center">
  <a href="LICENSE.md"><img src="https://img.shields.io/badge/License-All%20Rights%20Reserved-red.svg?style=flat-square" alt="License: All Rights Reserved"></a>
  <a href="https://lichess.org/@/LaOndine"><img src="https://img.shields.io/badge/Lichess-Play%20Ondine-orange?style=flat-square&logo=lichess" alt="Lichess"></a>
  <img src="https://img.shields.io/badge/Rust-2024%20Edition-black?style=flat-square&logo=rust" alt="Rust">
</p>

Ondine is a high-performance, multi-threaded UCI chess engine written in pure, memory-safe Rust on top of `cozy-chess`. It pairs a custom **[NNUE](https://www.chessprogramming.org/NNUE)** evaluation network with an optimized [alpha-beta search](https://www.chessprogramming.org/Alpha-Beta) pipeline featuring modern pruning, history-guided reductions, and lockless concurrency.

Play against Ondine on Lichess: [lichess.org/@/LaOndine](https://lichess.org/@/LaOndine)

---

## Architecture & Features

### Evaluation & Endgame Knowledge

- **[NNUE](https://www.chessprogramming.org/NNUE) Evaluation (`ondine.nnue`):** Efficiently Updatable Neural Network evaluated with incremental accumulator updates on move make/unmake via `nnue-rs`. Supports modern architectures (HalfKAv2, HalfKAv2_hm, SFNNv10) with automatic score normalization to standard centipawns.
- **[Syzygy Tablebase](https://www.chessprogramming.org/Syzygy_Bases) Probing:**
  - **Root Probing:** Instant WDL & DTZ resolution for positions with ≤ 6 pieces, selecting optimal winning lines to convert endgames without searching.
  - **In-Tree Probing:** Depth-gated tablebase lookups to guarantee exact theoretical play in simplified branches.

### Search Pipeline

- **[Principal Variation Search (PVS)](https://www.chessprogramming.org/Principal_Variation_Search):** [Negamax](https://www.chessprogramming.org/Negamax) [alpha-beta search](https://www.chessprogramming.org/Alpha-Beta) with scout zero-window probing and full-depth re-searches.
- **[Dynamic Aspiration Windows](https://www.chessprogramming.org/Aspiration_Windows):** Tight initial search bounds (±20 cp) centered on the previous iteration score at `depth >= 4`, geometrically widening on fail-high/low with full-window fallback.
- **Pruning & Reductions:**
  - **[Null Move Pruning (NMP)](https://www.chessprogramming.org/Null_Move_Pruning):** Adaptive depth reduction (`R = 3 + depth / 3 + clamp((eval - β) / 200, 0, 3)`) with [zugzwang](https://www.chessprogramming.org/Zugzwang) verification (non-pawn material check) and high-depth verification searches (`depth >= 12`).
  - **[Reverse Futility Pruning (RFP)](https://www.chessprogramming.org/Reverse_Futility_Pruning):** Static evaluation cutoffs at shallow depths (`depth <= 7`, margin = 60 cp if improving, 80 cp otherwise).
  - **[Razoring](https://www.chessprogramming.org/Razoring):** Quiescence verification when static evaluation falls far below alpha at shallow depths (`depth <= 3`, margin = `150 * depth`).
  - **[Futility Pruning (FP)](https://www.chessprogramming.org/Futility_Pruning):** Prunes unpromising quiet moves near leaf nodes (`static_eval + depth * 100 <= α` for `depth <= 4`) while strictly exempting the first quiet move, killer moves, and countermoves.
  - **[Late Move Pruning (LMP)](https://www.chessprogramming.org/Late_Move_Pruning):** Move count thresholds based on quadratic depth scaling (`(4 + 2 * depth²) / (2 - improving)` for `depth <= 4`), exempting killer and counter moves.
  - **[History-Adjusted Late Move Reductions (LMR)](https://www.chessprogramming.org/Late_Move_Reductions):** Logarithmic base reductions dynamically softened or deepened by quiet history scores and improving flag (`reduction - history / 4096 - improving`).
  - **[Static Exchange Evaluation (SEE)](https://www.chessprogramming.org/Static_Exchange_Evaluation):** Iterative exchange evaluation with x-ray discovery for capture verification and pruning.

- **Search Extensions & Reductions:**
  - **[Check Extensions](https://www.chessprogramming.org/Check_Extensions):** 1-ply search depth extension when in check.
  - **[Singular Extensions](https://www.chessprogramming.org/Singular_Extensions):** Verifies critical TT moves by searching alternative candidate moves at reduced depth `(depth - 1) / 2`; extends depth if the TT move is uniquely superior.
  - **[Internal Iterative Reduction (IIR)](https://www.chessprogramming.org/Internal_Iterative_Reduction):** 1-ply search depth reduction at `depth >= 4` when no TT move is available.

- **[Quiescence Search](https://www.chessprogramming.org/Quiescence_Search):** Tactical capture and promotion resolution featuring stand-pat evaluation, TT probing, big-delta cutoffs (1800 cp), delta pruning (200 cp margin), and SEE capture filtering (`SEE < 0`).

### Move Ordering

Moves are ordered using an optimized 6-stage move picker:

1. **[Transposition Table](https://www.chessprogramming.org/Transposition_Table) Move:** Hash move from previous iterations or shallower searches.
2. **Good Captures:** [MVV-LVA](https://www.chessprogramming.org/MVV-LVA) sorted captures, boosted by Capture History, and validated with fast piece value checks or `SEE >= 0`.
3. **[Killer Move Heuristic](https://www.chessprogramming.org/Killer_Heuristic):** 2 killer moves per ply (pseudo-legal and deduplicated).
4. **[Countermove Heuristic](https://www.chessprogramming.org/Countermove_Heuristic):** Refutation moves indexed against the opponent's previous move.
5. **Quiet Moves:** Scored using a 64×64 [Butterfly History](https://www.chessprogramming.org/History_Heuristic#Butterfly_History) Table and [Continuation History](https://www.chessprogramming.org/History_Heuristic#Continuation_History) Table with proportional gravity damping.
6. **Bad Captures:** Deferred losing captures (`SEE < 0`).

### Concurrency, Memory & Deployment

- **[Lockless Lazy SMP](https://www.chessprogramming.org/Lazy_SMP):** Multi-threaded parallel search using a 4-way associative XOR-hashed [Transposition Table](https://www.chessprogramming.org/Transposition_Table) (`AtomicU64`) and asymmetric thread depth staggering with zero mutex overhead during search. Complete table clearing on `ucinewgame`.
- **Low-Footprint Cloud Architecture:** Default 64 MB Transposition Table (configurable via UCI `Hash`) tuned for 512 MB memory constraints (Heroku Dynos, VPS containers) with zero swap thrashing.
- **Persistent Memory (`ondine_memory.bin`):** Automatic serialization and restoration of Transposition Table entries across sessions, with cryptographic NNUE network fingerprinting (`ondine_memory.sig`) to discard stale tables when weights update.
- **[Polyglot Opening Book](https://www.chessprogramming.org/PolyGlot) (`book.bin`):** Embedded Polyglot binary book reader with resilient directory fallback for instant, high-quality opening moves.
- **Cloud-Hardened [Time Management](https://www.chessprogramming.org/Time_Management):** Dynamic time allocation with a 150 ms network safety margin against cloud latency, panic extensions on sharp score drops, and synchronized atomic limits to avoid iterative deepening extension desyncs.

---

## Author

Made with ♟️ by **mintykiera**

## License

Copyright © 2026 mintykiera. All rights reserved. See [`LICENSE.md`](LICENSE.md) for terms.
