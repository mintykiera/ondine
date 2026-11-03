use cozy_chess::Color;
use std::time::Duration;

pub(crate) fn parse_uci_param(tokens: &[&str], name: &str) -> Option<u64> {
    tokens
        .iter()
        .position(|&r| r == name)
        .and_then(|i| tokens.get(i + 1))
        .and_then(|s| s.parse::<u64>().ok())
}

pub(crate) fn parse_total_clock(tokens: &[&str], side: Color) -> Option<Duration> {
    let time_key = if side == Color::White {
        "wtime"
    } else {
        "btime"
    };
    parse_uci_param(tokens, time_key).map(Duration::from_millis)
}

pub(crate) fn parse_go_time(tokens: &[&str], side: Color) -> Duration {
    if let Some(ms) = parse_uci_param(tokens, "movetime") {
        return Duration::from_millis(ms);
    }

    let time_key = if side == Color::White {
        "wtime"
    } else {
        "btime"
    };
    let inc_key = if side == Color::White { "winc" } else { "binc" };

    if let Some(our_time) = parse_uci_param(tokens, time_key) {
        // lichess-bot already deducts move_overhead (350ms) before passing wtime/btime to the engine.
        // A minimal 20ms safety margin is sufficient for internal process latency.
        let safe_time = our_time.saturating_sub(20);
        if safe_time < 50 {
            return Duration::from_millis(10);
        }

        let our_inc = parse_uci_param(tokens, inc_key).unwrap_or(0);

        if safe_time < 1000 {
            let ms = if our_inc > 0 {
                let inc_share = (our_inc * 3) / 4;
                let bank_share = safe_time / 4;
                (bank_share + inc_share).min(safe_time.saturating_sub(25)).max(10)
            } else {
                (safe_time / 6).max(10)
            };
            return Duration::from_millis(ms);
        }

        let explicit_mtg = parse_uci_param(tokens, "movestogo");
        let mtg = explicit_mtg.unwrap_or_else(|| {
            if safe_time < 2000 {
                50
            } else if safe_time < 10000 {
                45
            } else {
                40
            }
        });

        let base = safe_time / mtg.max(1);
        let target = if safe_time < 3000 {
            base + (our_inc * 3) / 5
        } else {
            base + (our_inc * 3) / 4
        };
        let max_time = if explicit_mtg.is_some() {
            safe_time / 2
        } else if our_inc > 0 {
            (safe_time / 3 + (our_inc * 3) / 4).min(safe_time.saturating_sub(50)).max(10)
        } else {
            (safe_time / 3).max(10)
        };
        let ms = target.min(max_time).max(10);

        return Duration::from_millis(ms);
    }

    if tokens.contains(&"infinite") {
        return Duration::from_secs(3600);
    }

    Duration::from_millis(1000)
}
