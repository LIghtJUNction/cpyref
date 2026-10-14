use cpyref::select::{select, Spec};

fn copied(data: &[u8], spec: Spec) -> Vec<u8> {
    let span = select(data, &spec).unwrap();
    data[span.range()].to_vec()
}

fn text(value: &str) -> Spec {
    Spec { text: Some(value.into()), ..Spec::default() }
}

fn pattern(value: &str) -> Spec {
    Spec { regex: Some(value.into()), ..Spec::default() }
}

#[test]
fn whole_input_preserves_arbitrary_bytes() {
    let data = b"\xff\0hello\r\n";
    assert_eq!(copied(data, Spec::default()), data);
}

#[test]
fn inclusive_lines_preserve_crlf() {
    let data = b"first\r\nsecond\r\nthird";
    assert_eq!(copied(data, Spec { lines: Some("2:3".into()), ..Spec::default() }), b"second\r\nthird");
}

#[test]
fn single_line_retains_terminator() {
    assert_eq!(copied(b"a\nb\n", Spec { lines: Some("2".into()), ..Spec::default() }), b"b\n");
}

#[test]
fn final_newline_does_not_create_a_phantom_line() {
    let error = select(b"a\nb\n", &Spec { lines: Some("3".into()), ..Spec::default() }).unwrap_err();
    assert_eq!(error.code, "NO_MATCH");
    assert_eq!(error.details["line_count"], 2);
}

#[test]
fn invalid_line_ranges_are_rejected() {
    for value in ["0", "2:1", ":3", "1:", "x", "1:2:3", "-1"] {
        assert_eq!(select(b"a\nb", &Spec { lines: Some(value.into()), ..Spec::default() }).unwrap_err().code, "INVALID_ARGUMENT");
    }
}

#[test]
fn line_copy_does_not_require_utf8() {
    assert_eq!(copied(b"a\n\xff\r\n", Spec { lines: Some("2".into()), ..Spec::default() }), b"\xff\r\n");
}

#[test]
fn exact_unicode_bytes_are_unchanged() {
    assert_eq!(copied("前缀：你好🦀。".as_bytes(), text("你好🦀")), "你好🦀".as_bytes());
}

#[test]
fn overlapping_literal_matches_are_ambiguous() {
    let error = select(b"banana", &text("ana")).unwrap_err();
    assert_eq!(error.code, "AMBIGUOUS");
    assert_eq!(error.exit, 3);
    assert_eq!(error.details["match_count"], 2);
}

#[test]
fn ambiguity_reports_positions_without_source_text() {
    let error = select(b"SECRET\nSECRET", &text("SECRET")).unwrap_err();
    assert_eq!(error.details["candidates"][1]["span"]["line_start"], 2);
    assert!(!serde_json::to_string(&error).unwrap().contains("SECRET"));
}

#[test]
fn no_match_is_distinct_from_ambiguity() {
    let error = select(b"abc", &text("xyz")).unwrap_err();
    assert_eq!(error.code, "NO_MATCH");
    assert_eq!(error.exit, 2);
}

#[test]
fn occurrence_is_explicit_and_one_based() {
    let span = select(b"banana", &Spec { occurrence: Some(2), ..text("ana") }).unwrap();
    assert_eq!(span.byte_start, 3);
    assert_eq!(span.byte_end, 6);
}

#[test]
fn occurrence_out_of_range_does_not_fall_back() {
    assert_eq!(select(b"one", &Spec { occurrence: Some(2), ..text("one") }).unwrap_err().code, "NO_MATCH");
    assert_eq!(select(b"one", &Spec { occurrence: Some(0), ..text("one") }).unwrap_err().code, "INVALID_ARGUMENT");
}

#[test]
fn regex_capture_by_name_and_number() {
    for group in ["url", "1"] {
        let spec = Spec { group: Some(group.into()), ..pattern(r"href=(?P<url>\S+)") };
        assert_eq!(copied(b"href=https://example.test/a?x=1&y=2", spec), b"https://example.test/a?x=1&y=2");
    }
}

#[test]
fn unknown_capture_is_an_error() {
    assert_eq!(select(b"abc", &Spec { group: Some("missing".into()), ..pattern("(abc)") }).unwrap_err().code, "INVALID_GROUP");
}

#[test]
fn optional_absent_capture_is_not_an_empty_success() {
    assert_eq!(select(b"b", &Spec { group: Some("1".into()), ..pattern("(a)?b") }).unwrap_err().code, "NO_MATCH");
}

#[test]
fn invalid_regex_is_reported() {
    assert_eq!(select(b"abc", &pattern("[")).unwrap_err().code, "INVALID_REGEX");
}

#[test]
fn zero_width_regex_is_rejected() {
    assert_eq!(select(b"abc", &pattern("^")).unwrap_err().code, "EMPTY_SELECTION");
}

