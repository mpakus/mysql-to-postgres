//! Strict one-way MySQL load-file conversion. No legacy code is evaluated.
use super::*;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fmt, fs,
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DefaultPolicy {
    #[default]
    RequireExplicitChoice,
    My2pgReviewed,
}
#[derive(Clone, Debug)]
pub struct ImportOptions {
    pub target_schema: Option<String>,
    pub consistency: Consistency,
    pub source_env: String,
    pub target_env: String,
    pub append_data_only: bool,
    pub default_policy: DefaultPolicy,
}
impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            target_schema: None,
            consistency: Consistency::Frozen,
            source_env: "MY2PG_SOURCE_URL".into(),
            target_env: "MY2PG_TARGET_URL".into(),
            append_data_only: false,
            default_policy: DefaultPolicy::RequireExplicitChoice,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct CompatibilityEntry {
    pub clause: String,
    pub target: String,
    pub span: SourceSpan,
}
#[derive(Clone, Debug, Serialize)]
pub struct CompatibilityReport {
    pub version: u32,
    pub input_sha256: String,
    pub reference_revision: String,
    pub default_policy: DefaultPolicy,
    pub mapped: Vec<CompatibilityEntry>,
    pub warnings: Vec<String>,
    pub explicit_omissions: Vec<String>,
}
#[derive(Debug)]
pub struct ImportedConfig {
    pub config: MigrationConfig,
    pub toml: String,
    pub compatibility: CompatibilityReport,
}
#[derive(Clone, Debug, Serialize)]
pub struct ImportError {
    pub code: String,
    pub message: String,
    pub span: SourceSpan,
}
impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} (line {}, column {})",
            self.code, self.message, self.span.line, self.span.column
        )
    }
}
impl std::error::Error for ImportError {}
fn span(text: &str, start: usize, end: usize) -> SourceSpan {
    let start = start.min(text.len());
    let prefix = &text[..start];
    SourceSpan {
        start,
        end: end.min(text.len()),
        line: prefix.bytes().filter(|b| *b == b'\n').count() + 1,
        column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
    }
}
fn error(text: &str, start: usize, end: usize, code: &str, message: &str) -> ImportError {
    ImportError {
        code: code.into(),
        message: message.into(),
        span: span(text, start, end),
    }
}

