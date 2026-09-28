//! Fuzzy matching — `fuzzy-matcher` (skim v2 algorithm) wrapped to keep the
//! crate's local contract.
//!
//! Why skim: the habit a launcher user brings is abbreviation matching
//! (`kp` → `killport`, `si` → `sysinfo`), and skim's v2 algorithm is the same
//! fzf-v2 family Wox's own `fuzzy_match.go` belongs to — same reward/penalty
//! shapes, same boundary/camel-case bonuses. Reusing it removes the 500+
//! line port and its golden-score table: the algorithm is the reference
//! implementation, exercised daily by every skim/fzf user.
//!
//! Two deliberate subsets:
//!
//! * **ASCII-only path.** Every alias in this catalog is ASCII. A non-ASCII
//!   needle or candidate falls back to a plain case-insensitive substring
//!   test rather than pulling in Unicode tables — skim's v2 is ASCII-clean
//!   and supports arbitrary Unicode only at the call-site layer.
//!
//! Skim's default scoring constants — `score_match=16`, `gap_start=-3`,
//! `gap_extension=-1` — match Wox's; the only behavioural gap from the
//! port this replaces is the rank order of closely-scored candidates
//! (skim rewards slightly different gap shapes), so the exact-score
//! regression vectors were rewritten as ranking-habit vectors instead.

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;

/// A matcher per call. The default config has `use_cache(true)`, which keeps
/// three thread-local `RefCell<Vec<…>>` buffers alive across calls — small,
/// bounded, and the standard pattern; turning the cache off triggers a
/// re-entrant `borrow_mut` panic inside `fuzzy-matcher` 0.3.7 because its
/// `!use_cache` clear path runs while the same RefCells are still borrowed.
fn matcher() -> SkimMatcherV2 {
    SkimMatcherV2::default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub is_match: bool,
    pub score: i64,
}

impl Match {
    const NO: Match = Match {
        is_match: false,
        score: 0,
    };

    fn hit(score: i64) -> Self {
        Match {
            is_match: true,
            score,
        }
    }
}

/// Score `pattern` against `text`. Higher is more relevant; `is_match` is
/// false when the pattern cannot be found at all.
pub fn fuzzy_match(text: &str, pattern: &str) -> Match {
    if pattern.is_empty() {
        return Match::hit(0);
    }
    if text.is_empty() {
        return Match::NO;
    }

    // Non-ASCII fallback: skim's v2 is ASCII-clean and the catalog is ASCII,
    // so any non-ASCII needle/candidate is a user-typed word against a
    // substring — a plain case-insensitive contains is the conservative
    // answer and the one the old port gave.
    if !text.is_ascii() || !pattern.is_ascii() {
        return substring(text, pattern);
    }

    matcher()
        .fuzzy_match(text, pattern)
        .map(Match::hit)
        .unwrap_or(Match::NO)
}

