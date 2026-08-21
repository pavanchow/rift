//! Rift's routing DSL. Rules are compiled once at startup into matchers, not
//! re-read from YAML on every reload. One rule per line, first match wins.
//!
//!   route host "api.example.com" path ~ "^/v1/" -> upstream "http://127.0.0.1:9001"
//!   route path "/health"                        -> respond 200 "ok"
//!   route path "/static/"  header "X-Env" "prod" -> upstream "http://10.0.0.5:80"
//!
//! Matchers (all AND together): host (exact, case-insensitive), path (prefix, or
//! `~` regex), header (exact). Actions: upstream <url>, respond <code> [body].

use regex::Regex;

#[derive(Debug)]
pub enum PathMatch {
    Prefix(String),
    Regex(Regex),
}

#[derive(Debug, Clone)]
pub enum Action {
    Upstream(String),
    Respond(u16, String),
}

#[derive(Debug)]
pub struct Rule {
    pub host: Option<String>,
    pub path: Option<PathMatch>,
    pub headers: Vec<(String, String)>,
    pub action: Action,
}

impl Rule {
    /// Does this rule match the request? `headers` are (lowercased-name, value).
    pub fn matches(&self, host: Option<&str>, path: &str, headers: &[(String, String)]) -> bool {
        if let Some(h) = &self.host {
            match host {
                Some(rh) if rh.eq_ignore_ascii_case(h) => {}
                _ => return false,
            }
        }
        if let Some(p) = &self.path {
            let ok = match p {
                PathMatch::Prefix(pre) => path.starts_with(pre.as_str()),
                PathMatch::Regex(re) => re.is_match(path),
            };
            if !ok {
                return false;
            }
        }
        for (k, v) in &self.headers {
            let kl = k.to_ascii_lowercase();
            if !headers.iter().any(|(hk, hv)| *hk == kl && hv == v) {
                return false;
            }
        }
        true
    }
}

/// The compiled routing table. First matching rule wins.
#[derive(Debug)]
pub struct Router {
    pub rules: Vec<Rule>,
}

impl Router {
    pub fn route(&self, host: Option<&str>, path: &str, headers: &[(String, String)]) -> Option<&Action> {
        self.rules
            .iter()
            .find(|r| r.matches(host, path, headers))
            .map(|r| &r.action)
    }
}

// --- lexer ---

#[derive(Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Arrow,
    Tilde,
}

fn lex(line: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let cs: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '#' {
            break; // trailing comment
        } else if c == '"' {
            i += 1;
            let mut s = String::new();
            loop {
                match cs.get(i) {
                    None => return Err("unterminated string".into()),
                    Some('"') => {
                        i += 1;
                        break;
                    }
                    Some('\\') => {
                        match cs.get(i + 1) {
                            Some('"') => s.push('"'),
                            Some('\\') => s.push('\\'),
                            Some('n') => s.push('\n'),
                            Some(other) => s.push(*other),
                            None => return Err("dangling escape".into()),
                        }
                        i += 2;
                    }
                    Some(ch) => {
                        s.push(*ch);
                        i += 1;
                    }
                }
            }
            out.push(Tok::Str(s));
        } else if c == '~' {
            out.push(Tok::Tilde);
            i += 1;
        } else if c == '-' && cs.get(i + 1) == Some(&'>') {
            out.push(Tok::Arrow);
            i += 2;
        } else {
            let start = i;
            while i < cs.len() && !cs[i].is_whitespace() && cs[i] != '"' {
                i += 1;
            }
            out.push(Tok::Ident(cs[start..i].iter().collect()));
        }
    }
    Ok(out)
}

/// Parse the DSL into a router. Returns an error naming the offending line.
pub fn parse(src: &str) -> Result<Router, String> {
    let mut rules = Vec::new();
    for (n, raw) in src.lines().enumerate() {
        let toks = lex(raw).map_err(|e| format!("line {}: {e}", n + 1))?;
        if toks.is_empty() {
            continue;
        }
        rules.push(parse_rule(&toks).map_err(|e| format!("line {}: {e}", n + 1))?);
    }
    Ok(Router { rules })
}