#[test]
fn dotall_and_multiline_are_explicit() {
    assert_eq!(copied(b"start\nx\nend", Spec { dotall: true, ..pattern("start.*end") }), b"start\nx\nend");
    assert_eq!(copied(b"start\nvalue=3\nend", Spec { multiline: true, ..pattern("^value=.*$") }), b"value=3");
}

#[test]
fn ignore_case_preserves_source_case() {
    assert_eq!(copied(b"Hello", Spec { ignore_case: true, ..text("hello") }), b"Hello");
}

#[test]
fn multiline_literal_does_not_normalize_newlines() {
    assert_eq!(copied(b"a\r\nb\r\nc", text("a\r\nb")), b"a\r\nb");
    assert_eq!(select(b"a\r\nb", &text("a\nb")).unwrap_err().code, "NO_MATCH");
}

#[test]
fn markers_include_or_exclude_exact_delimiters() {
    let spec = Spec { from: Some("<x>".into()), to: Some("</x>".into()), ..Spec::default() };
    assert_eq!(copied(b"prefix<x>\r\nbody\r\n</x>tail", spec.clone()), b"<x>\r\nbody\r\n</x>");
    assert_eq!(copied(b"<x>body</x>", Spec { exclude_markers: true, ..spec }), b"body");
}

#[test]
fn repeated_marker_blocks_need_disambiguation() {
    let spec = Spec { from: Some("[".into()), to: Some("]".into()), ..Spec::default() };
    assert_eq!(select(b"[a][b]", &spec).unwrap_err().code, "AMBIGUOUS");
    assert_eq!(copied(b"[a][b]", Spec { occurrence: Some(2), ..spec }), b"[b]");
}

#[test]
fn regex_markers_work_across_lines() {
    let spec = Spec { from: Some("^BEGIN [0-9]+$".into()), to: Some("^END$".into()), markers_regex: true, multiline: true, ..Spec::default() };
    assert_eq!(copied(b"BEGIN 42\nbody\nEND\ntail", spec), b"BEGIN 42\nbody\nEND");
}

#[test]
fn unmatched_markers_report_the_missing_boundary() {
    let spec = Spec { from: Some("BEGIN".into()), to: Some("END".into()), ..Spec::default() };
    assert_eq!(select(b"nothing", &spec).unwrap_err().details["reason"], "START_MARKER_NOT_FOUND");
    assert_eq!(select(b"BEGIN only", &spec).unwrap_err().details["reason"], "END_MARKER_NOT_FOUND");
}

#[test]
fn within_limits_search_and_keeps_absolute_positions() {
    let spec = Spec { within: Some("2:3".into()), ..text("same") };
    let span = select(b"same\nother\nsame\n", &spec).unwrap();
    assert_eq!(span.line_start, 3);
    assert_eq!(span.byte_start, 11);
}

#[test]
fn context_filters_are_applied_before_occurrence() {
    let spec = Spec { before: Some("target:".into()), context_lines: 0, occurrence: Some(1), ..text("value") };
    let span = select(b"other:value\ntarget:value", &spec).unwrap();
    assert_eq!(span.line_start, 2);
}

#[test]
fn following_context_can_select_one_match() {
    let spec = Spec { after: Some("accepted".into()), context_lines: 0, ..text("value") };
    assert_eq!(select(b"value rejected\nvalue accepted", &spec).unwrap().line_start, 2);
}

#[test]
fn context_does_not_escape_within_scope() {
    let spec = Spec { before: Some("secret".into()), within: Some("2".into()), context_lines: 10, ..text("value") };
    assert_eq!(select(b"secret\nvalue", &spec).unwrap_err().code, "NO_MATCH");
}

#[test]
fn conflicting_and_empty_selectors_fail() {
    assert_eq!(select(b"a", &Spec { lines: Some("1".into()), ..text("a") }).unwrap_err().code, "INVALID_ARGUMENT");
    assert_eq!(select(b"a", &text("")).unwrap_err().code, "INVALID_ARGUMENT");
    assert_eq!(select(b"a", &Spec { before: Some("".into()), ..text("a") }).unwrap_err().code, "INVALID_ARGUMENT");
    assert_eq!(select(b"a", &Spec { from: Some("a".into()), ..Spec::default() }).unwrap_err().code, "INVALID_ARGUMENT");
}

#[test]
fn text_matching_rejects_invalid_utf8() {
    assert_eq!(select(b"\xffa", &text("a")).unwrap_err().code, "INVALID_UTF8");
}

#[test]
fn empty_input_has_no_success_reference() {
    assert_eq!(select(b"", &Spec::default()).unwrap_err().code, "EMPTY_SELECTION");
}

#[test]
fn excessive_matches_fail_instead_of_hiding_candidates() {
    let input = vec![b'a'; 10_001];
    assert_eq!(select(&input, &text("a")).unwrap_err().code, "MATCH_LIMIT");
}