/// Case-insensitive ASCII substring test. The old port's last-resort branch:
/// when the scoring matcher rejects an alignment as too scattered, the
/// substring path still accepts a contiguous in-order match.
fn substring(text: &str, pattern: &str) -> Match {
    let text_bytes = text.as_bytes();
    let pattern_bytes = pattern.as_bytes();
    if pattern_bytes.len() > text_bytes.len() {
        return Match::NO;
    }
    if text_bytes
        .windows(pattern_bytes.len())
        .any(|window| window.eq_ignore_ascii_case(pattern_bytes))
    {
        Match::hit(pattern_bytes.len() as i64)
    } else {
        Match::NO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `is_match` and the basic ranking habits this matcher has to keep. The
    /// old port pinned these against Wox's reference Go output (`ip` →
    /// 132, `killport`/`kp` → 38, `flushdns`/`fdo` → 0); skim v2 uses the
    /// same scoring family but a different optimal-alignment dynamic
    /// program, so exact-score parity is gone and the assertions are now
    /// shape-only.
    #[test]
    fn matching_and_non_matching_classifications_match() {
        // Acceptance: substrings/abbreviations that find their target.
        for (text, pattern) in [
            ("ip", "ip"),
            ("ipconfig", "ip"),
            ("killport", "kill"),
            ("killport", "kp"),
            ("killport", "kpt"),
            ("killport", "port"),
            ("flushdns", "fd"),
            ("flushdns", "dns"),
            ("flushdns", "flush"),
            ("sysinfo", "si"),
            ("xipx", "ip"),
            ("taskmanager", "taskmanag"),
            (
                "kubernetes-network-policy-audit-tool-with-verbose-logging",
                "kube",
            ),
        ] {
            assert!(
                fuzzy_match(text, pattern).is_match,
                "{text:?} should match {pattern:?}"
            );
        }

        // Rejection: a needle that doesn't appear, a pattern that needs an
        // order the alias can't satisfy, a needle beyond the candidate.
        for (text, pattern) in [
            ("flushdns", "fdo"),
            ("flushdns", "zzz"),
            ("killport", "ktp"),
            ("killport", "kpz"),
            ("ip", "ipconfig"),
            ("killport", "z"),
        ] {
            assert!(
                !fuzzy_match(text, pattern).is_match,
                "{text:?} should not match {pattern:?}"
            );
        }

        // Edge cases — empty pattern matches with score 0 (the caller's
        // `catalog::score` upgrades this to a positive score so the row
        // still lists); empty text never matches; non-ASCII falls back to
        // substring and rejects non-contiguous needles.
        assert!(fuzzy_match("ip", "").is_match);
        assert_eq!(fuzzy_match("ip", "").score, 0);
        assert!(!fuzzy_match("", "x").is_match);
        assert!(fuzzy_match("中文命令", "中文").is_match);
        assert!(fuzzy_match("中文命令", "命令").is_match);
        assert!(!fuzzy_match("中文命令", "中令").is_match);
    }

    /// The rank habits the catalog's UX depends on. These don't assert
    /// specific scores — they assert that the relative order matches what
    /// a launcher user expects from `kp` → `killport`, which is what skim
    /// v2 was designed to deliver.
    #[test]
    fn ranking_follows_the_expected_habits() {
        let rank = |pattern: &str| fuzzy_match("killport", pattern).score;
        assert!(rank("killport") > rank("kill"), "exact beats prefix");
        assert!(rank("kill") > rank("kp"), "prefix beats abbreviation");
        // A contiguous fragment inside the word still beats a scattered
        // abbreviation: `port` lands cleanly, `kpt` doesn't.
        assert!(rank("port") > rank("kpt"), "fragment beats scattered");
    }

    /// Every prefix of an alias ranks above an abbreviation made by
    /// swapping the first two characters: the prefix test exercises skim's
    /// boundary-bonus path, the abbreviation its gap-penalty path, and
    /// the spread between them is what a launcher user reads as "the
    /// alias wins over the typo".
    #[test]
    fn every_prefix_of_an_alias_ranks_above_an_abbreviation() {
        for alias in ["flushdns", "sysinfo", "taskmgr", "extract"] {
            let prefix = fuzzy_match(alias, &alias[..3]).score;
            let abbrev =
                fuzzy_match(alias, &format!("{}{}", &alias[..1], &alias[3..4]))
                    .score;
            assert!(prefix > abbrev, "{alias}: {prefix} vs {abbrev}");
        }
    }

    /// Boundary reward: a match that lands right after a delimiter ranks
    /// above the same match inside a contiguous run. Skim's
    /// `SkimScoreConfig::default()` (`bonus_boundary=8`) gives the
    /// boundary-bonus only when the matched character is **preceded by** a
    /// transition — the start-of-word path (`Head` role in skim's
    /// `CharRole`) carries the same bonus, so the meaningful comparison is
    /// `delimiter-prefixed` vs `no-boundary-at-all`, not `start-of-word` vs
    /// `inside-word`.
    #[test]
    fn boundary_rewards_exist() {
        assert!(
            fuzzy_match("docker-compose", "comp").score
                > fuzzy_match("dockercompose", "comp").score,
            "match after a delimiter earns the boundary bonus"
        );
        assert!(
            fuzzy_match("foo-camel-case", "case").score
                > fuzzy_match("foocamelcase", "case").score,
            "two delimiters before the match beat no boundary at all"
        );
    }

    /// The non-ASCII fallback keeps the old port's contract: contiguous
    /// matches score their byte length, non-contiguous ones are rejected.
    /// Score is byte-length here, not character count, to match the
    /// substring semantics.
    #[test]
    fn non_ascii_falls_back_to_a_substring_test() {
        assert_eq!(fuzzy_match("中文命令", "中文"), Match::hit(6)); // 2 chars * 3 UTF-8 bytes
        assert_eq!(fuzzy_match("中文命令", "命令"), Match::hit(6));
        assert!(
            !fuzzy_match("中文命令", "中令").is_match,
            "substring only off-ASCII"
        );
    }

    /// Greedy/optimal-alignment regression vectors transcribed from the old
    /// port's golden table. The exact scores are gone (skim's DP is
    /// different from the hand-rolled one), but the match/no-match
    /// classification is preserved — that is what keeps the
    /// single-keystroke path honest.
    #[test]
    fn greedy_branch_keeps_match_classification() {
        for (text, pattern, should_match) in [
            ("killport", "p", true),
            ("flushdns", "f", true),
            ("sysinfo", "s", true),
            ("ip", "i", true),
            ("killport", "t", true),
            ("ip", "p", true),
            ("killport", "z", false),
            ("taskmanager", "taskmanag", true),
            (
                "kubernetes-network-policy-audit-tool-with-verbose-logging",
                "kube",
                true,
            ),
        ] {
            let hit = fuzzy_match(text, pattern);
            assert_eq!(
                hit.is_match, should_match,
                "text={text:?} pattern={pattern:?}"
            );
        }
    }
}
