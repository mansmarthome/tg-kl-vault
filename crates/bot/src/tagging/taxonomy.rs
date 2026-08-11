//! The fixed category table: the single source of truth for the slug list,
//! the heuristic keywords, and the UI labels, so none of the three can drift.
//!
//! The **order of `TAGS` is a wire format** — the index is encoded into
//! `callback_data` (`bm:tt:<id>:<idx>:…`) that outlives the process in users'
//! chat history. You may **append** and you may **rename a slug**, but you may
//! **never reorder or remove** an entry. The golden test below pins this.

pub struct Category {
    /// English slug. Stored in `bookmark_tags.tag` and shown directly in the UI
    /// (decision #4: tags are always English slugs).
    pub slug: &'static str,
    /// Lowercase substrings scored by the local heuristic tagger.
    pub keywords: &'static [&'static str],
}

/// ~20 categories, `other` last. APPEND ONLY — see module docs.
pub const TAGS: &[Category] = &[
    Category { slug: "tech", keywords: &["tech", "software", "hardware", "gadget", "computer", "cloud", "database", "api", "linux", "devops", "kubernetes", "gpu", "cuda"] },
    Category { slug: "ai", keywords: &["ai", "artificial intelligence", "machine learning", "deep learning", "neural", "llm", "gpt", "chatbot", "transformer", "diffusion"] },
    Category { slug: "programming", keywords: &["programming", "rust", "python", "javascript", "typescript", "golang", "compiler", "async", "framework", "refactor", "code review", "simd"] },
    Category { slug: "science", keywords: &["science", "physics", "chemistry", "biology", "space", "astronomy", "research", "quantum", "climate"] },
    Category { slug: "business", keywords: &["business", "management", "strategy", "marketing", "enterprise", "b2b", "saas"] },
    Category { slug: "finance", keywords: &["finance", "investing", "stock", "market", "crypto", "bitcoin", "economy", "bank", "trading"] },
    Category { slug: "startup", keywords: &["startup", "founder", "venture", "vc", "funding", "seed round", "yc"] },
    Category { slug: "design", keywords: &["design", "ux", "ui", "typography", "figma", "product design", "css"] },
    Category { slug: "security", keywords: &["security", "vulnerability", "exploit", "cve", "malware", "encryption", "privacy", "breach", "phishing"] },
    Category { slug: "health", keywords: &["health", "medicine", "fitness", "nutrition", "mental health", "wellness", "disease"] },
    Category { slug: "gaming", keywords: &["gaming", "game", "playstation", "xbox", "nintendo", "esports", "steam"] },
    Category { slug: "entertainment", keywords: &["movie", "film", "music", "tv", "streaming", "celebrity", "netflix"] },
    Category { slug: "news", keywords: &["news", "breaking", "headline", "report", "coverage"] },
    Category { slug: "politics", keywords: &["politics", "election", "government", "policy", "senate", "congress", "president"] },
    Category { slug: "education", keywords: &["education", "learning", "course", "university", "tutorial", "study", "teaching"] },
    Category { slug: "lifestyle", keywords: &["lifestyle", "productivity", "habits", "minimalism", "self improvement"] },
    Category { slug: "sports", keywords: &["sports", "football", "soccer", "basketball", "baseball", "tennis", "olympics"] },
    Category { slug: "travel", keywords: &["travel", "flight", "hotel", "tourism", "destination", "vacation"] },
    Category { slug: "food", keywords: &["food", "recipe", "cooking", "restaurant", "cuisine", "baking"] },
    Category { slug: "other", keywords: &[] },
];

/// Canonicalizes a raw tag string to a known slug: lowercases, `_`→`-`, applies
/// a few aliases, then matches against `TAGS`. Returns `None` for anything not
/// in the table (so the AI can't invent categories).
pub fn normalize(raw: &str) -> Option<&'static str> {
    let lowered = raw.trim().to_ascii_lowercase().replace('_', "-");
    let canonical = match lowered.as_str() {
        "artificial-intelligence" | "ml" | "machine-learning" | "llm" => "ai",
        "programing" | "coding" | "dev" | "development" => "programming",
        "technology" | "it" => "tech",
        "infosec" | "cybersecurity" | "cyber-security" => "security",
        "econ" | "economics" | "crypto" | "investing" => "finance",
        "game" | "games" | "videogames" | "video-games" => "gaming",
        "movies" | "tv" | "music" => "entertainment",
        "edu" => "education",
        other => other,
    };
    TAGS.iter().find(|c| c.slug == canonical).map(|c| c.slug)
}

pub fn idx_of(slug: &str) -> Option<usize> {
    TAGS.iter().position(|c| c.slug == slug)
}

pub fn slug_of(idx: usize) -> Option<&'static str> {
    TAGS.get(idx).map(|c| c.slug)
}

/// All slugs, in wire order.
pub fn all_slugs() -> impl Iterator<Item = &'static str> {
    TAGS.iter().map(|c| c.slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden pin: the slug order is a wire format (callback_data indices).
    /// Append-only — never reorder or delete. Renames are allowed (update the
    /// expected string here deliberately).
    #[test]
    fn taxonomy_order_is_append_only_golden() {
        let slugs: Vec<&str> = all_slugs().collect();
        assert_eq!(
            slugs,
            vec![
                "tech", "ai", "programming", "science", "business", "finance",
                "startup", "design", "security", "health", "gaming",
                "entertainment", "news", "politics", "education", "lifestyle",
                "sports", "travel", "food", "other",
            ]
        );
    }

    #[test]
    fn other_is_present_and_last() {
        assert_eq!(TAGS.last().map(|c| c.slug), Some("other"));
        assert!(idx_of("other").is_some());
    }

    #[test]
    fn normalize_maps_aliases_and_rejects_unknown() {
        assert_eq!(normalize("AI"), Some("ai"));
        assert_eq!(normalize("machine_learning"), Some("ai"));
        assert_eq!(normalize("Technology"), Some("tech"));
        assert_eq!(normalize("cybersecurity"), Some("security"));
        assert_eq!(normalize("not-a-category"), None);
    }

    #[test]
    fn idx_slug_round_trip() {
        for (i, cat) in TAGS.iter().enumerate() {
            assert_eq!(idx_of(cat.slug), Some(i));
            assert_eq!(slug_of(i), Some(cat.slug));
        }
    }
}
