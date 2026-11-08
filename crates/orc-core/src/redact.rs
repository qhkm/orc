//! Remove secrets from text before orc sends it to another provider.
//!
//! orc passes one agent's output, check logs and diffs to a different agent,
//! often from a different vendor (hand-off brief, reviewer prompt, cross-tool
//! resume, REPL provider switch). A token that leaked into a test log must not
//! travel with it. Each secret is replaced by `[REDACTED:<kind>]`, so the next
//! agent knows something was there without seeing it.
//!
//! Two layers:
//! - patterns: private keys, well-known token prefixes, JWTs, bearer headers,
//!   URL passwords, `SECRET_NAME=value` assignments, quoted `"token": "..."` keys;
//! - known values: the exact values of secret-named variables in orc's own
//!   environment and in the project's `.env*` / `.dev.vars` files, which catches
//!   passwords and keys with no recognisable shape.
//!
//! Redact before truncating: a cut can split a secret so no pattern matches.

use regex::{Captures, Regex};
use std::path::Path;
use std::sync::OnceLock;

/// The marker every redaction starts with.
pub const MARKER: &str = "[REDACTED:";

/// Shortest value treated as a secret; shorter ones are too likely to be ordinary words.
const MIN_SECRET_LEN: usize = 8;

/// Text with its secrets replaced, and how many were replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    pub text: String,
    pub count: usize,
}

/// One sentence to add to a prompt whose content went through redaction.
pub fn note() -> &'static str {
    "Values shown as `[REDACTED:...]` are secrets orc removed before sending this to you. \
     They are not bugs: do not try to restore them, and leave the real values in files and the environment alone."
}

/// Whether `text` contains at least one redaction marker.
pub fn has_redactions(text: &str) -> bool {
    text.contains(MARKER)
}

/// Redacts by pattern, plus any exact values it was given.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    /// Exact secret values, longest first so a value that contains another wins.
    known: Vec<String>,
}

impl Redactor {
    /// Patterns only.
    pub fn new() -> Self {
        Self::default()
    }

    /// Patterns, plus secret-named values from orc's environment and from the
    /// `.env*` and `.dev.vars` files at the root of `dir`.
    pub fn for_dir(dir: &Path) -> Self {
        let mut values: Vec<String> = std::env::vars()
            .filter(|(name, value)| is_secret_name(name) && value.len() >= MIN_SECRET_LEN)
            .map(|(_, value)| value)
            .collect();
        values.extend(env_file_secrets(dir));
        Self::default().with_known_values(values)
    }

    /// Add exact values to remove wherever they appear.
    pub fn with_known_values(mut self, values: impl IntoIterator<Item = String>) -> Self {
        self.known.extend(values.into_iter().map(|v| v.trim().to_string()).filter(|v| v.len() >= MIN_SECRET_LEN));
        self.known.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        self.known.dedup();
        self
    }

    pub fn redact(&self, text: &str) -> Redacted {
        let mut count = 0;
        let mut out = text.to_string();
        for value in &self.known {
            let n = out.matches(value.as_str()).count();
            if n > 0 {
                out = out.replace(value.as_str(), "[REDACTED:value]");
                count += n;
            }
        }
        let (out, n) = redact_patterns(&out);
        Redacted { text: out, count: count + n }
    }

    /// Shorthand for `redact(text).text`.
    pub fn clean(&self, text: &str) -> String {
        self.redact(text).text
    }
}

/// Pattern-only redaction, for callers with no project directory at hand.
pub fn redact(text: &str) -> Redacted {
    Redactor::new().redact(text)
}

/// `true` for variable names that conventionally hold a secret
/// (`OPENAI_API_KEY`, `DB_PASSWORD`, `GITHUB_TOKEN`, `STRIPE_SECRET`, `SIGNING_KEY`).
pub fn is_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase().replace('-', "_");
    const PARTS: [&str; 8] =
        ["SECRET", "TOKEN", "PASSWORD", "PASSWD", "API_KEY", "APIKEY", "PRIVATE_KEY", "CREDENTIAL"];
    PARTS.iter().any(|p| upper.contains(p)) || upper.ends_with("_KEY") || upper == "KEY"
}

/// Values that are obviously examples, not real secrets.
fn is_placeholder(value: &str) -> bool {
    let v = value.to_ascii_lowercase();
    ["xxxx", "your", "example", "changeme", "placeholder", "redacted", "dummy", "<", "${", "****"]
        .iter()
        .any(|p| v.contains(p))
        || v.chars().all(|c| c == v.chars().next().unwrap_or(' '))
}

