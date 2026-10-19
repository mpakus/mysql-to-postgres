//! Explicit PostgreSQL simple-config replacement for MySQL FULLTEXT indexes.
use super::quote_identifier;
use crate::model::{SourceIndex, TablePlan, ValueKind};
use regex::Regex;

pub(super) fn columns(index: &SourceIndex, table: &TablePlan) -> Option<Vec<String>> {
    if index.unique || index.primary || index.parts.is_empty() {
        return None;
    }
    index
        .parts
        .iter()
        .map(|part| {
            if part.expression.is_some() || part.prefix_length.is_some() || part.descending {
                return None;
            }
            table
                .columns
                .iter()
                .find(|column| {
                    Some(&column.source_name) == part.column.as_ref()
                        && column.kind == ValueKind::Text
                })
                .map(|column| column.target_name.clone())
        })
        .collect()
}

pub(crate) fn expression(columns: &[String]) -> String {
    let document = columns
        .iter()
        .map(|column| {
            format!(
                "COALESCE({}::pg_catalog.text, ''::pg_catalog.text)",
                quote_identifier(column)
            )
        })
        .collect::<Vec<_>>()
        .join(" || ' '::pg_catalog.text || ");
    format!("pg_catalog.to_tsvector('pg_catalog.simple'::pg_catalog.regconfig, {document})")
}

/// Recognize only our generated expression, including COALESCE argument boundaries.
/// PostgreSQL adds text coercions and groups concatenations when deparsing.
pub(crate) fn matches(expression: &str, columns: &[String]) -> bool {
    if columns.is_empty() {
        return false;
    }
    let tokens =
        Regex::new(r#"\s+|"(?:[^"]|"")*"|'(?:[^']|'')*'|::|\|\||[a-zA-Z_\x{80}-\x{10FFFF}][a-zA-Z0-9_$\x{80}-\x{10FFFF}]*|[(),.]"#)
            .unwrap();
    let mut end = 0;
    let mut parsed = vec![];
    for token in tokens.find_iter(expression) {
        if token.start() != end {
            return false;
        }
        end = token.end();
        if !token.as_str().trim().is_empty() {
            parsed.push(token.as_str());
        }
    }
    if end != expression.len() {
        return false;
    }
    let expected: Vec<_> = columns
        .iter()
        .enumerate()
        .flat_map(|(index, column)| {
            (index > 0)
                .then_some(None)
                .into_iter()
                .chain([Some(column.clone())])
        })
        .collect();
    let mut parser = Expression { tokens: &parsed };
    parser
        .search_document()
        .is_some_and(|document| document == expected && parser.tokens.is_empty())
}

struct Expression<'a> {
    tokens: &'a [&'a str],
}
impl Expression<'_> {
    fn take(&mut self, token: &str) -> bool {
        if self
            .tokens
            .first()
            .is_some_and(|value| value.eq_ignore_ascii_case(token))
        {
            self.tokens = &self.tokens[1..];
            true
        } else {
            false
        }
    }
    fn require(&mut self, token: &str) -> Option<()> {
        self.take(token).then_some(())
    }
    fn builtin(&mut self, name: &str) -> Option<()> {
        if self.take("pg_catalog") {
            self.require(".")?;
        }
        self.require(name)
    }
    fn cast(&mut self, name: &str) -> Option<()> {
        self.require("::")?;
        self.builtin(name)
    }
    fn search_document(&mut self) -> Option<Vec<Option<String>>> {
        self.builtin("to_tsvector")?;
        self.require("(")?;
        if !self.take("'simple'") && !self.take("'pg_catalog.simple'") {
            return None;
        }
        self.cast("regconfig")?;
        self.require(",")?;
        let document = self.document(0)?;
        self.require(")")?;
        Some(document)
    }
    fn column(&mut self, depth: u8) -> Option<String> {
        if depth > 64 {
            return None;
        }
        let column = if self.take("(") {
            let name = self.column(depth + 1)?;
            self.require(")")?;
            name
        } else {
            let token = *self.tokens.first()?;
            let name = if token.starts_with('"') {
                token[1..token.len() - 1].replace("\"\"", "\"")
            } else if token
                .starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_' || !ch.is_ascii())
            {
                token.to_ascii_lowercase()
            } else {
                return None;
            };
            self.tokens = &self.tokens[1..];
            name
        };
        if self.take("::") {
            self.builtin("text")?;
        }
        Some(column)
    }
    fn document(&mut self, depth: u8) -> Option<Vec<Option<String>>> {
        if depth > 64 {
            return None;
        }
        let mut document = if self.take("(") {
            let values = self.document(depth + 1)?;
            self.require(")")?;
            values
        } else if self.take("coalesce") {
            self.require("(")?;
            let column = self.column(0)?;
            self.require(",")?;
            self.require("''")?;
            self.cast("text")?;
            self.require(")")?;
            vec![Some(column)]
        } else {
            self.require("' '")?;
            self.cast("text")?;
            vec![None]
        };
        if self.take("||") {
            document.extend(self.document(depth + 1)?);
        }
        Some(document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deparse_variations_preserve_exact_columns_null_handling_and_separators() {
        let columns = vec!["name".into(), "Body \" (x)".into()];
        let expected = expression(&columns);
        assert!(matches(&expected, &columns));
        let deparsed = "to_tsvector('simple'::regconfig, ((COALESCE((name)::text, ''::text) || ' '::text) || COALESCE(\"Body \"\" (x)\", ''::text)))";
        assert!(matches(deparsed, &columns));
        for altered in [
            deparsed.replace("'simple'", "'english'"),
            deparsed.replace("' '::text", "''::text"),
            deparsed.replace("''::text", "'fallback'::text"),
            deparsed.replace("(name)::text", "other::text"),
            deparsed.replace("COALESCE((name)::text, ''::text)", "name"),
            deparsed.replace("::text", "::varchar"),
            "to_tsvector('simple'::regconfig, COALESCE(name::text, ''::text || ' '::text || COALESCE(\"Body \"\" (x)\", ''::text)))".into(),
        ] {
            assert!(!matches(&altered, &columns), "{altered}");
        }
        assert!(!matches(
            deparsed,
            &[columns[1].clone(), columns[0].clone()]
        ));
        assert!(!matches(&expected, &[]));
        assert!(matches(
            "to_tsvector('simple'::regconfig, COALESCE(текст, ''::text))",
            &["текст".into()]
        ));
    }
}
