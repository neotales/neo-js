//! Tests for the third-party notice text embedded in the library.
//!
//! The accessor must return the complete notice file without constructing a
//! runtime or requiring a permission.

#[test]
fn notices_are_embedded_and_complete() {
    let text = neo_js::third_party_notices();

    // The accessor is the source file verbatim, not a hand-maintained copy.
    assert_eq!(text, include_str!("../src/js/path.LICENSE.md"));

    for expected in [
        "Third-party notices for `neo:path`",
        "c7e88644f45a78f65bf639c9d365bb9eff21bcfe",
        "Copyright 2022-2026 Neotales",
        "Copyright 2018-2022 the Deno authors",
        "Copyright (c) 2013 James Halliday",
        "Permission is hereby granted",
        "THE SOFTWARE IS PROVIDED \"AS IS\"",
    ] {
        assert!(
            text.contains(expected),
            "notice text is missing {expected:?}"
        );
    }

    // The file is more than a heading and a pointer; it carries full text.
    assert!(text.len() > 3_000, "notice text is unexpectedly short");
}