/// Long values with both letters and digits look like keys or passwords even
/// under an innocent name (e.g. `DATABASE_URL`, `SUPABASE_ANON`).
fn looks_random(value: &str) -> bool {
    value.len() >= 16
        && !value.contains(char::is_whitespace)
        && value.chars().any(|c| c.is_ascii_digit())
        && value.chars().any(|c| c.is_ascii_alphabetic())
}

/// Secret values from `KEY=VALUE` files at the root of `dir`.
fn env_file_secrets(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut values = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_env_file = name == ".env" || name.starts_with(".env.") || name == ".dev.vars";
        // Templates hold placeholders, not secrets.
        if !is_env_file || name.ends_with(".example") || name.ends_with(".sample") || name.ends_with(".template") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else { continue };
        values.extend(parse_env_secrets(&content));
    }
    values
}

/// The secret-looking values in a dotenv file.
fn parse_env_secrets(content: &str) -> Vec<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            if line.starts_with('#') {
                return None;
            }
            let (key, value) = line.split_once('=')?;
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
            let wanted = (is_secret_name(key.trim()) || looks_random(&value))
                && value.len() >= MIN_SECRET_LEN
                && !is_placeholder(&value);
            wanted.then_some(value)
        })
        .collect()
}

struct Patterns {
    pem: Regex,
    pem_open: Regex,
    tokens: Vec<(&'static str, Regex)>,
    jwt: Regex,
    bearer: Regex,
    url_password: Regex,
    query: Regex,
    assignment: Regex,
    quoted_key: Regex,
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let re = |p: &str| Regex::new(p).expect("valid redaction regex");
        Patterns {
            pem: re(r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----"),
            // A key block whose END line was cut off or never printed.
            pem_open: re(r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*"),
            tokens: vec![
                ("anthropic-key", re(r"\bsk-ant-[A-Za-z0-9_-]{20,}")),
                ("openai-key", re(r"\bsk-(?:proj-|svcacct-|admin-)?[A-Za-z0-9_-]{20,}")),
                ("github-token", re(r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}")),
                ("github-token", re(r"\bgithub_pat_[A-Za-z0-9_]{22,}")),
                ("gitlab-token", re(r"\bglpat-[A-Za-z0-9_-]{20,}")),
                ("slack-token", re(r"\bxox[abprs]-[A-Za-z0-9-]{10,}")),
                ("aws-access-key", re(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b")),
                ("google-api-key", re(r"\bAIza[0-9A-Za-z_-]{35}")),
                ("stripe-key", re(r"\b(?:sk|rk|pk)_(?:live|test)_[0-9A-Za-z]{16,}")),
                ("stripe-webhook-secret", re(r"\bwhsec_[0-9A-Za-z]{16,}")),
                ("npm-token", re(r"\bnpm_[A-Za-z0-9]{36}")),
                ("huggingface-token", re(r"\bhf_[A-Za-z0-9]{30,}")),
                ("groq-key", re(r"\bgsk_[A-Za-z0-9]{40,}")),
            ],
            jwt: re(r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"),
            bearer: re(r"(?i)\b(bearer|token)(\s+)([A-Za-z0-9._~+/=-]{16,})"),
            url_password: re(r"\b([a-zA-Z][a-zA-Z0-9+.-]*://[^\s:/@]+):([^\s@/]+)@"),
            query: re(r"(?i)([?&](?:api[_-]?key|access[_-]?token|token|secret|password|sig|signature|key)=)([^&\s#'\x22]{8,})"),
            assignment: re(r"\b([A-Z][A-Z0-9_]*)(\s*[:=]\s*)([\x22']?)([^\s\x22'`]{8,})"),
            quoted_key: re(r"(?i)([\x22'])([a-z0-9_.-]*(?:secret|token|password|passwd|api[_-]?key|access[_-]?key|private[_-]?key)[a-z0-9_.-]*)([\x22']\s*[:=]\s*)([\x22'])([^\x22'\s]{8,})([\x22'])"),
        }
    })
}

fn redact_patterns(text: &str) -> (String, usize) {
    let p = patterns();
    let mut count = 0usize;
    let mut out = text.to_string();

    let mut swap = |re: &Regex, out: &mut String, f: &dyn Fn(&Captures) -> Option<String>| {
        let mut n = 0;
        let replaced = re
            .replace_all(out, |c: &Captures| match f(c) {
                Some(r) => {
                    n += 1;
                    r
                }
                None => c[0].to_string(),
            })
            .into_owned();
        *out = replaced;
        count += n;
    };

    swap(&p.pem, &mut out, &|_| Some("[REDACTED:private-key]".into()));
    swap(&p.pem_open, &mut out, &|_| Some("[REDACTED:private-key]".into()));
    for (kind, re) in &p.tokens {
        swap(re, &mut out, &|_| Some(format!("[REDACTED:{kind}]")));
    }
    swap(&p.jwt, &mut out, &|_| Some("[REDACTED:jwt]".into()));
    swap(&p.bearer, &mut out, &|c| {
        let value = &c[3];
        // Prose like "token expires" stays; only key-shaped values go.
        (looks_random(value) && !value.contains(MARKER)).then(|| format!("{}{}[REDACTED:bearer]", &c[1], &c[2]))
    });
    swap(&p.url_password, &mut out, &|c| {
        (!c[2].starts_with(MARKER)).then(|| format!("{}:[REDACTED:password]@", &c[1]))
    });
    swap(&p.query, &mut out, &|c| {
        (!is_placeholder(&c[2]) && !c[2].starts_with(MARKER)).then(|| format!("{}[REDACTED:value]", &c[1]))
    });
    swap(&p.assignment, &mut out, &|c| {
        let (name, value) = (&c[1], &c[4]);
        let secret = is_secret_name(name) && !is_placeholder(value) && !value.starts_with(MARKER)
            // `TOKEN=$(cat file)`, `KEY=os.environ[...]`: a reference, not a value.
            && !value.contains('(') && !value.contains('[')
            // `TOKEN_TTL=86400000` is a number, not a secret.
            && !value.chars().all(|c| c.is_ascii_digit());
        secret.then(|| format!("{}{}{}[REDACTED:{}]", name, &c[2], &c[3], name))
    });
    swap(&p.quoted_key, &mut out, &|c| {
        let value = &c[5];
        (!is_placeholder(value) && !value.starts_with(MARKER))
            .then(|| format!("{}{}{}{}[REDACTED:value]{}", &c[1], &c[2], &c[3], &c[4], &c[6]))
    });
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(text: &str) -> String {
        redact(text).text
    }

    #[test]
    fn removes_well_known_tokens() {
        let cases = [
            ("key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123", "anthropic-key"),
            ("OPENAI sk-proj-abcdefghijklmnopqrstuvwx1234", "openai-key"),
            ("using sk-xtwb5apIbuB5AIe7jJGEbA for vrs", "openai-key"),
            ("ghp_abcdefghijklmnopqrstuvwxyz0123456789", "github-token"),
            ("github_pat_11ABCDEFG0123456789_abcdefghijklmnop", "github-token"),
            ("xoxb-1234567890-abcdefghij", "slack-token"),
            ("AKIAIOSFODNN7EXAMPLE was used", "aws-access-key"),
            ("AIzaSyA1234567890abcdefghijklmnopqrstuv", "google-api-key"),
            ("sk_live_abcdefghijklmnop1234", "stripe-key"),
            ("whsec_abcdefghijklmnop1234", "stripe-webhook-secret"),
            ("hf_abcdefghijklmnopqrstuvwxyz12345678", "huggingface-token"),
        ];
        for (text, kind) in cases {
            let out = r(text);
            assert!(out.contains(&format!("[REDACTED:{kind}]")), "{text} -> {out}");
        }
    }

    #[test]
    fn removes_private_key_blocks_including_truncated_ones() {
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nAAAAB\n-----END OPENSSH PRIVATE KEY-----";
        let out = r(&format!("before\n{key}\nafter"));
        assert_eq!(out, "before\n[REDACTED:private-key]\nafter");

        let cut = "log:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA";
        assert_eq!(r(cut), "log:\n[REDACTED:private-key]");
    }

    #[test]
    fn removes_jwt_bearer_and_url_passwords() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert_eq!(r(&format!("cookie={jwt};")), "cookie=[REDACTED:jwt];");
        assert_eq!(
            r("Authorization: Bearer abc123def456ghi789jkl"),
            "Authorization: Bearer [REDACTED:bearer]"
        );
        assert_eq!(
            r("connecting to postgres://app:s3cretPassw0rd@db.internal:5432/app"),
            "connecting to postgres://app:[REDACTED:password]@db.internal:5432/app"
        );
        assert_eq!(r("GET /v1/x?api_key=abcd1234efgh&page=2"), "GET /v1/x?api_key=[REDACTED:value]&page=2");
    }

    #[test]
    fn removes_secret_assignments_but_keeps_the_name() {
        assert_eq!(r("DB_PASSWORD=hunter2hunter2"), "DB_PASSWORD=[REDACTED:DB_PASSWORD]");
        assert_eq!(r("export STRIPE_KEY=\"abcdefghijk\""), "export STRIPE_KEY=\"[REDACTED:STRIPE_KEY]\"");
        assert_eq!(r("JWT_SECRET: supersecretvalue"), "JWT_SECRET: [REDACTED:JWT_SECRET]");
        assert_eq!(
            r(r#"{"api_key": "abcd1234efgh5678"}"#),
            r#"{"api_key": "[REDACTED:value]"}"#
        );
    }

    #[test]
    fn leaves_ordinary_code_and_prose_alone() {
        let keep = [
            "let token = get_token();",
            "PORT=3000",
            "NODE_ENV=production",
            "AUTHOR=someone",
            "OPENAI_API_KEY=your-api-key-here",
            "API_KEY=${API_KEY}",
            "GITHUB_TOKEN=$(gh auth token)",
            "SECRET_KEY=os.environ[\"SECRET_KEY\"]",
            "the token expires after an hour",
            "test result: ok. 230 passed; 0 failed",
            "https://github.com/qhkm/orc",
            "error[E0308]: mismatched types at src/relay.rs:71",
            "sk-short",
            "TOKEN_TTL_MS=86400000",
        ];
        for text in keep {
            assert_eq!(r(text), text, "should be unchanged");
        }
    }

    #[test]
    fn counts_redactions_and_marks_them() {
        let out = redact("A=1\nGITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789\nDB_PASSWORD=hunter2hunter2");
        assert_eq!(out.count, 2);
        assert!(has_redactions(&out.text));
        assert!(!has_redactions("nothing here"));
    }

    #[test]
    fn redacting_twice_changes_nothing_more() {
        let once = r("DB_PASSWORD=hunter2hunter2 postgres://u:p4ssw0rd99@h/db Bearer abc123def456ghi789jkl");
        assert_eq!(redact(&once).count, 0);
        assert_eq!(r(&once), once);
    }

    #[test]
    fn known_values_are_removed_wherever_they_appear() {
        let red = Redactor::new().with_known_values(["plainpassword".to_string(), "short".to_string()]);
        let out = red.redact("connect failed for plainpassword (user short)");
        assert_eq!(out.text, "connect failed for [REDACTED:value] (user short)");
        assert_eq!(out.count, 1);
    }

    #[test]
    fn reads_secret_values_from_env_files_but_not_templates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".env"),
            "# comment\nPORT=3000\nNODE_ENV=production\nDB_PASSWORD=\"correct horse battery\"\nexport SESSION=a1b2c3d4e5f6g7h8i9\n",
        )
        .unwrap();
        std::fs::write(dir.path().join(".dev.vars"), "CF_API_TOKEN=cf-token-value-1\n").unwrap();
        std::fs::write(dir.path().join(".env.example"), "DB_PASSWORD=examplevalue123\n").unwrap();

        let red = Redactor::for_dir(dir.path());
        let out = red.clean("pw=correct horse battery sess=a1b2c3d4e5f6g7h8i9 cf=cf-token-value-1 env=production port=3000 ex=examplevalue123");
        assert_eq!(
            out,
            "pw=[REDACTED:value] sess=[REDACTED:value] cf=[REDACTED:value] env=production port=3000 ex=examplevalue123"
        );
    }

    #[test]
    fn secret_names() {
        for n in ["OPENAI_API_KEY", "DB_PASSWORD", "GITHUB_TOKEN", "stripe-secret", "SIGNING_KEY", "GOOGLE_CREDENTIALS"] {
            assert!(is_secret_name(n), "{n}");
        }
        for n in ["PORT", "PATH", "HOME", "AUTHOR", "KEYBOARD", "MONKEY_COUNT"] {
            assert!(!is_secret_name(n), "{n}");
        }
    }
}
