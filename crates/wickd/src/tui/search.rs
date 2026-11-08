//! Local symbol search over the cached asset list.
//!
//! Ranking (lower is better): exact symbol, symbol prefix, a word in the
//! company name starting with the query, symbol substring, name substring.
//! Ties go to the shorter symbol, then alphabetical — so "app" ranks APP,
//! then APPF…, then Apple Inc. (AAPL) via its name.

use wickd_core::equities::Asset;

/// Max results returned to the UI.
pub const MAX_RESULTS: usize = 50;

fn score(asset: &Asset, q: &str) -> Option<u32> {
    let sym = asset.symbol.to_ascii_lowercase();
    if sym == q {
        return Some(0);
    }
    if sym.starts_with(q) {
        return Some(1);
    }
    let name = asset.name.to_ascii_lowercase();
    if name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| !w.is_empty() && w.starts_with(q))
    {
        return Some(2);
    }
    if sym.contains(q) {
        return Some(3);
    }
    if q.len() >= 3 && name.contains(q) {
        return Some(4);
    }
    None
}

/// Indices into `assets` of the best matches for `query`, best first.
pub fn search(assets: &[Asset], query: &str) -> Vec<usize> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u32, usize)> = assets
        .iter()
        .enumerate()
        .filter_map(|(i, a)| score(a, &q).map(|s| (s, i)))
        .collect();
    hits.sort_by(|(sa, ia), (sb, ib)| {
        let (a, b) = (&assets[*ia], &assets[*ib]);
        sa.cmp(sb)
            // Tradable names first within a tier (OTC/inactive oddities last).
            .then(b.tradable.cmp(&a.tradable))
            .then(a.symbol.len().cmp(&b.symbol.len()))
            .then(a.symbol.cmp(&b.symbol))
    });
    hits.into_iter().take(MAX_RESULTS).map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(sym: &str, name: &str) -> Asset {
        Asset {
            symbol: sym.into(),
            name: name.into(),
            exchange: "NASDAQ".into(),
            tradable: true,
        }
    }

    fn syms(assets: &[Asset], q: &str) -> Vec<String> {
        search(assets, q)
            .into_iter()
            .map(|i| assets[i].symbol.clone())
            .collect()
    }

    #[test]
    fn ranks_exact_then_prefix_then_name() {
        let assets = vec![
            a("AAPL", "Apple Inc. Common Stock"),
            a("APPF", "AppFolio, Inc."),
            a("APP", "AppLovin Corporation"),
            a("SNAP", "Snap Inc."),
        ];
        assert_eq!(syms(&assets, "app"), vec!["APP", "APPF", "AAPL"]);
        assert_eq!(syms(&assets, "aapl"), vec!["AAPL"]);
        assert_eq!(syms(&assets, "apple"), vec!["AAPL"]);
    }

    #[test]
    fn name_word_prefix_beats_symbol_substring() {
        let assets = vec![
            a("XNVDAX", "Something Else"),
            a("NVDA", "NVIDIA Corporation"),
        ];
        assert_eq!(syms(&assets, "nvidia"), vec!["NVDA"]);
        assert_eq!(syms(&assets, "nvda"), vec!["NVDA", "XNVDAX"]);
    }

    #[test]
    fn short_queries_skip_name_substrings_and_empty_is_empty() {
        let assets = vec![a("SPY", "SPDR S&P 500 ETF Trust")];
        // "pd" is a substring of "spdr" but too short for substring matching.
        assert!(syms(&assets, "pd").is_empty());
        assert_eq!(syms(&assets, "500"), vec!["SPY"]);
        assert!(search(&assets, "   ").is_empty());
    }

    #[test]
    fn tradable_first_within_tier() {
        let mut otc = a("ABCD", "Abcd Holdings");
        otc.tradable = false;
        let assets = vec![otc, a("ABCE", "Abce Corp")];
        assert_eq!(syms(&assets, "abc"), vec!["ABCE", "ABCD"]);
    }
}