#[derive(Clone, PartialEq)]
enum Kind {
    Word,
    Quoted,
    Regex,
    Symbol,
}
#[derive(Clone)]
struct Token {
    kind: Kind,
    value: String,
    start: usize,
    end: usize,
}
fn tokenize(text: &str) -> Result<Vec<Token>, ImportError> {
    if text.len() > 1024 * 1024 || text.contains('\0') {
        return Err(error(
            text,
            0,
            0,
            "IMPORT_INPUT",
            "load input must be at most 1 MiB without NUL",
        ));
    }
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if text[at..].starts_with("--") {
            at += text[at..].find('\n').unwrap_or(bytes.len() - at);
            continue;
        }
        if text[at..].starts_with("/*") {
            let start = at;
            at += 2;
            let mut depth = 1;
            while at < bytes.len() && depth > 0 {
                if text[at..].starts_with("/*") {
                    depth += 1;
                    at += 2;
                } else if text[at..].starts_with("*/") {
                    depth -= 1;
                    at += 2;
                } else {
                    at += text[at..].chars().next().unwrap().len_utf8();
                }
                if depth > 16 {
                    return Err(error(
                        text,
                        start,
                        at,
                        "IMPORT_DEPTH",
                        "comment nesting exceeds limit",
                    ));
                }
            }
            if depth != 0 {
                return Err(error(
                    text,
                    start,
                    at,
                    "IMPORT_SYNTAX",
                    "unterminated comment",
                ));
            }
            continue;
        }
        let start = at;
        let (kind, value) = if matches!(bytes[at], b'\'' | b'"') {
            let quote = bytes[at];
            at += 1;
            let mut value = String::new();
            let mut closed = false;
            while at < bytes.len() {
                if bytes[at] == quote {
                    at += 1;
                    if bytes.get(at) == Some(&quote) {
                        value.push(char::from(quote));
                        at += 1;
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    let ch = text[at..].chars().next().unwrap();
                    // Backslash quoting is intentionally ambiguous across legacy readers.
                    if ch == '\\' {
                        return Err(error(
                            text,
                            start,
                            at + 1,
                            "IMPORT_ESCAPE",
                            "use doubled quotes or percent-encoded URI characters; backslash string escapes are unsupported",
                        ));
                    }
                    value.push(ch);
                    at += ch.len_utf8();
                }
            }
            if !closed {
                return Err(error(
                    text,
                    start,
                    at,
                    "IMPORT_SYNTAX",
                    "unterminated quoted value",
                ));
            }
            (Kind::Quoted, value)
        } else if bytes[at] == b'~' {
            at += 1;
            let Some(&open) = bytes.get(at) else {
                return Err(error(
                    text,
                    start,
                    at,
                    "IMPORT_REGEX",
                    "missing regex delimiter",
                ));
            };
            let close = match open {
                b'/' => b'/',
                b'<' => b'>',
                b'|' => b'|',
                b'\'' => b'\'',
                b'"' => b'"',
                _ => {
                    return Err(error(
                        text,
                        start,
                        at + 1,
                        "IMPORT_REGEX",
                        "unsupported regex delimiter",
                    ));
                }
            };
            at += 1;
            let from = at;
            while at < bytes.len() && bytes[at] != close {
                if bytes[at] == b'\\' && bytes.get(at + 1) == Some(&close) {
                    at += 2;
                } else {
                    at += text[at..].chars().next().unwrap().len_utf8();
                }
            }
            if at == bytes.len() {
                return Err(error(text, start, at, "IMPORT_REGEX", "unterminated regex"));
            }
            let value = text[from..at].to_owned();
            at += 1;
            (Kind::Regex, value)
        } else {
            let remaining = &text[at..];
            let first = remaining
                .split(|ch: char| ch.is_whitespace() || ch == ';')
                .next()
                .unwrap();
            if first.contains("://") || first.starts_with("jdbc:") {
                at += first.len();
                (Kind::Word, first.to_owned())
            } else if b",;().=".contains(&bytes[at]) {
                at += 1;
                (Kind::Symbol, text[start..at].into())
            } else {
                while at < bytes.len()
                    && !bytes[at].is_ascii_whitespace()
                    && !b",;().=".contains(&bytes[at])
                {
                    at += text[at..].chars().next().unwrap().len_utf8();
                }
                // Comparisons need their complete operator; '=' is otherwise punctuation.
                if at > start
                    && matches!(&text[start..at], "<" | ">")
                    && bytes.get(at) == Some(&b'=')
                {
                    at += 1;
                }
                (Kind::Word, text[start..at].to_owned())
            }
        };
        if value.contains("{{")
            || value.contains("}}")
            || (kind != Kind::Regex
                && kind != Kind::Quoted
                && (value.contains('#') || value.starts_with('$')))
        {
            return Err(error(
                text,
                start,
                at,
                "IMPORT_READFORM",
                "templates, reader forms and code expansion are unsupported",
            ));
        }
        tokens.push(Token {
            kind,
            value,
            start,
            end: at,
        });
        if tokens.len() > 8192 {
            return Err(error(
                text,
                start,
                at,
                "IMPORT_INPUT",
                "load token count exceeds limit",
            ));
        }
    }
    Ok(tokens)
}
struct Parser<'a> {
    text: &'a str,
    tokens: Vec<Token>,
    at: usize,
    config: MigrationConfig,
    report: CompatibilityReport,
    seen: BTreeSet<String>,
    database: String,
}
impl Parser<'_> {
    fn token(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }
    fn is(&self, value: &str) -> bool {
        self.token().is_some_and(|t| {
            t.kind != Kind::Quoted && t.kind != Kind::Regex && t.value.eq_ignore_ascii_case(value)
        })
    }
    fn take(&mut self) -> Result<Token, ImportError> {
        let token = self
            .token()
            .cloned()
            .ok_or_else(|| self.fail("IMPORT_SYNTAX", "unexpected end of load command"))?;
        self.at += 1;
        Ok(token)
    }
    fn eat(&mut self, value: &str) -> bool {
        if self.is(value) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, value: &str) -> Result<(), ImportError> {
        if self.eat(value) {
            Ok(())
        } else {
            Err(self.fail(
                "IMPORT_SYNTAX",
                "missing required grammar keyword or separator",
            ))
        }
    }
    fn fail(&self, code: &str, message: &str) -> ImportError {
        let (start, end) = self
            .token()
            .map_or((self.text.len(), self.text.len()), |t| (t.start, t.end));
        error(self.text, start, end, code, message)
    }
    fn at_error(&self, t: &Token, code: &str, message: &str) -> ImportError {
        error(self.text, t.start, t.end, code, message)
    }
    fn quoted(&mut self) -> Result<String, ImportError> {
        let token = self.take()?;
        if token.kind != Kind::Quoted {
            return Err(self.at_error(&token, "IMPORT_SYNTAX", "requires a quoted literal"));
        }
        Ok(token.value)
    }
    fn name(&mut self) -> Result<String, ImportError> {
        let token = self.take()?;
        if !matches!(token.kind, Kind::Word | Kind::Quoted) || token.value.is_empty() {
            return Err(self.at_error(&token, "IMPORT_SYNTAX", "requires a finite name"));
        }
        Ok(token.value)
    }
    fn mapped(&mut self, start: usize, clause: &str, target: &str) {
        let end = self
            .tokens
            .get(self.at.saturating_sub(1))
            .map_or(start, |t| t.end);
        self.report.mapped.push(CompatibilityEntry {
            clause: clause.into(),
            target: target.into(),
            span: span(self.text, start, end),
        });
    }
    fn unique(&mut self, key: &str, t: &Token) -> Result<(), ImportError> {
        if !self.seen.insert(key.into()) {
            Err(self.at_error(
                t,
                "IMPORT_CONFLICT",
                "duplicate or conflicting legacy option",
            ))
        } else {
            Ok(())
        }
    }
    fn mode(&mut self, mode: MigrationMode, t: &Token) -> Result<(), ImportError> {
        if self.seen.contains("mode") && self.config.migration.mode != mode {
            return Err(self.at_error(t, "IMPORT_CONFLICT", "conflicting migration modes"));
        }
        self.seen.insert("mode".into());
        self.config.migration.mode = mode;
        Ok(())
    }
    fn options(&mut self) -> Result<(), ImportError> {
        loop {
            let first = self.take()?;
            let start = first.start;
            let mut words = vec![first.value.to_ascii_lowercase()];
            while !self.is("=") && !self.is(",") && !self.boundary() {
                words.push(self.take()?.value.to_ascii_lowercase());
            }
            let key = words.join(" ");
            if self.eat("=") {
                let value = self.take()?;
                if value.kind != Kind::Word {
                    return Err(self.at_error(
                        &value,
                        "IMPORT_NUMBER",
                        "requires a positive numeric limit",
                    ));
                }
                self.unique(&key, &first)?;
                match key.as_str() {
                    "workers"|"batch rows"|"max parallel create index"|"concurrency" => {
                        let n=value.value.parse::<usize>().ok().filter(|n|*n>0)
                            .ok_or_else(||self.at_error(&value,"IMPORT_NUMBER","numeric limit is invalid or out of range"))?;
                        match key.as_str() {
                            "workers"=>self.config.migration.table_workers=n,
                            "batch rows"=>self.config.migration.batch_rows=n,
                            "max parallel create index"=>self.config.migration.index_workers=n,
                            _ if n==1=>{},
                            _=>return Err(self.at_error(&first,"IMPORT_READER_POLICY","range concurrency needs the explicit frozen-source range-reader contract")),
                        }
                    }
                    "batch size"=> {
                        let mut bytes=value.value;
                        if bytes.bytes().all(|b|b.is_ascii_digit()) {bytes.push_str(&self.name()?);}
                        self.config.migration.batch_bytes=byte_limit(&bytes).ok_or_else(||self.at_error(&first,"IMPORT_NUMBER","batch byte limit requires a checked B/kB/MB/GB unit"))?;
                    }
                    "prefetch rows"|"batch concurrency"|"rows per range"|"chunk size"=>return Err(self.at_error(&first,"IMPORT_UNIT_POLICY","legacy queue/range units require an explicit TOML choice; no implicit conversion")),
                    _=>return Err(self.at_error(&first,"IMPORT_OPTION","unsupported WITH option")),
                }
            } else {
                match key.as_str() {
                    "include drop" => {
                        self.unique("existing", &first)?;
                        self.config.target.on_existing = ExistingPolicy::Recreate;
                    }
                    "include no drop" => {
                        self.unique("existing", &first)?;
                    }
                    "truncate" => {
                        self.unique("truncate", &first)?;
                        if self.config.target.on_existing == ExistingPolicy::Recreate {
                            return Err(self.at_error(
                                &first,
                                "IMPORT_CONFLICT",
                                "recreate and truncate cannot be combined",
                            ));
                        }
                        self.config.target.on_existing = ExistingPolicy::Truncate;
                    }
                    "no truncate" => {
                        self.unique("truncate", &first)?;
                    }
                    "data only" | "create no tables" => {
                        if self.seen.contains("create tables") {
                            return Err(self.at_error(
                                &first,
                                "IMPORT_CONFLICT",
                                "data-only and create tables conflict",
                            ));
                        }
                        self.mode(MigrationMode::DataOnly, &first)?
                    }
                    "schema only" => self.mode(MigrationMode::SchemaOnly, &first)?,
                    "create tables" => {
                        if self.config.migration.mode == MigrationMode::DataOnly {
                            return Err(self.at_error(
                                &first,
                                "IMPORT_CONFLICT",
                                "data-only and create tables conflict",
                            ));
                        }
                        self.unique(&key, &first)?;
                    }
                    "create indexes" | "foreign keys" => {
                        self.unique(&key, &first)?;
                    }
                    "reset sequences" | "reset no sequences" | "no reset sequences" => {
                        self.unique("sequences", &first)?;
                        self.config.migration.reset_sequences = key == "reset sequences";
                    }
                    "quote identifiers" | "downcase identifiers" | "snake_case identifiers" => {
                        self.unique("identifiers", &first)?;
                        self.config.migration.identifiers = match key.as_str() {
                            "quote identifiers" => IdentifierPolicy::Preserve,
                            "downcase identifiers" => IdentifierPolicy::Downcase,
                            _ => IdentifierPolicy::SnakeCase,
                        };
                    }
                    "single reader per thread" => {
                        self.unique("readers", &first)?;
                    }
                    "on error stop" => {
                        self.unique("row_error", &first)?;
                    }
                    "multiple readers per thread" => {
                        return Err(self.at_error(
                            &first,
                            "IMPORT_READER_POLICY",
                            "multiple readers require an explicit frozen-source range policy",
                        ));
                    }
                    _ => {
                        return Err(self.at_error(
                            &first,
                            "IMPORT_OPTION",
                            "unsupported WITH option",
                        ));
                    }
                }
            }
            self.mapped(start, "WITH", &format!("migration/target: {key}"));
            if !self.eat(",") {
                break;
            }
            if self.boundary() {
                return Err(self.fail("IMPORT_SYNTAX", "trailing option comma"));
            }
        }
        Ok(())
    }
    fn boundary(&self) -> bool {
        self.token().is_none()
            || [
                ";",
                "CAST",
                "SET",
                "ALTER",
                "INCLUDING",
                "EXCLUDING",
                "BEFORE",
                "AFTER",
                "DECODING",
                "MATERIALIZE",
                "DISTRIBUTE",
                "WITH",
            ]
            .iter()
            .any(|word| self.is(word))
    }
    fn filters(&mut self, include: bool, start: usize) -> Result<(), ImportError> {
        if include {
            self.expect("ONLY")?;
        }
        self.expect("TABLE")?;
        self.expect("NAMES")?;
        self.expect("MATCHING")?;
        loop {
            let token = self.take()?;
            match token.kind {
                Kind::Quoted => {
                    if include {
                        self.config.tables.include.push(token.value);
                    } else {
                        self.config.tables.exclude.push(token.value);
                    }
                }
                Kind::Regex => {
                    if !compatible_regex(&token.value) {
                        return Err(self.at_error(&token,"IMPORT_REGEX","regex requires the common finite literal/class/group/quantifier subset"));
                    }
                    if include {
                        self.config.tables.include_regex.push(token.value);
                    } else {
                        self.config.tables.exclude_regex.push(token.value);
                    }
                }
                _ => {
                    return Err(self.at_error(
                        &token,
                        "IMPORT_SYNTAX",
                        "filter requires a quoted exact name or supported regex",
                    ));
                }
            }
            if !self.eat(",") {
                break;
            }
        }
        self.mapped(
            start,
            if include { "INCLUDING" } else { "EXCLUDING" },
            "tables original-name filters",
        );
        Ok(())
    }
    fn alter(&mut self, start: usize) -> Result<(), ImportError> {
        if self.eat("SCHEMA") {
            let old = self.quoted()?;
            self.expect("RENAME")?;
            self.expect("TO")?;
            let new = self.quoted()?;
            if old != self.database {
                return Err(self.fail(
                    "IMPORT_SCHEMA",
                    "schema rename must match the decoded MySQL database",
                ));
            }
            if self.seen.contains("schema") && self.config.target.schema != new {
                return Err(self.fail("IMPORT_CONFLICT", "destination schema choices conflict"));
            }
            self.config.target.schema = new;
            self.seen.insert("schema".into());
            self.mapped(start, "ALTER SCHEMA", "target.schema");
        } else {
            self.expect("TABLE")?;
            self.expect("NAMES")?;
            self.expect("MATCHING")?;
            let original = self.quoted()?;
            if self.eat("IN") {
                self.expect("SCHEMA")?;
                if self.quoted()? != self.database {
                    return Err(
                        self.fail("IMPORT_SCHEMA", "table scope must match source database")
                    );
                }
            }
            let (name, schema) = if self.eat("RENAME") {
                self.expect("TO")?;
                (Some(self.quoted()?), None)
            } else {
                self.expect("SET")?;
                self.expect("SCHEMA")?;
                (None, Some(self.quoted()?))
            };
            if let Some(mapping) = self
                .config
                .tables
                .rename
                .iter_mut()
                .find(|r| r.source == original)
            {
                if (name.is_some() && mapping.target != mapping.source)
                    || (schema.is_some() && mapping.schema.is_some())
                {
                    return Err(self.fail("IMPORT_CONFLICT", "duplicate table rename action"));
                }
                if let Some(name) = name {
                    mapping.target = name;
                }
                if schema.is_some() {
                    mapping.schema = schema;
                }
            } else {
                self.config.tables.rename.push(TableRename {
                    source: original.clone(),
                    target: name.unwrap_or(original),
                    schema,
                });
            }
            self.mapped(start, "ALTER TABLE", "tables.rename");
        }
        Ok(())
    }
    fn sessions(&mut self, start: usize) -> Result<(), ImportError> {
        let source = if self.eat("MYSQL") {
            self.expect("PARAMETERS")?;
            true
        } else {
            if self.eat("POSTGRESQL") {
                self.expect("PARAMETERS")?;
            }
            false
        };
        loop {
            let key = self.name()?.to_ascii_lowercase();
            if !self.eat("TO") {
                self.expect("=")?;
            }
            let value = self.quoted()?;
            let settings = if source {
                &mut self.config.source.session
            } else {
                &mut self.config.target.session
            };
            if settings.insert(key, value).is_some() {
                return Err(self.fail("IMPORT_CONFLICT", "duplicate session setting"));
            }
            if !self.eat(",") {
                break;
            }
        }
        self.mapped(
            start,
            "SET",
            if source {
                "source.session"
            } else {
                "target.session"
            },
        );
        Ok(())
    }
    fn casts(&mut self, start: usize) -> Result<(), ImportError> {
        loop {
            let cast_start = self.token().map_or(start, |t| t.start);
            let mut rule = CastRule::default();
            if self.eat("TYPE") {
                rule.source_type = Some(self.name()?.to_ascii_lowercase());
            } else {
                self.expect("COLUMN")?;
                rule.source_table = Some(self.name()?);
                self.expect(".")?;
                rule.source_column = Some(self.name()?);
            }
            let mut guardsets = vec![Guards::default()];
            let mut actions = BTreeSet::new();
            loop {
                if self.eat("WHEN") {
                    if self.is("(") {
                        let parsed = self.guards(0)?;
                        guardsets = combine(self, guardsets, parsed)?;
                    } else if self.eat("SIGNED") {
                        set_bool(self, &mut rule.unsigned, false)?;
                    } else if self.eat("UNSIGNED") {
                        set_bool(self, &mut rule.unsigned, true)?;
                    } else if self.eat("DEFAULT") {
                        if rule.default.is_some() {
                            return Err(self.fail("IMPORT_CONFLICT", "duplicate default guard"));
                        }
                        rule.default = Some(self.quoted()?);
                    } else {
                        self.expect("NOT")?;
                        self.expect("NULL")?;
                        set_bool(self, &mut rule.not_null, true)?;
                    }
                } else if self.eat("AND") {
                    self.expect("NOT")?;
                    self.expect("NULL")?;
                    set_bool(self, &mut rule.not_null, true)?;
                } else if self.is("WITH")
                    && self
                        .tokens
                        .get(self.at + 1)
                        .is_some_and(|token| token.value.eq_ignore_ascii_case("EXTRA"))
                {
                    self.expect("WITH")?;
                    self.expect("EXTRA")?;
                    self.expect("AUTO_INCREMENT")?;
                    set_bool(self, &mut rule.auto_increment, true)?;
                } else if self.eat("TO") {
                    if !actions.insert("type") {
                        return Err(self.fail("IMPORT_CONFLICT", "duplicate target type"));
                    }
                    let mut kind = self.name()?.to_ascii_lowercase();
                    if self.eat("(") {
                        kind.push('(');
                        kind.push_str(&self.name()?);
                        if self.eat(",") {
                            kind.push(',');
                            kind.push_str(&self.name()?);
                        }
                        self.expect(")")?;
                        kind.push(')');
                    }
                    if (self.is("WITH") || self.is("WITHOUT"))
                        && self
                            .tokens
                            .get(self.at + 1)
                            .is_some_and(|token| token.value.eq_ignore_ascii_case("TIME"))
                    {
                        kind.push(' ');
                        kind.push_str(&self.take()?.value.to_ascii_lowercase());
                        self.expect("TIME")?;
                        self.expect("ZONE")?;
                        kind.push_str(" time zone");
                    } else if kind == "double" && self.eat("PRECISION") {
                        kind.push_str(" precision");
                    } else if kind == "character" && self.eat("VARYING") {
                        kind.push_str(" varying");
                    }
                    rule.target_type = Some(kind);
                } else if self.is("DROP") || self.is("KEEP") {
                    let drop = self.eat("DROP");
                    if !drop {
                        self.expect("KEEP")?;
                    }
                    if self.eat("DEFAULT") {
                        if !actions.insert("default") {
                            return Err(self.fail("IMPORT_CONFLICT", "duplicate default action"));
                        }
                        rule.drop_default = drop;
                    } else if self.eat("TYPEMOD") {
                        if !actions.insert("typemod") {
                            return Err(self.fail("IMPORT_CONFLICT", "duplicate typemod action"));
                        }
                        rule.drop_typemod = drop;
                    } else {
                        self.expect("NOT")?;
                        self.expect("NULL")?;
                        if !actions.insert("null") {
                            return Err(self.fail("IMPORT_CONFLICT", "duplicate null action"));
                        }
                        rule.drop_not_null = drop;
                    }
                } else if self.eat("USING") {
                    let transform = self.take()?;
                    if transform.kind != Kind::Word
                        || !TRANSFORMS.contains(&transform.value.as_str())
                    {
                        return Err(self.at_error(
                            &transform,
                            "IMPORT_TRANSFORM",
                            "only implemented named transforms are accepted; no package/code forms",
                        ));
                    }
                    if rule.transform.replace(transform.value).is_some() {
                        return Err(self.fail("IMPORT_CONFLICT", "duplicate transform"));
                    }
                } else {
                    break;
                }
            }
            if guardsets
                .iter()
                .any(|guard| guard.precision.is_some() || guard.scale.is_some())
                && !matches!(rule.source_type.as_deref(), Some("decimal" | "numeric"))
            {
                return Err(error(
                    self.text,
                    cast_start,
                    self.tokens
                        .get(self.at.saturating_sub(1))
                        .map_or(cast_start, |t| t.end),
                    "IMPORT_GUARD",
                    "precision/scale guards require an explicit decimal/numeric source type; display widths and unknown column types are not equivalent",
                ));
            }
            if rule.source_column.is_some()
                && (rule.unsigned.is_some()
                    || rule.not_null.is_some()
                    || rule.default.is_some()
                    || rule.auto_increment.is_some())
            {
                return Err(error(
                    self.text,
                    cast_start,
                    self.tokens
                        .get(self.at.saturating_sub(1))
                        .map_or(cast_start, |token| token.end),
                    "IMPORT_GUARD",
                    "legacy column casts ignore source guards; choose an explicit reviewed TOML rule instead",
                ));
            }
            if rule.source_type.is_some() && rule.auto_increment.is_none() {
                // Both pinned legacy matchers require source/rule AUTO_INCREMENT agreement.
                rule.auto_increment = Some(false);
            }
            for guards in guardsets {
                let mut expanded = rule.clone();
                expanded.precision = guards.precision;
                expanded.scale = guards.scale;
                self.config.cast.push(expanded);
            }
            self.mapped(cast_start, "CAST", "ordered cast rules");
            if !self.eat(",") {
                break;
            }
        }
        Ok(())
    }
    fn guards(&mut self, depth: usize) -> Result<Vec<Guards>, ImportError> {
        if depth > 16 {
            return Err(self.fail("IMPORT_DEPTH", "guard nesting exceeds limit"));
        }
        self.expect("(")?;
        let op = self.take()?.value.to_ascii_lowercase();
        let result = if op == "and" || op == "or" {
            let mut result = if op == "and" {
                vec![Guards::default()]
            } else {
                vec![]
            };
            let mut children = 0;
            while self.is("(") {
                children += 1;
                let child = self.guards(depth + 1)?;
                if op == "and" {
                    result = combine(self, result, child)?;
                } else {
                    result.extend(child);
                }
                if result.len() > 32 {
                    return Err(self.fail("IMPORT_GUARD", "guard expansion exceeds32 rules"));
                }
            }
            if children < 2 {
                return Err(self.fail(
                    "IMPORT_GUARD",
                    "and/or requires at least two finite comparison guards",
                ));
            }
            result
        } else {
            let left = self.take()?;
            let right = self.take()?;
            let left_name = left.value.to_ascii_lowercase();
            let right_name = right.value.to_ascii_lowercase();
            let (name, value, reverse) = if matches!(left_name.as_str(), "precision" | "scale") {
                (&left_name, &right.value, false)
            } else if matches!(right_name.as_str(), "precision" | "scale") {
                (&right_name, &left.value, true)
            } else {
                return Err(self.fail(
                    "IMPORT_GUARD",
                    "comparison requires exactly one precision/scale variable",
                ));
            };
            let value = value.parse::<u32>().map_err(|_| {
                self.fail(
                    "IMPORT_GUARD",
                    "guard constant must be a bounded nonnegative integer",
                )
            })?;
            let comparison = match (op.as_str(), reverse) {
                ("=", _) => Comparison::Eq,
                ("<", false) | (">", true) => Comparison::Lt,
                ("<=", false) | (">=", true) => Comparison::Le,
                (">", false) | ("<", true) => Comparison::Gt,
                (">=", false) | ("<=", true) => Comparison::Ge,
                _ => {
                    return Err(self.fail("IMPORT_GUARD", "unsupported finite comparison operator"));
                }
            };
            let mut guards = Guards::default();
            let guard = Some(NumericGuard {
                op: comparison,
                value,
            });
            if name == "precision" {
                guards.precision = guard;
            } else {
                guards.scale = guard;
            }
            vec![guards]
        };
        self.expect(")")?;
        Ok(result)
    }
}
#[derive(Clone, Default)]
struct Guards {
    precision: Option<NumericGuard>,
    scale: Option<NumericGuard>,
}
fn combine(
    p: &Parser<'_>,
    left: Vec<Guards>,
    right: Vec<Guards>,
) -> Result<Vec<Guards>, ImportError> {
    if left.len().saturating_mul(right.len()) > 32 {
        return Err(p.fail("IMPORT_GUARD", "guard expansion exceeds32 rules"));
    }
    let mut result = Vec::new();
    for a in left {
        for b in &right {
            if (a.precision.is_some() && b.precision.is_some())
                || (a.scale.is_some() && b.scale.is_some())
            {
                return Err(p.fail(
                    "IMPORT_GUARD",
                    "conjunction has multiple bounds for one field; choose explicit TOML rules",
                ));
            }
            result.push(Guards {
                precision: a.precision.clone().or(b.precision.clone()),
                scale: a.scale.clone().or(b.scale.clone()),
            });
        }
    }
    Ok(result)
}
fn set_bool(p: &Parser<'_>, field: &mut Option<bool>, value: bool) -> Result<(), ImportError> {
    if field.replace(value).is_some() {
        Err(p.fail("IMPORT_CONFLICT", "duplicate boolean guard"))
    } else {
        Ok(())
    }
}
fn byte_limit(value: &str) -> Option<usize> {
    let split = value.find(|ch: char| !ch.is_ascii_digit())?;
    let count = value[..split].parse::<usize>().ok().filter(|n| *n > 0)?;
    let multiplier = match value[split..].to_ascii_uppercase().as_str() {
        "B" => 1,
        "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        _ => return None,
    };
    count.checked_mul(multiplier)
}
fn compatible_regex(value: &str) -> bool {
    if value.is_empty() || value.contains("(?") {
        return false;
    }
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && !chars
                .next()
                .is_some_and(|ch| ".^$*+?{}[]()|/\\-".contains(ch))
        {
            return false;
        }
    }
    regex::Regex::new(value).is_ok()
}

