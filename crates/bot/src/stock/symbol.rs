//! Symbol normalization for TWSE / TPEx / US tickers. **Pure** — no network,
//! no DB. Two-stage Taiwan disambiguation (probe `.TW` then `.TWO`) lives in
//! `StockService::resolve`; this module only classifies the *shape* of a raw
//! string and, when the market is unambiguous, produces a canonical `Symbol`.
//!
//! The whitelist in [`passes_whitelist`] is a **security gate, not hygiene**:
//! the raw string is later interpolated into a Yahoo URL *path*, and the
//! canonical form is packed into Telegram `callback_data` whose fields are
//! `:`-separated. Every accepted symbol is therefore guaranteed to be ASCII,
//! at most 12 bytes, and free of `:`, `/`, and whitespace — the property the
//! `symbols_never_contain_the_callback_separator` fuzz test pins down.

/// Which schedule bucket a symbol belongs to. Wire form (`callback_data`, DB
/// `market` column): `"tw"` | `"us"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Market {
    Tw,
    Us,
}

impl Market {
    pub fn as_wire(self) -> &'static str {
        match self {
            Market::Tw => "tw",
            Market::Us => "us",
        }
    }

    pub fn from_wire(s: &str) -> Option<Market> {
        match s {
            "tw" => Some(Market::Tw),
            "us" => Some(Market::Us),
            _ => None,
        }
    }
}

/// Finer-grained exchange board. Wire form: `"twse"` | `"tpex"` | `"us"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    Twse,
    Tpex,
    Us,
}

impl Board {
    pub fn as_wire(self) -> &'static str {
        match self {
            Board::Twse => "twse",
            Board::Tpex => "tpex",
            Board::Us => "us",
        }
    }

    pub fn from_wire(s: &str) -> Option<Board> {
        match s {
            "twse" => Some(Board::Twse),
            "tpex" => Some(Board::Tpex),
            "us" => Some(Board::Us),
            _ => None,
        }
    }

    pub fn market(self) -> Market {
        match self {
            Board::Twse | Board::Tpex => Market::Tw,
            Board::Us => Market::Us,
        }
    }

    /// The Yahoo suffix for a Taiwan board (`".TW"` / `".TWO"`), or `""` for US.
    pub fn yahoo_suffix(self) -> &'static str {
        match self {
            Board::Twse => ".TW",
            Board::Tpex => ".TWO",
            Board::Us => "",
        }
    }
}

/// A fully-classified symbol. `canonical` is exactly what gets sent to Yahoo
/// and stored in the DB (e.g. `2330.TW`, `6488.TWO`, `AAPL`, `BRK-B`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub canonical: String,
    pub market: Market,
    pub board: Board,
    /// The bare local code without market suffix (`2330`, `6488`, `AAPL`).
    pub local_code: String,
}

/// Result of [`parse`]. A bare 4–6 digit Taiwan code cannot be assigned to a
/// board from its shape alone (both TWSE and TPEx use the same numbering), so
/// it is returned as [`Parsed::TaiwanAmbiguous`] for the service layer to probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Resolved(Symbol),
    TaiwanAmbiguous { local_code: String },
    Invalid(&'static str),
}

/// Classifies a raw user string. Never performs IO.
pub fn parse(raw: &str) -> Parsed {
    let trimmed = raw.trim().trim_start_matches('$').trim();
    let up = trimmed.to_ascii_uppercase();

    // Security gate first. Everything downstream trusts this.
    if !passes_whitelist(&up) {
        return Parsed::Invalid("代號格式不正確");
    }

    // Explicit market suffix wins. Check `.TWO` before `.TW` for clarity even
    // though the suffixes are not prefixes of one another.
    if let Some(code) = up.strip_suffix(".TWO") {
        return if is_tw_code(code) {
            Parsed::Resolved(Symbol {
                canonical: up.clone(),
                market: Market::Tw,
                board: Board::Tpex,
                local_code: code.to_owned(),
            })
        } else {
            Parsed::Invalid("台股代號格式不正確")
        };
    }
    if let Some(code) = up.strip_suffix(".TW") {
        return if is_tw_code(code) {
            Parsed::Resolved(Symbol {
                canonical: up.clone(),
                market: Market::Tw,
                board: Board::Twse,
                local_code: code.to_owned(),
            })
        } else {
            Parsed::Invalid("台股代號格式不正確")
        };
    }

    // Bare Taiwan code: 4–6 digits with an optional single trailing letter
    // (`00400A` is a real TWSE symbol). The board is genuinely unknown here.
    if is_tw_code(&up) {
        return Parsed::TaiwanAmbiguous { local_code: up };
    }

    // Otherwise treat as US. The two Taiwan suffixes were already consumed, so
    // rewriting `.` to `-` here is safe (`BRK.B` -> `BRK-B`, which Yahoo serves).
    if is_us_ticker(&up) {
        let canonical = up.replace('.', "-");
        return Parsed::Resolved(Symbol {
            canonical: canonical.clone(),
            market: Market::Us,
            board: Board::Us,
            local_code: canonical,
        });
    }

    Parsed::Invalid("無法辨識的代號")
}