fn parse_rule(toks: &[Tok]) -> Result<Rule, String> {
    let mut i = 0;
    match toks.first() {
        Some(Tok::Ident(k)) if k == "route" => i += 1,
        _ => return Err("expected 'route'".into()),
    }
    let mut host = None;
    let mut path = None;
    let mut headers = Vec::new();

    loop {
        match toks.get(i) {
            Some(Tok::Arrow) => {
                i += 1;
                break;
            }
            Some(Tok::Ident(k)) if k == "host" => {
                host = Some(expect_str(toks, i + 1)?);
                i += 2;
            }
            Some(Tok::Ident(k)) if k == "path" => {
                if toks.get(i + 1) == Some(&Tok::Tilde) {
                    let pat = expect_str(toks, i + 2)?;
                    let re = Regex::new(&pat).map_err(|e| format!("bad regex: {e}"))?;
                    path = Some(PathMatch::Regex(re));
                    i += 3;
                } else {
                    path = Some(PathMatch::Prefix(expect_str(toks, i + 1)?));
                    i += 2;
                }
            }
            Some(Tok::Ident(k)) if k == "header" => {
                let name = expect_str(toks, i + 1)?;
                let val = expect_str(toks, i + 2)?;
                headers.push((name, val));
                i += 3;
            }
            Some(_) => return Err("expected host/path/header or '->'".into()),
            None => return Err("missing '->' and action".into()),
        }
    }

    let action = match toks.get(i) {
        Some(Tok::Ident(k)) if k == "upstream" => Action::Upstream(expect_str(toks, i + 1)?),
        Some(Tok::Ident(k)) if k == "respond" => {
            let code: u16 = match toks.get(i + 1) {
                Some(Tok::Ident(c)) => c.parse().map_err(|_| "respond needs a status code")?,
                _ => return Err("respond needs a status code".into()),
            };
            let body = match toks.get(i + 2) {
                Some(Tok::Str(s)) => s.clone(),
                _ => String::new(),
            };
            Action::Respond(code, body)
        }
        _ => return Err("expected 'upstream' or 'respond' after '->'".into()),
    };

    Ok(Rule { host, path, headers, action })
}

fn expect_str(toks: &[Tok], i: usize) -> Result<String, String> {
    match toks.get(i) {
        Some(Tok::Str(s)) => Ok(s.clone()),
        _ => Err("expected a quoted string".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.to_string())).collect()
    }

    #[test]
    fn prefix_and_upstream() {
        let r = parse(r#"route path "/api/" -> upstream "http://127.0.0.1:9001""#).unwrap();
        match r.route(None, "/api/users", &[]) {
            Some(Action::Upstream(u)) => assert_eq!(u, "http://127.0.0.1:9001"),
            other => panic!("{other:?}"),
        }
        assert!(r.route(None, "/other", &[]).is_none());
    }

    #[test]
    fn host_and_regex() {
        let r = parse(r#"route host "api.example.com" path ~ "^/v[0-9]+/" -> respond 200 "ok""#).unwrap();
        assert!(r.route(Some("api.example.com"), "/v2/x", &[]).is_some());
        assert!(r.route(Some("API.EXAMPLE.COM"), "/v2/x", &[]).is_some()); // case-insensitive host
        assert!(r.route(Some("other.com"), "/v2/x", &[]).is_none());
        assert!(r.route(Some("api.example.com"), "/nope", &[]).is_none());
    }

    #[test]
    fn header_match() {
        let r = parse(r#"route path "/" header "X-Env" "prod" -> respond 204 """#).unwrap();
        assert!(r.route(None, "/", &hdr(&[("x-env", "prod")])).is_some());
        assert!(r.route(None, "/", &hdr(&[("x-env", "dev")])).is_none());
        assert!(r.route(None, "/", &[]).is_none());
    }

    #[test]
    fn first_match_wins() {
        let src = "route path \"/a\" -> respond 201 \"first\"\nroute path \"/a\" -> respond 202 \"second\"";
        let r = parse(src).unwrap();
        match r.route(None, "/a", &[]) {
            Some(Action::Respond(c, _)) => assert_eq!(*c, 201),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn comments_and_blanks_ignored() {
        let src = "# a comment\n\nroute path \"/\" -> respond 200 \"hi\"  # trailing\n";
        let r = parse(src).unwrap();
        assert_eq!(r.rules.len(), 1);
    }

    #[test]
    fn parse_errors_name_the_line() {
        assert!(parse("route path -> respond 200").unwrap_err().contains("line 1"));
        assert!(parse("nonsense").unwrap_err().contains("line 1"));
        assert!(parse("route path ~ \"(\" -> respond 200 \"x\"").unwrap_err().contains("bad regex"));
    }
}