fn decoded(value: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let source = value.as_bytes();
    let mut at = 0;
    while at < source.len() {
        if source[at] == b'%' {
            let pair = source.get(at + 1..at + 3)?;
            let hex = std::str::from_utf8(pair).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            bytes.push(source[at]);
            at += 1;
        }
    }
    String::from_utf8(bytes)
        .ok()
        .filter(|value| !value.contains('\0'))
}
fn uri(token: &Token, source: bool, text: &str) -> Result<(String, String), ImportError> {
    let fail = || {
        error(
            text,
            token.start,
            token.end,
            "IMPORT_URI",
            "requires an explicit percent-encoded MySQL/PostgreSQL TCP URL without legacy/JDBC/options-file parameters",
        )
    };
    if !matches!(token.kind, Kind::Word | Kind::Quoted) {
        return Err(fail());
    }
    let mut value = token.value.clone();
    if !source && value.to_ascii_lowercase().starts_with("pgsql://") {
        value = format!("postgresql://{}", &value[8..]);
    }
    // Double-at/colon legacy reader semantics differ; reject instead of guessing.
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(""))
        .ok_or_else(fail)?;
    if authority.contains("@@")
        || authority
            .split('@')
            .next()
            .is_some_and(|part| part.contains("::"))
    {
        return Err(fail());
    }
    let parsed = url::Url::parse(&value).map_err(|_| fail())?;
    if (source && parsed.scheme() != "mysql")
        || (!source && !matches!(parsed.scheme(), "postgresql" | "postgres"))
    {
        return Err(fail());
    }
    let database = decoded(parsed.path().strip_prefix('/').unwrap_or("")).ok_or_else(fail)?;
    if database.is_empty() || database.contains('/') {
        return Err(fail());
    }
    Ok((value, database))
}

