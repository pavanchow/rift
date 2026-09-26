//! Integration coverage for Rift's routing DSL: the lexer's string handling, the
//! parser's error surface, and the first-match router's AND semantics. Exercises
//! the edges the inline unit tests leave open.

use rift::rules::{parse, Action};

fn hdr(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
        .collect()
}

fn respond_code(a: Option<&Action>) -> u16 {
    match a {
        Some(Action::Respond(c, _)) => *c,
        other => panic!("expected respond, got {other:?}"),
    }
}

fn respond_body(a: Option<&Action>) -> String {
    match a {
        Some(Action::Respond(_, b)) => b.clone(),
        other => panic!("expected respond, got {other:?}"),
    }
}

fn upstream_url(a: Option<&Action>) -> String {
    match a {
        Some(Action::Upstream(u)) => u.clone(),
        other => panic!("expected upstream, got {other:?}"),
    }
}

// --- string / lexer behavior ---

#[test]
fn escape_sequences_decode() {
    let r = parse(r#"route path "/" -> respond 200 "a\nb\"c\\d""#).unwrap();
    assert_eq!(respond_body(r.route(None, "/", &[])), "a\nb\"c\\d");
}

#[test]
fn unknown_escape_keeps_following_char() {
    // `\t` is not a recognized escape; the lexer keeps the char after the backslash.
    let r = parse(r#"route path "/" -> respond 200 "x\ty""#).unwrap();
    assert_eq!(respond_body(r.route(None, "/", &[])), "xty");
}

#[test]
fn unterminated_string_errors() {
    let e = parse(r#"route path "/oops -> respond 200 "x""#).unwrap_err();
    assert!(e.contains("unterminated string"), "{e}");
    assert!(e.contains("line 1"), "{e}");
}

#[test]
fn dangling_escape_errors() {
    let e = parse("route path \"/x\\").unwrap_err();
    assert!(e.contains("dangling escape"), "{e}");
}

#[test]
fn trailing_comment_is_stripped() {
    let r = parse(r#"route path "/" -> respond 200 "hi" # go away"#).unwrap();
    assert_eq!(respond_body(r.route(None, "/", &[])), "hi");
}

#[test]
fn full_line_comment_and_blanks_produce_no_rules() {
    let r = parse("# only a comment\n\n   \n").unwrap();
    assert_eq!(r.rules.len(), 0);
    assert!(r.route(None, "/anything", &[]).is_none());
}

#[test]
fn hash_inside_string_is_literal_not_comment() {
    let r = parse(r##"route path "/" -> respond 200 "a#b""##).unwrap();
    assert_eq!(respond_body(r.route(None, "/", &[])), "a#b");
}

// --- matcher semantics ---

#[test]
fn host_absent_rule_matches_any_host() {
    let r = parse(r#"route path "/" -> respond 200 "ok""#).unwrap();
    assert!(r.route(Some("whatever.com"), "/", &[]).is_some());
    assert!(r.route(None, "/", &[]).is_some());
}

#[test]
fn host_required_but_request_has_none_fails() {
    let r = parse(r#"route host "api.example.com" path "/" -> respond 200 "ok""#).unwrap();
    assert!(r.route(None, "/", &[]).is_none());
}

#[test]
fn prefix_is_literal_starts_with() {
    let r = parse(r#"route path "/ap" -> respond 200 "ok""#).unwrap();
    assert!(r.route(None, "/api/users", &[]).is_some());
    assert!(r.route(None, "/nope", &[]).is_none());
}

#[test]
fn regex_is_unanchored_is_match() {
    let r = parse(r#"route path ~ "v[0-9]+" -> respond 200 "ok""#).unwrap();
    assert!(r.route(None, "/api/v2/x", &[]).is_some());
    assert!(r.route(None, "/api/vX/x", &[]).is_none());
}

#[test]
fn multiple_headers_all_must_match() {
    let r = parse(r#"route path "/" header "X-A" "1" header "X-B" "2" -> respond 200 "ok""#).unwrap();
    assert!(r.route(None, "/", &hdr(&[("x-a", "1"), ("x-b", "2")])).is_some());
    assert!(r.route(None, "/", &hdr(&[("x-a", "1")])).is_none());
    assert!(r.route(None, "/", &hdr(&[("x-a", "1"), ("x-b", "9")])).is_none());
}

#[test]
fn header_name_case_insensitive_value_case_sensitive() {
    let r = parse(r#"route path "/" header "X-Env" "Prod" -> respond 200 "ok""#).unwrap();
    // request header names arrive lowercased; the rule name is lowercased too.
    assert!(r.route(None, "/", &hdr(&[("x-env", "Prod")])).is_some());
    // value must match exactly, including case.
    assert!(r.route(None, "/", &hdr(&[("x-env", "prod")])).is_none());
}

#[test]
fn all_matcher_kinds_and_together() {
    let src = r#"route host "h.com" path ~ "^/v1/" header "X-Key" "k" -> upstream "http://127.0.0.1:9001""#;
    let r = parse(src).unwrap();
    let good = r.route(Some("h.com"), "/v1/go", &hdr(&[("x-key", "k")]));
    assert_eq!(upstream_url(good), "http://127.0.0.1:9001");
    // each dimension can independently fail the match
    assert!(r.route(Some("other.com"), "/v1/go", &hdr(&[("x-key", "k")])).is_none());
    assert!(r.route(Some("h.com"), "/v2/go", &hdr(&[("x-key", "k")])).is_none());
    assert!(r.route(Some("h.com"), "/v1/go", &hdr(&[("x-key", "x")])).is_none());
}

#[test]
fn first_match_wins_across_match_kinds() {
    let src = "route path \"/x\" -> respond 201 \"prefix\"\nroute path ~ \"^/x\" -> respond 202 \"regex\"";
    let r = parse(src).unwrap();
    assert_eq!(respond_code(r.route(None, "/x", &[])), 201);
}

#[test]
fn no_rule_matches_returns_none() {
    let r = parse(r#"route path "/only" -> respond 200 "ok""#).unwrap();
    assert!(r.route(None, "/somewhere-else", &[]).is_none());
}

// --- action parsing ---

#[test]
fn respond_without_body_is_empty() {
    let r = parse(r#"route path "/" -> respond 204"#).unwrap();
    assert_eq!(respond_code(r.route(None, "/", &[])), 204);
    assert_eq!(respond_body(r.route(None, "/", &[])), "");
}

#[test]
fn respond_max_u16_code_parses() {
    let r = parse(r#"route path "/" -> respond 65535"#).unwrap();
    assert_eq!(respond_code(r.route(None, "/", &[])), 65535);
}

#[test]
fn respond_code_out_of_u16_range_errors() {
    let e = parse(r#"route path "/" -> respond 70000 "x""#).unwrap_err();
    assert!(e.contains("respond needs a status code"), "{e}");
}

#[test]
fn respond_non_numeric_code_errors() {
    let e = parse(r#"route path "/" -> respond okay "x""#).unwrap_err();
    assert!(e.contains("respond needs a status code"), "{e}");
}

// --- parser error surface ---

#[test]
fn missing_route_keyword_errors() {
    let e = parse(r#"host "h.com" -> respond 200 "x""#).unwrap_err();
    assert!(e.contains("expected 'route'"), "{e}");
}

#[test]
fn unknown_matcher_keyword_errors() {
    let e = parse(r#"route method "GET" -> respond 200 "x""#).unwrap_err();
    assert!(e.contains("expected host/path/header or '->'"), "{e}");
}

#[test]
fn missing_arrow_and_action_errors() {
    let e = parse(r#"route path "/x""#).unwrap_err();
    assert!(e.contains("missing '->' and action"), "{e}");
}

#[test]
fn missing_action_after_arrow_errors() {
    let e = parse(r#"route path "/x" ->"#).unwrap_err();
    assert!(e.contains("expected 'upstream' or 'respond'"), "{e}");
}

#[test]
fn matcher_expecting_string_gets_none_errors() {
    let e = parse(r#"route host -> respond 200 "x""#).unwrap_err();
    assert!(e.contains("expected a quoted string"), "{e}");
}

#[test]
fn bad_regex_errors_naming_the_line() {
    let e = parse("route path \"/\" -> respond 200 \"a\"\nroute path ~ \"(\" -> respond 200 \"b\"").unwrap_err();
    assert!(e.contains("bad regex"), "{e}");
    assert!(e.contains("line 2"), "{e}");
}

#[test]
fn error_line_number_points_at_the_offending_line() {
    let src = "route path \"/ok\" -> respond 200 \"a\"\n\nnonsense here";
    let e = parse(src).unwrap_err();
    assert!(e.contains("line 3"), "{e}");
}