/// `^[A-Z0-9][A-Z0-9.^-]{0,11}$` — hand-rolled to avoid a `regex` dependency.
/// Guarantees ASCII, length 1..=12, and the absence of `:`, `/`, whitespace.
fn passes_whitelist(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 12 {
        return false;
    }
    if !(bytes[0].is_ascii_uppercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b'.' | b'^' | b'-'))
}

/// `^[0-9]{4,6}[A-Z]?$`
fn is_tw_code(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let digits = if bytes[bytes.len() - 1].is_ascii_uppercase() {
        &bytes[..bytes.len() - 1]
    } else {
        bytes
    };
    (4..=6).contains(&digits.len()) && digits.iter().all(u8::is_ascii_digit)
}

/// `^[A-Z][A-Z0-9.-]{0,9}$`
fn is_us_ticker(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes.len() > 10 || !bytes[0].is_ascii_uppercase() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(raw: &str) -> Symbol {
        match parse(raw) {
            Parsed::Resolved(s) => s,
            other => panic!("expected Resolved for {raw:?}, got {other:?}"),
        }
    }

    #[test]
    fn explicit_taiwan_suffixes_pick_the_board() {
        let twse = resolved("2330.TW");
        assert_eq!(twse.canonical, "2330.TW");
        assert_eq!(twse.board, Board::Twse);
        assert_eq!(twse.market, Market::Tw);
        assert_eq!(twse.local_code, "2330");

        let tpex = resolved("6488.TWO");
        assert_eq!(tpex.board, Board::Tpex);
        assert_eq!(tpex.local_code, "6488");
    }

    #[test]
    fn a_bare_taiwan_code_is_ambiguous() {
        assert_eq!(
            parse("2330"),
            Parsed::TaiwanAmbiguous {
                local_code: "2330".to_owned()
            }
        );
    }

    #[test]
    fn taiwan_codes_with_a_trailing_letter_are_accepted() {
        // 00400A is a real TWSE symbol; pin it down.
        assert_eq!(
            parse("00400A"),
            Parsed::TaiwanAmbiguous {
                local_code: "00400A".to_owned()
            }
        );
        assert_eq!(resolved("00400A.TW").board, Board::Twse);
    }

    #[test]
    fn us_ticker_dot_becomes_dash() {
        let s = resolved("brk.b");
        assert_eq!(s.canonical, "BRK-B");
        assert_eq!(s.market, Market::Us);
        assert_eq!(s.board, Board::Us);
    }

    #[test]
    fn leading_dollar_and_whitespace_are_stripped() {
        assert_eq!(resolved("  $aapl ").canonical, "AAPL");
    }

    #[test]
    fn junk_is_invalid_not_a_panic() {
        assert!(matches!(parse(""), Parsed::Invalid(_)));
        assert!(matches!(parse("   "), Parsed::Invalid(_)));
        assert!(matches!(parse("a b"), Parsed::Invalid(_)));
        assert!(matches!(parse("../../v7/quote"), Parsed::Invalid(_)));
        assert!(matches!(parse("2330:evil"), Parsed::Invalid(_)));
        assert!(matches!(parse("WAYTOOLONGTICKER"), Parsed::Invalid(_)));
    }

    #[test]
    fn symbols_never_contain_the_callback_separator() {
        // The whole point of the whitelist: any Resolved.canonical is safe to
        // embed in `:`-separated callback_data and in a URL path. Fuzz a wide
        // byte alphabet (deterministic LCG, no `rand` dependency) and assert
        // the invariant on every accepted symbol.
        const ALPHABET: &[u8] = b"AZ09.-^:/ $abz\t\n";
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };

        for _ in 0..10_000 {
            let len = (next() % 20) as usize;
            let raw: String = (0..len)
                .map(|_| ALPHABET[(next() as usize) % ALPHABET.len()] as char)
                .collect();

            let canon = match parse(&raw) {
                Parsed::Resolved(s) => Some(s.canonical),
                Parsed::TaiwanAmbiguous { local_code } => Some(local_code),
                Parsed::Invalid(_) => None,
            };
            if let Some(c) = canon {
                assert!(c.is_ascii(), "non-ascii canonical from {raw:?}: {c:?}");
                assert!(c.len() <= 16, "oversized canonical from {raw:?}: {c:?}");
                assert!(
                    !c.contains([':', '/', ' ', '\t', '\n']),
                    "unsafe canonical from {raw:?}: {c:?}"
                );
            }
        }
    }

    #[test]
    fn wire_round_trips() {
        for m in [Market::Tw, Market::Us] {
            assert_eq!(Market::from_wire(m.as_wire()), Some(m));
        }
        for b in [Board::Twse, Board::Tpex, Board::Us] {
            assert_eq!(Board::from_wire(b.as_wire()), Some(b));
        }
        assert_eq!(Board::Twse.market(), Market::Tw);
        assert_eq!(Board::Tpex.market(), Market::Tw);
        assert_eq!(Board::Us.market(), Market::Us);
    }
}