pub fn parse(text: &str, options: &ImportOptions) -> Result<ImportedConfig, ImportError> {
    let tokens = tokenize(text)?;
    let initial = span(text, 0, 0);
    if options.source_env == options.target_env {
        return Err(error(
            text,
            0,
            0,
            "IMPORT_OPTIONS",
            "source and target credential references must differ",
        ));
    }
    let config:MigrationConfig=serde_json::from_value(serde_json::json!({
        "version":1,"source":{"url_env":options.source_env,"consistency":options.consistency},
        "target":{"url_env":options.target_env,"schema":options.target_schema.clone().unwrap_or_else(||"pending_schema".into())}
    })).map_err(|_|error(text,0,0,"IMPORT_OPTIONS","invalid import options"))?;
    let report=CompatibilityReport {version:1,input_sha256:format!("{:x}",Sha256::digest(text.as_bytes())),
        reference_revision:"231ab86778ca5ffd7de40878714760c8b4860cdf".into(),default_policy:options.default_policy,
        mapped:vec![],warnings:vec![
            "Reviewed my2pg defaults: no display-width boolean/integer inference; DATETIME retains wall-clock timestamp and TIME uses duration interval.".into(),
            "Legacy zero-date/default/NUL/collation behavior is not silently inherited; explicit casts and planning remain required.".into(),
            "Identifiers preserve case unless explicitly mapped; collision/dependency checks occur during planning.".into(),
            "Name/default guards use exact case-sensitive target semantics; review source names and literal case against the plan.".into(),
            "TLS verifies hostnames by default; configure reviewed CA/session settings before migration.".into(),
            "Legacy worker and byte limits map to named my2pg budgets; resource incompatibilities fail instead of resizing memory.".into(),
            "This is configuration translation, not execution/compatibility certification; run check and inspect plan.".into(),
        ],explicit_omissions:vec![
            "Legacy connection credentials/URLs are omitted. Set the source and target URL environment references in the generated configuration.".into(),
            "Implicit option-file/environment/template lookup is omitted; no legacy code, SQL or transform is executed.".into(),
        ]};
    let mut p = Parser {
        text,
        tokens,
        at: 0,
        config,
        report,
        seen: BTreeSet::new(),
        database: String::new(),
    };
    if options.target_schema.is_some() {
        p.seen.insert("schema".into());
    }
    p.expect("LOAD")?;
    p.expect("DATABASE")?;
    p.expect("FROM")?;
    let source_token = p.take()?;
    let (source, dbname) = uri(&source_token, true, text)?;
    p.database = dbname;
    p.expect("INTO")?;
    let target_token = p.take()?;
    let (target, _) = uri(&target_token, false, text)?;
    // Pure validation shares the real connection contract, without reading env/files.
    resolve_credentials_with(&p.config, |name| {
        if name == options.source_env {
            Some(source.clone())
        } else if name == options.target_env {
            Some(target.clone())
        } else {
            None
        }
    })
    .map_err(|failure| {
        error(
            text,
            if failure.field.starts_with("source") {
                source_token.start
            } else {
                target_token.start
            },
            if failure.field.starts_with("source") {
                source_token.end
            } else {
                target_token.end
            },
            "IMPORT_URI",
            "legacy URI is outside the executable network/credential contract",
        )
    })?;
    p.mapped(
        0,
        "LOAD DATABASE",
        "source/target URL environment references",
    );
    while !p.is(";") {
        let clause = p.take()?;
        let start = clause.start;
        match clause.value.to_ascii_uppercase().as_str() {
            "WITH"=>p.options()?,"CAST"=>p.casts(start)?,"INCLUDING"=>p.filters(true,start)?,
            "EXCLUDING"=>p.filters(false,start)?,"ALTER"=>p.alter(start)?,"SET"=>p.sessions(start)?,
            "BEFORE"|"AFTER"|"DECODING"|"MATERIALIZE"=>return Err(p.at_error(&clause,"IMPORT_CAPABILITY","clause requires the corresponding reviewed hook/view/decoding capability; import is not partial")),
            _=>return Err(p.at_error(&clause,"IMPORT_CLAUSE","unsupported MySQL load clause")),
        }
    }
    p.expect(";")?;
    if p.token().is_some() {
        return Err(p.fail("IMPORT_COMMANDS", "exactly one load command is accepted"));
    }
    if !p.seen.contains("schema") {
        return Err(ImportError {
            code: "IMPORT_SCHEMA_REQUIRED".into(),
            message: "choose an explicit destination schema or matching ALTER SCHEMA rename".into(),
            span: initial,
        });
    }
    if options.append_data_only {
        if p.config.migration.mode != MigrationMode::DataOnly
            || p.config.target.on_existing != ExistingPolicy::Error
        {
            return Err(error(
                text,
                0,
                0,
                "IMPORT_CONFLICT",
                "append opt-in requires data-only without recreate/truncate",
            ));
        }
        p.config.target.on_existing = ExistingPolicy::Append;
    }
    if p.config.migration.mode == MigrationMode::DataOnly
        && p.config.target.on_existing == ExistingPolicy::Error
    {
        return Err(error(
            text,
            0,
            text.len(),
            "IMPORT_CONFLICT",
            "data-only import requires --append-data-only or an explicit truncate policy",
        ));
    }
    if p.config.target.on_existing == ExistingPolicy::Recreate && p.seen.contains("truncate") {
        // A no-truncate declaration is compatible with scoped replacement.
        if p.report
            .mapped
            .iter()
            .any(|entry| entry.target == "migration/target: truncate")
        {
            return Err(error(
                text,
                0,
                0,
                "IMPORT_CONFLICT",
                "recreate and truncate cannot be combined",
            ));
        }
    }
    if options.default_policy != DefaultPolicy::My2pgReviewed {
        return Err(error(
            text,
            0,
            0,
            "IMPORT_DEFAULT_POLICY",
            "explicitly review and choose my2pg defaults; legacy implicit casts are not equivalent",
        ));
    }
    validate(&p.config).map_err(|_|error(text,0,text.len(),"IMPORT_CONFIG","translated fields conflict with executable TOML validation; review sizing/types/session policy"))?;
    let normalized = toml::to_string_pretty(&p.config).map_err(|_| {
        error(
            text,
            0,
            0,
            "IMPORT_CONFIG",
            "cannot serialize translated configuration",
        )
    })?;
    super::parse(&normalized, Path::new(".")).map_err(|_| {
        error(
            text,
            0,
            0,
            "IMPORT_CONFIG",
            "normalized TOML did not pass the real configuration parser",
        )
    })?;
    let summary = serde_json::to_string_pretty(&p.report).map_err(|_| {
        error(
            text,
            0,
            0,
            "IMPORT_REPORT",
            "cannot serialize compatibility report",
        )
    })?;
    let mut toml =
        String::from("# my2pg strict import compatibility report; review before execution\n");
    for line in summary.lines() {
        toml.push_str("# ");
        toml.push_str(line);
        toml.push('\n');
    }
    toml.push('\n');
    toml.push_str(&normalized);
    Ok(ImportedConfig {
        config: p.config,
        toml,
        compatibility: p.report,
    })
}

/// Publish one fully parsed TOML/report artifact atomically; never overwrite.
pub fn publish_new(imported: &ImportedConfig, output: &Path) -> Result<(), ImportError> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let failure = || ImportError {
        code: "IMPORT_OUTPUT".into(),
        message: "cannot publish private import artifact; existing outputs are never overwritten"
            .into(),
        span: SourceSpan {
            start: 0,
            end: 0,
            line: 1,
            column: 1,
        },
    };
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = format!(
        ".my2pg-import-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let staged = parent.join(name);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&staged).map_err(|_| failure())?;
    let result = (|| {
        file.write_all(imported.toml.as_bytes())?;
        file.sync_all()?;
        fs::hard_link(&staged, output)
    })()
    .map_err(|_| failure());
    drop(file);
    // Cleanup never removes an existing/final output owned by another import.
    let cleanup = fs::remove_file(&staged);
    result?;
    cleanup.map_err(|_| failure())?;
    // Persist both directory entries after publication and staging cleanup.
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| ImportError {
            message:
                "complete artifact was published, but directory durability could not be confirmed"
                    .into(),
            ..failure()
        })
}
