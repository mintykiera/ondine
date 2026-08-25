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
        let safe_time = our_time.saturating_sub(50);
        if safe_time < 100 {
            return Duration::from_millis(10);
        }

        let our_inc = parse_uci_param(tokens, inc_key).unwrap_or(0);
        let explicit_mtg = parse_uci_param(tokens, "movestogo");
        let mut mtg = explicit_mtg.unwrap_or(25);

        if explicit_mtg.is_none() && safe_time < 2000 {
            mtg = 40;
        }

        let base = safe_time / mtg.max(1);
        let target = base + (our_inc * 3) / 4;
        let max_time = if explicit_mtg.is_some() {
            safe_time / 2
        } else {
            safe_time / 4
        };
        let ms = target.min(max_time).max(10);

        return Duration::from_millis(ms);
    }

    if tokens.contains(&"infinite") {
        return Duration::from_secs(3600);
    }

    Duration::from_millis(1000)
}
