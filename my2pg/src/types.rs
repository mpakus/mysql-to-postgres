//! Pure PostgreSQL type signatures shared by planning and verification.
//! This recognizes a finite grammar, not arbitrary SQL or catalog/search-path resolution.

pub fn equivalent_postgres_types(actual: &str, expected: &str) -> bool {
    match (
        normalize_postgres_type(actual),
        normalize_postgres_type(expected),
    ) {
        (Some(actual), Some(expected)) => actual == expected,
        _ => false,
    }
}

pub fn normalize_postgres_type(value: &str) -> Option<String> {
    let mut value = value.trim();
    if value.is_empty() || value.contains('\0') {
        return None;
    }
    let mut arrays = 0usize;
    while let Some(element) = value.strip_suffix("[]") {
        arrays = arrays.checked_add(1)?;
        value = element.trim_end();
    }
    if value.contains('"') || value.contains('.') {
        let identity = qualified_identity(value)?;
        return Some(format!("{identity}{}", "[]".repeat(arrays)));
    }
    let lower = value.to_ascii_lowercase();
    let (head, modifier, tail) = if let Some((head, rest)) = lower.split_once('(') {
        let (modifier, tail) = rest.split_once(')')?;
        if modifier.contains(['(', ')']) || tail.contains(['(', ')']) {
            return None;
        }
        (head, Some(modifier), tail)
    } else {
        (lower.as_str(), None, "")
    };
    let head = head.split_whitespace().collect::<Vec<_>>().join(" ");
    let tail = tail.split_whitespace().collect::<Vec<_>>().join(" ");
    if matches!(
        head.as_str(),
        "timestamp with time zone"
            | "timestamp without time zone"
            | "time with time zone"
            | "time without time zone"
    ) && (!tail.is_empty() || modifier.is_some())
    {
        return None;
    }
    let (base, timezone) = match head.as_str() {
        "timestamp with time zone" => ("timestamp", "with time zone"),
        "timestamp without time zone" => ("timestamp", "without time zone"),
        "time with time zone" => ("time", "with time zone"),
        "time without time zone" => ("time", "without time zone"),
        other => (other, tail.as_str()),
    };
    let base = match base {
        "int2" => "smallint",
        "int" | "int4" => "integer",
        "int8" => "bigint",
        "float4" => "real",
        "float8" => "double precision",
        "bool" => "boolean",
        "character varying" => "varchar",
        "char" | "bpchar" => "character",
        "decimal" => "numeric",
        "bit varying" => "varbit",
        other => other,
    };
    let canonical = match (base, timezone) {
        ("timestamp", "with time zone") => "timestamptz",
        ("timestamp", "without time zone" | "") => "timestamp",
        ("time", "with time zone") => "timetz",
        ("time", "without time zone" | "") => "time",
        (_, "") => base,
        _ => return None,
    };
    if !matches!(
        canonical,
        "smallint"
            | "integer"
            | "bigint"
            | "real"
            | "double precision"
            | "boolean"
            | "bytea"
            | "text"
            | "json"
            | "jsonb"
            | "uuid"
            | "inet"
            | "money"
            | "date"
            | "numeric"
            | "character"
            | "varchar"
            | "bit"
            | "varbit"
            | "timestamp"
            | "timestamptz"
            | "time"
            | "timetz"
            | "interval"
            | "geometry"
    ) {
        return None;
    }
    let mut parameters = if let Some(modifier) = modifier {
        modifier
            .split(',')
            .map(|part| {
                let part = part.trim();
                if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                    None
                } else {
                    part.parse::<u32>().ok()
                }
            })
            .collect::<Option<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let valid = match canonical {
        "numeric" => {
            parameters.is_empty()
                || ((1..=2).contains(&parameters.len())
                    && (1..=1000).contains(&parameters[0])
                    && (parameters.len() == 1 || parameters[1] <= parameters[0]))
        }
        "character" | "varchar" | "bit" | "varbit" => {
            parameters.is_empty() || (parameters.len() == 1 && parameters[0] > 0)
        }
        "timestamp" | "timestamptz" | "time" | "timetz" | "interval" => {
            parameters.is_empty() || (parameters.len() == 1 && parameters[0] <= 6)
        }
        _ => parameters.is_empty(),
    };
    if !valid {
        return None;
    }
    if canonical == "numeric" && parameters.len() == 1 {
        parameters.push(0);
    }
    let suffix = if parameters.is_empty() {
        String::new()
    } else {
        format!(
            "({})",
            parameters
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    Some(format!("{canonical}{suffix}{}", "[]".repeat(arrays)))
}

fn qualified_identity(value: &str) -> Option<String> {
    let mut chars = value.chars().peekable();
    let mut parts = Vec::new();
    loop {
        while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
            chars.next();
        }
        let mut part = String::new();
        let quoted = chars.peek() == Some(&'"');
        if quoted {
            chars.next();
            loop {
                match chars.next()? {
                    '"' if chars.peek() == Some(&'"') => {
                        chars.next();
                        part.push('"');
                    }
                    '"' => break,
                    ch => part.push(ch),
                }
            }
        } else {
            while chars
                .peek()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
            {
                part.push(chars.next()?.to_ascii_lowercase());
            }
            if !part.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_') {
                return None;
            }
        }
        if part.is_empty() || part.len() > 63 {
            return None;
        }
        parts.push(format!("\"{}\"", part.replace('"', "\"\"")));
        while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
            chars.next();
        }
        match chars.next() {
            Some('.') if parts.len() == 1 => {}
            None if parts.len() == 2 || quoted => return Some(parts.join(".")),
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_preserve_modifiers_timezones_arrays_and_case_sensitive_identities() {
        for (actual, expected) in [
            ("timestamp(6) with time zone", "timestamptz(6)"),
            ("numeric(65, 30)", "decimal(65,30)"),
            ("character varying(40)", "varchar(40)"),
            ("geometry", "geometry"),
            ("BOOLEAN[]", "bool[]"),
            ("integer[][]", "int4[][]"),
            ("numeric(20)", "numeric(20,0)"),
            ("legacy.state", "\"legacy\".\"state\""),
            ("\"Odd Schema\".\"a\"\"b\"[]", "\"Odd Schema\".\"a\"\"b\"[]"),
        ] {
            assert!(
                equivalent_postgres_types(actual, expected),
                "{actual} vs {expected}"
            );
        }
        for (actual, expected) in [
            ("timestamp(6)", "timestamptz(6)"),
            ("numeric(65,29)", "numeric(65,30)"),
            ("integer[]", "integer[][]"),
            ("\"State\"", "\"state\""),
            ("varchar(4)", "text"),
        ] {
            assert!(!equivalent_postgres_types(actual, expected));
        }
    }
    #[test]
    fn unknown_or_malformed_grammar_fails_closed() {
        for unknown in [
            "state",
            "integer garbage",
            "numeric(0,0)",
            "numeric(20,21)",
            "timestamp(7)",
            "integer(2)",
            "numeric(2,)",
            "public.state; DROP TABLE x",
            "\"unterminated",
            "public..state",
            "a.b.c",
            "integer[2]",
            "time(3) with invalid zone",
            "timestamp with time zone(3) garbage",
            "timestamp with time zone(3)",
        ] {
            assert_eq!(normalize_postgres_type(unknown), None, "{unknown}");
            assert!(!equivalent_postgres_types(unknown, unknown));
        }
    }
}
