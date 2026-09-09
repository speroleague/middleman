#![allow(clippy::unwrap_used)]
use middleman_packet::guides::{self, BEGIN, END};

#[test]
fn merging_preserves_surrounding_bytes_and_newline_style() {
    for newline in ["\n", "\r\n"] {
        let old =
            format!("用户 instructions{newline}{BEGIN}{newline}old{newline}{END}{newline}tail");
        let expected = format!(
            "用户 instructions{newline}{BEGIN}{newline}new{newline}line{newline}{END}{newline}tail"
        );
        let merged = guides::merge(&old, "new\nline\n").unwrap();
        assert_eq!(merged, expected);
        assert_eq!(guides::merge(&merged, "new\nline\n").unwrap(), merged);
    }
    let merged = guides::merge("existing without newline", "body").unwrap();
    assert!(merged.starts_with("existing without newline\n"));
    assert_eq!(guides::merge(&merged, "body").unwrap(), merged);
}

#[test]
fn malformed_and_duplicate_markers_are_rejected() {
    for text in [
        BEGIN.to_string(),
        END.to_string(),
        format!("{END}\n{BEGIN}"),
        format!("{BEGIN}\n{BEGIN}\n{END}"),
        format!("prefix {BEGIN}\n{END}"),
        format!("{BEGIN}suffix\n{END}"),
    ] {
        assert_eq!(guides::merge(&text, "body"), Err(guides::Error::Markers));
    }
    assert_eq!(guides::merge("", BEGIN), Err(guides::Error::Markers));
    assert_eq!(
        guides::merge(&"a".repeat(guides::MAX_BYTES + 1), "body"),
        Err(guides::Error::TooLarge)
    );
}

#[test]
fn generated_integration_uses_available_commands_and_empty_map_is_explicit() {
    let body = guides::agents_md();
    assert!(body.contains("middleman prepare"));
    assert!(body.contains("middleman index --changed"));
    assert!(body.contains("middleman propose --task-id"));
    assert!(body.contains("middleman review"));
    let map =
        guides::agent_context("<script>\n# forged", &std::collections::BTreeMap::new()).unwrap();
    assert!(map.contains("No entries observed."));
    assert!(!map.contains("<script>"));
    assert!(!map.contains("\n# forged"));
}
