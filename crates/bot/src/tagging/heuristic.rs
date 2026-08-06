//! Local keyword-scoring tagger. The terminal fallback: it never fails and
//! never returns empty (always at least `["other"]`). That property is exactly
//! why it can sit at the end of the retry ladder and why a `tag_state = 2`
//! ("heuristic failed") state need not exist.

use super::taxonomy::TAGS;
use super::{TagInput, Tagger};

pub struct HeuristicTagger {
    max_tags: usize,
}

impl HeuristicTagger {
    pub fn new(max_tags: usize) -> Self {
        Self { max_tags: max_tags.max(1) }
    }

    pub fn classify(&self, input: &TagInput<'_>) -> Vec<String> {
        let host = url::Url::parse(input.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let haystack = format!("{} {} {}", input.title, input.excerpt, host).to_ascii_lowercase();

        let mut scored: Vec<(usize, &'static str)> = TAGS
            .iter()
            .filter_map(|cat| {
                let score = cat
                    .keywords
                    .iter()
                    .filter(|kw| haystack.contains(*kw))
                    .count();
                (score > 0).then_some((score, cat.slug))
            })
            .collect();

        // Highest score first; ties keep taxonomy (wire) order via stable sort.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        let tags: Vec<String> = scored
            .into_iter()
            .take(self.max_tags)
            .map(|(_, slug)| slug.to_owned())
            .collect();

        if tags.is_empty() {
            vec!["other".to_owned()]
        } else {
            tags
        }
    }
}

impl Tagger for HeuristicTagger {
    async fn suggest(&self, input: &TagInput<'_>) -> anyhow::Result<Vec<String>> {
        Ok(self.classify(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_empty_falls_back_to_other() {
        let tagger = HeuristicTagger::new(3);
        let tags = tagger.classify(&TagInput {
            title: "zzz qqq",
            url: "https://nomatch.test/x",
            excerpt: "",
        });
        assert_eq!(tags, vec!["other"]);
    }

    #[test]
    fn matches_keywords_and_caps_at_max_tags() {
        let tagger = HeuristicTagger::new(2);
        let tags = tagger.classify(&TagInput {
            title: "A Rust async framework for machine learning security",
            url: "https://blog.test/post",
            excerpt: "programming and ai and security",
        });
        assert!(tags.len() <= 2);
        assert!(tags.iter().all(|t| super::super::taxonomy::normalize(t).is_some()));
    }
}
