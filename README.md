<h1 align="center">Ondine</h1>

<p align="center">
  <a href="LICENSE.md"><img src="https://img.shields.io/badge/License-All%20Rights%20Reserved-red.svg?style=flat-square" alt="License: All Rights Reserved"></a>
  <a href="https://lichess.org/@/LaOndine"><img src="https://img.shields.io/badge/Lichess-Play%20Ondine-orange?style=flat-square&logo=lichess" alt="Lichess"></a>
  <img src="https://img.shields.io/badge/Rust-2024%20Edition-black?style=flat-square&logo=rust" alt="Rust">
</p>

Ondine is a high-performance, multi-threaded UCI chess engine written in pure, memory-safe Rust on top of `cozy-chess`. It pairs a custom **[NNUE](https://www.chessprogramming.org/NNUE)** evaluation network with an optimized [alpha-beta search](https://www.chessprogramming.org/Alpha-Beta) pipeline featuring modern pruning, history-guided reductions, and lockless concurrency.

Play against Ondine on Lichess: [lichess.org/@/LaOndine](https://lichess.org/@/LaOndine)

## Architecture & Features

### Evaluation & Endgame Knowledge

- **Custom [NNUE](https://www.chessprogramming.org/NNUE) (`ondine.nnue`):** Efficiently Updatable Neural Network evaluated with incremental accumulator updates on move make/unmake via `nnue-rs`.
- **[Syzygy Tablebase](https://www.chessprogramming.org/Syzygy_Bases) Probing:**
  - **Root Probing:** Instant WDL & DTZ resolution for positions with ≤ 6 pieces, selecting optimal winning lines to convert endgames without searching.
  - **In-Tree Probing:** Depth-gated tablebase lookups to guarantee exact theoretical play in simplified branches.

### Search Pipeline

- **[Principal Variation Search (PVS)](https://www.chessprogramming.org/Principal_Variation_Search):** [Negamax](https://www.chessprogramming.org/Negamax) [alpha-beta search](https://www.chessprogramming.org/Alpha-Beta) with scout zero-window probing and full-depth re-searches.
- **[Dynamic Aspiration Windows](https://www.chessprogramming.org/Aspiration_Windows):** Tight initial search bounds (±20 cp) centered on the previous iteration score, geometrically widening on fail-high/low with full-window fallback.
- **Pruning & Reductions:**
  - **[Null Move Pruning (NMP)](https://www.chessprogramming.org/Null_Move_Pruning):** Adaptive depth reduction ($R = 3 + \text{depth} / 3 + \text{clamp}((\text{eval} - \beta) / 200,\, 0,\, 3)$) with [zugzwang](https://www.chessprogramming.org/Zugzwang) verification (non-pawn material check).
  - **[Reverse Futility Pruning (RFP)](https://www.chessprogramming.org/Reverse_Futility_Pruning):** Static evaluation margins at shallow depths.
  - **[Futility Pruning (FP)](https://www.chessprogramming.org/Futility_Pruning):** Prunes unpromising quiet moves near leaf nodes.
  - **[Late Move Pruning (LMP)](https://www.chessprogramming.org/Late_Move_Pruning):** Move count thresholds based on quadratic depth scaling ($3 + 2 \times \text{depth}^2$).
  - **[History-Adjusted Late Move Reductions (LMR)](https://www.chessprogramming.org/Late_Move_Reductions):** Base logarithmic reductions scaled dynamically by quiet history scores ($\text{reduction} - \text{history} / 4096$).
  - **[Static Exchange Evaluation (SEE)](https://www.chessprogramming.org/Static_Exchange_Evaluation):** Iterative exchange evaluation with x-ray discovery for capture verification and pruning.

- **Search Extensions:**
  - **[Check Extensions](https://www.chessprogramming.org/Check_Extensions):** Extends search depth when in check.
  - **[Singular Extensions](https://www.chessprogramming.org/Singular_Extensions):** Verifies critical TT moves by searching alternative candidate moves at reduced depth.
  - **[Internal Iterative Deepening (IID)](https://www.chessprogramming.org/Internal_Iterative_Deepening):** PV-node-gated shallow searches (≥ depth 6) to establish a hash move when TT lookups miss.

- **[Quiescence Search](https://www.chessprogramming.org/Quiescence_Search):** Tactical capture and promotion resolution with delta pruning, big-delta cutoffs, and SEE filtering.

### Move Ordering

Moves are ordered using an optimized 6-stage move picker:

1. **[Transposition Table](https://www.chessprogramming.org/Transposition_Table) Move:** Hash move from previous iterations.
2. **Good Captures:** [MVV-LVA](https://www.chessprogramming.org/MVV-LVA) sorted captures, boosted by Capture History, and validated with fast piece value checks or $\text{SEE} \ge 0$.
3. **[Killer Move Heuristic](https://www.chessprogramming.org/Killer_Heuristic):** 2 killer moves per ply (pseudo-legal and deduplicated).
4. **[Countermove Heuristic](https://www.chessprogramming.org/Countermove_Heuristic):** Refutation moves indexed against the opponent's previous move.
5. **Quiet Moves:** Scored using a 64×64 [Butterfly History](https://www.chessprogramming.org/History_Heuristic#Butterfly_History) Table and [Continuation History](https://www.chessprogramming.org/History_Heuristic#Continuation_History) Table with proportional gravity damping.
6. **Bad Captures:** Deferred losing captures ($\text{SEE} < 0$).

### Concurrency & System

- **[Lockless Lazy SMP](https://www.chessprogramming.org/Lazy_SMP):** Multi-threaded parallel search using a 4-way associative XOR-hashed [Transposition Table](https://www.chessprogramming.org/Transposition_Table) (`AtomicU64`) and asymmetric thread depth staggering with zero mutex overhead during search.
- **[Polyglot Opening Book](https://www.chessprogramming.org/PolyGlot):** Fast opening lookup integration.
- **Persistent Memory (`ondine_memory.bin`):** Automatic serialization and restoration of Transposition Table entries across sessions.
- **Adaptive [Time Management](https://www.chessprogramming.org/Time_Management):** Dynamic allocation with panic buffers on sharp score drops, soft/hard time margins, and ponderhit support.

---

## Author

Made with ♟️ by **mintykiera**

## License

Copyright © 2026 mintykiera. All rights reserved. See [`LICENSE.md`](LICENSE.md) for terms.
