/// Generate the Go-compatible content hash ID.
///
/// The original bot uses Go's `hash/fnv.New32()`, which is FNV-1: multiply by
/// the prime first, then xor the input byte. This is intentionally *not* FNV-1a
/// (Rust's common `fnv` crate implements FNV-1a and would break dedup parity).
pub fn gen_hash_id(source_link: &str, guid: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in format!("{source_link}||{guid}").bytes() {
        h = h.wrapping_mul(0x0100_0193);
        h ^= u32::from(b);
    }
    format!("{h:08x}")
}

#[cfg(test)]
mod tests {
    use super::gen_hash_id;

    #[test]
    fn fnv1_hash_matches_go_vectors() {
        let vectors = [
            ("https://example.com/feed", "post-1", "e4f61636"),
            ("", "", "557702a5"),
            (
                "https://blog.example.org/rss.xml",
                "https://blog.example.org/2024/01/hello",
                "979c07bb",
            ),
        ];

        for (link, guid, expected) in vectors {
            assert_eq!(gen_hash_id(link, guid), expected, "{link:?} {guid:?}");
        }
    }
}
