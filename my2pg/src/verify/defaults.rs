//! Finite typed default comparison. Expressions are never executed.
use crate::model::{ColumnPlan, ValueKind};
use crate::types::normalize_postgres_type;
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DefaultComparison {
    Equal,
    Different,
    Unsupported,
}
pub(crate) struct DeparseSettings {
    pub exact_float_output: bool,
}
#[derive(Clone, Copy, Default)]
pub(crate) enum IntervalStyle {
    #[default]
    Postgres,
    Verbose,
    Sql,
    Iso,
}
impl IntervalStyle {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "postgres" => Some(Self::Postgres),
            "postgres_verbose" => Some(Self::Verbose),
            "sql_standard" => Some(Self::Sql),
            "iso_8601" => Some(Self::Iso),
            _ => None,
        }
    }
}
pub(crate) struct EnumIdentity {
    pub expected_oid: u32,
    pub column_oid: u32,
    pub cast_oid: Option<u32>,
}
#[derive(Default)]
pub(crate) struct DefaultCatalogContext {
    pub interval_style: IntervalStyle,
    pub target_type_proven: bool,
    pub enum_identity: Option<EnumIdentity>,
}
pub(crate) fn compare(
    column: &ColumnPlan,
    actual: &str,
    settings: &DeparseSettings,
) -> DefaultComparison {
    compare_with_catalog(column, actual, settings, &DefaultCatalogContext::default())
}
pub(crate) fn compare_with_catalog(
    column: &ColumnPlan,
    actual: &str,
    settings: &DeparseSettings,
    context: &DefaultCatalogContext,
) -> DefaultComparison {
    let Some(expected) = column.default_sql.as_deref() else {
        return DefaultComparison::Unsupported;
    };
    let expected = typed(column, expected, settings, context, true);
    if let Some(Literal::Array(values)) = &expected
        && values.iter().any(|value| {
            value
                .as_ref()
                .is_none_or(|value| !column.set_labels.contains(value))
        })
    {
        return DefaultComparison::Unsupported;
    }
    match (expected, typed(column, actual, settings, context, false)) {
        (Some(_), Some(_))
            if context
                .enum_identity
                .as_ref()
                .is_some_and(|proof| proof.column_oid != proof.expected_oid) =>
        {
            DefaultComparison::Different
        }
        (Some(expected), Some(actual)) if expected == actual => DefaultComparison::Equal,
        (Some(_), Some(_)) => DefaultComparison::Different,
        _ => DefaultComparison::Unsupported,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Literal {
    Text(String),
    Number(String),
    Boolean(bool),
    Float(u64),
    Calendar(u32, u32, u32, u64),
    Duration(i64),
    Bytes(Vec<u8>),
    Array(Vec<Option<String>>),
    CurrentTimestamp(u8),
    Enum(String, Option<u32>),
    Json(Json),
    Network(IpAddr, u8),
}

fn current_precision(precision: u8, target: &str, context: &DefaultCatalogContext) -> u8 {
    let target = target
        .split_once('(')
        .and_then(|(_, s)| s.strip_suffix(')'))
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(6);
    if context.target_type_proven && (precision == 6 || precision == target) {
        target
    } else {
        precision
    }
}

// Only a complete SQL identifier or schema-qualified identifier is resolvable.
pub(crate) fn type_identifier(value: &str) -> Option<String> {
    let value = value.trim();
    let mut rest = value;
    for part in 0..2 {
        rest = rest.trim_start();
        if let Some(quoted) = rest.strip_prefix('"') {
            let mut chars = quoted.char_indices().peekable();
            let mut length = 0;
            let end = loop {
                let (index, ch) = chars.next()?;
                if ch == '\0' {
                    return None;
                }
                if ch == '"' {
                    if chars.peek().is_some_and(|(_, ch)| *ch == '"') {
                        chars.next();
                    } else {
                        break index + 2;
                    }
                }
                length += 1;
            };
            if length == 0 {
                return None;
            }
            rest = &rest[end..];
        } else {
            if !rest.starts_with(|ch: char| ch.is_alphabetic() || ch == '_') {
                return None;
            }
            let end = rest
                .find(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '$')))
                .unwrap_or(rest.len());
            rest = &rest[end..];
        }
        rest = rest.trim_start();
        if rest.is_empty() {
            return Some(value.to_owned());
        }
        if part == 1 {
            return None;
        }
        rest = rest.strip_prefix('.')?;
    }
    None
}
pub(crate) fn enum_cast(expression: &str) -> Option<String> {
    let expression = unwrapped(expression)?;
    let (_, tail) = quoted_literal(expression)?;
    type_identifier(tail.trim().strip_prefix("::")?.trim())
}

fn network(value: &str) -> Option<(IpAddr, u8)> {
    let (address, prefix) = value
        .split_once('/')
        .map_or((value, None), |(a, p)| (a, Some(p)));
    let address = address.parse::<IpAddr>().ok().or_else(|| {
        if let Some((prefix, dotted)) = address.rsplit_once(':') {
            format!("{prefix}:{}", ipv4(dotted)?).parse::<IpAddr>().ok()
        } else {
            ipv4(address).map(IpAddr::V4)
        }
    })?;
    let maximum = if address.is_ipv4() { 32 } else { 128 };
    let prefix = if let Some(prefix) = prefix {
        if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        prefix.parse::<u8>().ok()?
    } else {
        maximum
    };
    (prefix <= maximum).then_some((address, prefix))
}
fn ipv4(value: &str) -> Option<Ipv4Addr> {
    let mut octets = [0; 4];
    let mut fields = value.split('.');
    for octet in &mut octets {
        let value = fields.next()?;
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *octet = value.parse::<u8>().ok()?;
    }
    fields.next().is_none().then_some(Ipv4Addr::from(octets))
}

fn binary(value: &str) -> Option<Vec<u8>> {
    if value.starts_with("\\x") {
        return hex(value);
    }
    let bytes = value.as_bytes();
    let mut output = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            output.push(bytes[index]);
            index += 1;
        } else if bytes.get(index + 1) == Some(&b'\\') {
            output.push(b'\\');
            index += 2;
        } else {
            let octal = bytes.get(index + 1..index + 4)?;
            if !octal.iter().all(|b| matches!(b, b'0'..=b'7')) {
                return None;
            }
            output.push(u8::from_str_radix(std::str::from_utf8(octal).ok()?, 8).ok()?);
            index += 4;
        }
    }
    Some(output)
}

fn interval(value: &str, style: IntervalStyle) -> Option<i64> {
    if let Some(clock) = clock(value, true) {
        return Some(clock);
    }
    if value == "0" || value == "@ 0" {
        return Some(0);
    }
    match style {
        IntervalStyle::Verbose => {
            let value = value.strip_prefix("@ ")?;
            let (value, negative) = value
                .strip_suffix(" ago")
                .map_or((value, false), |v| (v, true));
            let tokens = value.split_whitespace().collect::<Vec<_>>();
            if !tokens.len().is_multiple_of(2) {
                return None;
            }
            let mut total = 0i64;
            let mut previous = 0;
            for pair in tokens.as_chunks::<2>().0 {
                let (rank, multiplier) = match pair[1] {
                    "hour" | "hours" => (1, 3_600_000_000),
                    "min" | "mins" => (2, 60_000_000),
                    "sec" | "secs" => (3, 1_000_000),
                    _ => return None,
                };
                if rank <= previous {
                    return None;
                }
                previous = rank;
                total = total.checked_add(interval_component(pair[0], multiplier)?)?;
            }
            if negative {
                total.checked_neg()
            } else {
                Some(total)
            }
        }
        IntervalStyle::Iso => {
            let mut rest = value.strip_prefix("PT")?;
            let mut total = 0i64;
            let mut previous = 0;
            while !rest.is_empty() {
                let end = rest.find(['H', 'M', 'S'])?;
                let (rank, multiplier) = match rest.as_bytes()[end] {
                    b'H' => (1, 3_600_000_000),
                    b'M' => (2, 60_000_000),
                    b'S' => (3, 1_000_000),
                    _ => return None,
                };
                if rank <= previous {
                    return None;
                }
                previous = rank;
                total = total.checked_add(interval_component(&rest[..end], multiplier)?)?;
                rest = &rest[end + 1..];
            }
            (previous != 0).then_some(total)
        }
        _ => None,
    }
}
fn interval_component(value: &str, multiplier: i64) -> Option<i64> {
    let negative = value.starts_with('-');
    let value = value.strip_prefix(['-', '+']).unwrap_or(value);
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || (multiplier != 1_000_000 && !fraction.is_empty())
    {
        return None;
    }
    let whole = whole.parse::<i64>().ok()?.checked_mul(multiplier)?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<i64>().ok()? * 10i64.pow(6 - fraction.len() as u32)
    };
    let total = whole.checked_add(fraction)?;
    if negative {
        total.checked_neg()
    } else {
        Some(total)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Json {
    Null,
    Bool(bool),
    String(String),
    Number(String, i64),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}
fn json(value: &str) -> Option<Json> {
    let mut reader = JsonReader {
        text: value,
        position: 0,
    };
    let value = reader.value(0)?;
    reader.space();
    (reader.position == reader.text.len()).then_some(value)
}
struct JsonReader<'a> {
    text: &'a str,
    position: usize,
}
impl JsonReader<'_> {
    fn space(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.position)
            .is_some_and(|b| matches!(b, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.position += 1;
        }
    }
    fn byte(&mut self, byte: u8) -> Option<()> {
        self.space();
        if self.text.as_bytes().get(self.position) != Some(&byte) {
            return None;
        }
        self.position += 1;
        Some(())
    }
    fn string(&mut self) -> Option<String> {
        self.space();
        let start = self.position;
        self.byte(b'"')?;
        loop {
            match *self.text.as_bytes().get(self.position)? {
                b'"' => {
                    self.position += 1;
                    break;
                }
                b'\\' => self.position += 2,
                _ => self.position += 1,
            }
        }
        let text = serde_json::from_str::<String>(&self.text[start..self.position]).ok()?;
        (!text.contains('\0')).then_some(text)
    }
    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth >= 128 {
            return None;
        }
        self.space();
        let value =
            match *self.text.as_bytes().get(self.position)? {
                b'"' => Json::String(self.string()?),
                b'n' | b't' | b'f' => {
                    let rest = &self.text[self.position..];
                    if rest.starts_with("null") {
                        self.position += 4;
                        Json::Null
                    } else if rest.starts_with("true") {
                        self.position += 4;
                        Json::Bool(true)
                    } else if rest.starts_with("false") {
                        self.position += 5;
                        Json::Bool(false)
                    } else {
                        return None;
                    }
                }
                b'[' => {
                    self.position += 1;
                    self.space();
                    let mut values = Vec::new();
                    if self.text.as_bytes().get(self.position) == Some(&b']') {
                        self.position += 1;
                        return Some(Json::Array(values));
                    }
                    loop {
                        values.push(self.value(depth + 1)?);
                        self.space();
                        if self.text.as_bytes().get(self.position) == Some(&b']') {
                            self.position += 1;
                            break;
                        }
                        self.byte(b',')?;
                    }
                    Json::Array(values)
                }
                b'{' => {
                    self.position += 1;
                    self.space();
                    let mut values = BTreeMap::new();
                    if self.text.as_bytes().get(self.position) == Some(&b'}') {
                        self.position += 1;
                        return Some(Json::Object(values));
                    }
                    loop {
                        let key = self.string()?;
                        self.byte(b':')?;
                        values.insert(key, self.value(depth + 1)?);
                        self.space();
                        if self.text.as_bytes().get(self.position) == Some(&b'}') {
                            self.position += 1;
                            break;
                        }
                        self.byte(b',')?;
                    }
                    Json::Object(values)
                }
                b'-' | b'0'..=b'9' => {
                    let start = self.position;
                    while self.text.as_bytes().get(self.position).is_some_and(|b| {
                        matches!(b, b'-' | b'+' | b'0'..=b'9' | b'.' | b'e' | b'E')
                    }) {
                        self.position += 1;
                    }
                    let token = &self.text[start..self.position];
                    // Library syntax validation accepts only a JSON numeric token.
                    token.parse::<serde_json::Number>().ok()?;
                    let (coefficient, exponent) = json_number(token)?;
                    Json::Number(coefficient, exponent)
                }
                _ => return None,
            };
        Some(value)
    }
}
fn json_number(value: &str) -> Option<(String, i64)> {
    let negative = value.starts_with('-');
    let value = value.strip_prefix('-').unwrap_or(value);
    let (mantissa, exponent) = value.split_once(['e', 'E']).unwrap_or((value, "0"));
    let exponent = exponent.parse::<i64>().ok()?;
    let (integral, fractional) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{integral}{fractional}");
    let leading = digits.len() - digits.trim_start_matches('0').len();
    if (integral.len() as i64)
        .checked_add(exponent)?
        .checked_sub(leading as i64)?
        > 131072
        || (fractional.len() as i64).checked_sub(exponent)? > 16383
    {
        return None;
    }
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Some(("0".into(), 0));
    }
    let trailing = digits.len() - digits.trim_end_matches('0').len();
    let exponent = exponent
        .checked_sub(fractional.len() as i64)?
        .checked_add(trailing as i64)?;
    Some((
        format!(
            "{}{}",
            if negative { "-" } else { "" },
            digits.trim_end_matches('0')
        ),
        exponent,
    ))
}

// Strip only complete balanced wrappers. Quoted data never changes the depth.
fn unwrapped(mut expression: &str) -> Option<&str> {
    expression = expression.trim();
    for _ in 0..32 {
        if !expression.starts_with('(') {
            return Some(expression);
        }
        let mut depth = 0u32;
        let mut position = 0;
        let mut end = None;
        while position < expression.len() {
            let rest = &expression[position..];
            if rest.starts_with('\'') || rest.starts_with("E'") || rest.starts_with("e'") {
                let (_, tail) = quoted_literal(rest)?;
                position = expression.len() - tail.len();
                continue;
            }
            let ch = rest.chars().next()?;
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        end = Some(position);
                        break;
                    }
                }
                _ => (),
            }
            position += ch.len_utf8();
        }
        if end != Some(expression.len() - 1) {
            return Some(expression);
        }
        expression = expression[1..expression.len() - 1].trim();
    }
    None
}

/// Complete SQL single-quoted token, including the finite escapes our planner emits.
pub(super) fn quoted_literal(expression: &str) -> Option<(String, &str)> {
    let (escape, expression) = if let Some(value) = expression
        .strip_prefix("E'")
        .or_else(|| expression.strip_prefix("e'"))
    {
        (true, value)
    } else {
        (false, expression.strip_prefix('\'')?)
    };
    let mut chars = expression.char_indices().peekable();
    let mut value = String::new();
    while let Some((position, ch)) = chars.next() {
        if ch == '\'' {
            if chars.peek().is_some_and(|(_, next)| *next == '\'') {
                chars.next();
                value.push('\'');
            } else {
                return Some((value, &expression[position + 1..]));
            }
        } else if escape && ch == '\\' {
            value.push(match chars.next()?.1 {
                '\\' => '\\',
                '\'' => '\'',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                'b' => '\u{0008}',
                'f' => '\u{000c}',
                _ => return None,
            });
        } else {
            value.push(ch);
        }
    }
    None
}

fn typed(
    column: &ColumnPlan,
    expression: &str,
    settings: &DeparseSettings,
    context: &DefaultCatalogContext,
    expected: bool,
) -> Option<Literal> {
    let target = normalize_postgres_type(&column.target_type)?;
    let base = if column.kind == ValueKind::Enum {
        target.as_str()
    } else {
        target.split('(').next()?
    };
    let expression = unwrapped(expression)?;
    if (column.kind == ValueKind::Datetime && base == "timestamp")
        || (column.kind == ValueKind::Timestamp && base == "timestamptz")
    {
        let upper = expression.to_ascii_uppercase();
        if upper == "CURRENT_TIMESTAMP" {
            return Some(Literal::CurrentTimestamp(current_precision(
                6, &target, context,
            )));
        }
        if let Some(precision) = upper
            .strip_prefix("CURRENT_TIMESTAMP(")
            .and_then(|s| s.strip_suffix(')'))
            && precision.len() == 1
            && matches!(precision.as_bytes()[0], b'0'..=b'6')
        {
            return Some(Literal::CurrentTimestamp(current_precision(
                precision.as_bytes()[0] - b'0',
                &target,
                context,
            )));
        }
    }
    let bit_token = expression.starts_with("B'") || expression.starts_with("b'");
    let quoted = bit_token
        || expression.starts_with('\'')
        || expression.starts_with("E'")
        || expression.starts_with("e'");
    let (value, tail) = if quoted {
        quoted_literal(if bit_token {
            &expression[1..]
        } else {
            expression
        })?
    } else {
        let (value, tail) = expression
            .split_once("::")
            .map_or((expression, ""), |(value, _)| {
                (value, &expression[value.len()..])
            });
        (unwrapped(value)?.to_owned(), tail)
    };
    let tail = tail.trim();
    let mut numeric_float = false;
    if !tail.is_empty() {
        let cast = tail.strip_prefix("::")?.trim();
        let cast = if base.starts_with("bit") && cast == "\"bit\"" {
            "bit"
        } else {
            cast
        };
        if column.kind == ValueKind::Enum && !expected && context.enum_identity.is_some() {
            type_identifier(cast)?;
            context.enum_identity.as_ref()?.cast_oid?;
        } else {
            let cast = normalize_postgres_type(cast.strip_prefix("pg_catalog.").unwrap_or(cast))?;
            numeric_float = column.kind == ValueKind::Float
                && matches!(base, "real" | "double precision")
                && cast == "numeric";
            if cast != target && cast != base && !numeric_float {
                return None;
            }
        }
    }
    if bit_token && !matches!(column.kind, ValueKind::Bit(_)) {
        return None;
    }
    match &column.kind {
        ValueKind::Text if quoted && matches!(base, "text" | "varchar" | "character") => {
            Some(Literal::Text(value))
        }
        ValueKind::Enum
            if quoted && target.contains('.') && column.enum_labels.contains(&value) =>
        {
            Some(Literal::Enum(
                value,
                context.enum_identity.as_ref().map(|proof| {
                    if expected {
                        proof.expected_oid
                    } else {
                        proof.cast_oid.unwrap_or(proof.column_oid)
                    }
                }),
            ))
        }
        ValueKind::Json if quoted && base == "jsonb" => json(&value).map(Literal::Json),
        ValueKind::Json if quoted && base == "json" => {
            serde_json::from_str::<serde::de::IgnoredAny>(&value).ok()?;
            Some(Literal::Text(value))
        }
        ValueKind::Text if quoted && base == "inet" => {
            network(&value).map(|(address, prefix)| Literal::Network(address, prefix))
        }
        ValueKind::Boolean if base == "boolean" => match value.to_ascii_lowercase().as_str() {
            "true" => Some(Literal::Boolean(true)),
            "false" => Some(Literal::Boolean(false)),
            _ => None,
        },
        ValueKind::SignedInteger | ValueKind::UnsignedInteger | ValueKind::Decimal
            if matches!(base, "smallint" | "integer" | "bigint" | "numeric") =>
        {
            number(&value, base == "numeric").map(Literal::Number)
        }
        ValueKind::Float
            if settings.exact_float_output && matches!(base, "real" | "double precision") =>
        {
            if !float_grammar(&value) {
                return None;
            }
            let zero = zero_mantissa(&value);
            // Numeric has no signed zero; its implicit conversion preserves +0.
            if numeric_float && zero {
                return Some(Literal::Float(0));
            }
            let bits = if base == "real" {
                let value: f32 = value.parse().ok()?;
                if !value.is_finite() || (value == 0.0 && !zero) {
                    return None;
                }
                u64::from(value.to_bits())
            } else {
                let value: f64 = value.parse().ok()?;
                if !value.is_finite() || (value == 0.0 && !zero) {
                    return None;
                }
                value.to_bits()
            };
            Some(Literal::Float(bits))
        }
        ValueKind::Date if quoted && base == "date" => {
            calendar(&value, false, false).map(|(y, m, d, t)| Literal::Calendar(y, m, d, t))
        }
        ValueKind::Datetime if quoted && base == "timestamp" => calendar(&value, true, false)
            .filter(|(_, _, _, t)| precision(&target, *t))
            .map(|(y, m, d, t)| Literal::Calendar(y, m, d, t)),
        ValueKind::Timestamp if quoted && base == "timestamptz" => calendar(&value, true, true)
            .filter(|(_, _, _, t)| precision(&target, *t))
            .map(|(y, m, d, t)| Literal::Calendar(y, m, d, t)),
        ValueKind::Interval if quoted && matches!(base, "interval" | "time") => {
            let time = if expected {
                clock(&value, true)?
            } else {
                interval(&value, context.interval_style)?
            };
            if base == "time" && !(0..86_400_000_000).contains(&time) {
                return None;
            }
            precision(&target, time.unsigned_abs()).then_some(Literal::Duration(time))
        }
        ValueKind::Binary if quoted && base == "bytea" => binary(&value).map(Literal::Bytes),
        ValueKind::Bit(width)
            if quoted
                && base == "bit"
                && value.len() == usize::from(*width)
                && value.bytes().all(|b| matches!(b, b'0' | b'1')) =>
        {
            Some(Literal::Text(value))
        }
        ValueKind::Set if quoted && target == "text[]" => array(&value).map(Literal::Array),
        _ => None,
    }
}

fn number(value: &str, decimal: bool) -> Option<String> {
    let negative = value.starts_with('-');
    let value = value.strip_prefix(['-', '+']).unwrap_or(value);
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || (!fraction.is_empty() && (!decimal || !fraction.bytes().all(|b| b.is_ascii_digit())))
        || (value.contains('.') && fraction.is_empty())
    {
        return None;
    }
    let whole = whole.trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = fraction.trim_end_matches('0');
    let sign = if negative && (whole != "0" || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    Some(format!(
        "{sign}{whole}{}",
        if fraction.is_empty() {
            String::new()
        } else {
            format!(".{fraction}")
        }
    ))
}

fn float_grammar(value: &str) -> bool {
    let (mantissa, exponent) = value
        .split_once(['e', 'E'])
        .map_or((value, None), |(m, e)| (m, Some(e)));
    let mantissa = mantissa.strip_prefix(['-', '+']).unwrap_or(mantissa);
    let mut digits = 0;
    let mut points = 0;
    for byte in mantissa.bytes() {
        if byte.is_ascii_digit() {
            digits += 1;
        } else if byte == b'.' {
            points += 1;
        } else {
            return false;
        }
    }
    digits > 0
        && points <= 1
        && exponent.is_none_or(|e| {
            let e = e.strip_prefix(['-', '+']).unwrap_or(e);
            !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit())
        })
}
fn zero_mantissa(value: &str) -> bool {
    value.split(['e', 'E']).next().is_some_and(|value| {
        value
            .bytes()
            .filter(u8::is_ascii_digit)
            .all(|byte| byte == b'0')
    })
}

fn precision(target: &str, micros: u64) -> bool {
    let digits = target
        .split_once('(')
        .and_then(|(_, s)| s.strip_suffix(')'))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(6);
    digits <= 6 && micros.is_multiple_of(10u64.pow(6 - digits))
}
fn clock(value: &str, signed: bool) -> Option<i64> {
    let negative = signed && value.starts_with('-');
    let value = if negative { &value[1..] } else { value };
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 6 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let parts = whole
        .split(':')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                part.parse::<u64>().ok()
            }
        })
        .collect::<Option<Vec<_>>>()?;
    if parts.len() != 3 || parts[1] > 59 || parts[2] > 59 {
        return None;
    }
    let micros = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().ok()? * 10u64.pow(6 - fraction.len() as u32)
    };
    let total = parts[0]
        .checked_mul(3600)?
        .checked_add(parts[1] * 60)?
        .checked_add(parts[2])?
        .checked_mul(1_000_000)?
        .checked_add(micros)?;
    let total = i64::try_from(total).ok()?;
    Some(if negative { -total } else { total })
}
fn calendar(value: &str, with_time: bool, utc: bool) -> Option<(u32, u32, u32, u64)> {
    let value = if utc {
        value
            .strip_suffix("+00:00")
            .or_else(|| value.strip_suffix("+00"))
            .or_else(|| value.strip_suffix('Z'))?
    } else {
        value
    };
    let (date, time) = if with_time {
        value.split_once(' ')?
    } else {
        (value, "")
    };
    if date.len() != 10 || date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
        return None;
    }
    let components = date
        .split('-')
        .map(|s| {
            if s.bytes().all(|b| b.is_ascii_digit()) {
                s.parse::<u32>().ok()
            } else {
                None
            }
        })
        .collect::<Option<Vec<_>>>()?;
    let [year, month, day] = components.as_slice() else {
        return None;
    };
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let maximum = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if !(1..=9999).contains(year) || *day == 0 || *day > maximum {
        return None;
    }
    let micros = if with_time {
        u64::try_from(clock(time, false)?).ok()?
    } else {
        0
    };
    if micros >= 86_400_000_000 {
        return None;
    }
    Some((*year, *month, *day, micros))
}
fn hex(value: &str) -> Option<Vec<u8>> {
    let value = value.strip_prefix("\\x")?;
    if !value.len().is_multiple_of(2) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok())
        .collect()
}
fn array(value: &str) -> Option<Vec<Option<String>>> {
    let value = value.strip_prefix('{')?.strip_suffix('}')?;
    if value.is_empty() {
        return Some(vec![]);
    }
    let mut chars = value.chars().peekable();
    let mut values = Vec::new();
    loop {
        let quoted = chars.peek() == Some(&'"');
        let mut element = String::new();
        if quoted {
            chars.next();
            loop {
                match chars.next()? {
                    '"' => break,
                    '\\' => element.push(chars.next()?),
                    ch => element.push(ch),
                }
            }
        } else {
            while let Some(&ch) = chars.peek() {
                if ch == ',' {
                    break;
                }
                if matches!(ch, '{' | '}' | '"' | '\\') || ch.is_whitespace() {
                    return None;
                }
                element.push(chars.next()?);
            }
            if element.is_empty() {
                return None;
            }
        }
        values.push(if !quoted && element.eq_ignore_ascii_case("NULL") {
            None
        } else {
            Some(element)
        });
        match chars.next() {
            None => return Some(values),
            Some(',') if chars.peek().is_some() => (),
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn column(kind: ValueKind, target: &str, expected: &str) -> ColumnPlan {
        ColumnPlan {
            source_name: "value".into(),
            target_name: "value".into(),
            source_type: "".into(),
            target_type: target.into(),
            kind,
            nullable: true,
            default_sql: Some(expected.into()),
            identity: false,
            generated_expression: None,
            copy: true,
            transform: None,
            charset: None,
            enum_labels: vec!["".into(), "a,b".into(), "quote'label".into()],
            set_labels: vec![
                "a,b".into(),
                "NULL".into(),
                "".into(),
                "slash\\label".into(),
                "quote\"label".into(),
            ],
            comment: None,
        }
    }
    #[test]
    fn core_default_spellings_preserve_typed_values() {
        let settings = DeparseSettings {
            exact_float_output: true,
        };
        for (kind, target, expected, actual) in [
            (
                ValueKind::Text,
                "varchar(20)",
                "E''",
                "''::character varying",
            ),
            (ValueKind::Text, "text", r"E'a\\b'", r"'a\b'::text"),
            (ValueKind::Decimal, "numeric(65,30)", "+0001.500", "1.5"),
            (ValueKind::SignedInteger, "integer", "-3", "'-3'::integer"),
            (ValueKind::Boolean, "boolean", "FALSE", "false"),
            (ValueKind::Float, "real", "0.1", "'0.1'::real"),
            (
                ValueKind::Float,
                "double precision",
                "1.5e2",
                "'150'::double precision",
            ),
            (
                ValueKind::Date,
                "date",
                "E'2024-02-29'",
                "'2024-02-29'::date",
            ),
            (
                ValueKind::Datetime,
                "timestamp(6)",
                "E'2024-02-29 01:02:03.000000'",
                "'2024-02-29 01:02:03'::timestamp without time zone",
            ),
            (
                ValueKind::Timestamp,
                "timestamptz(6)",
                "E'2024-02-29 01:02:03.123456+00'",
                "'2024-02-29 01:02:03.123456+00'::timestamp with time zone",
            ),
            (
                ValueKind::Interval,
                "interval(6)",
                "E'-838:59:59.000000'",
                "'-838:59:59'::interval(6)",
            ),
            (
                ValueKind::Binary,
                "bytea",
                r"E'\\x61000000'",
                r"'\x61000000'::bytea",
            ),
            (
                ValueKind::Bit(8),
                "bit(8)",
                "B'00000101'",
                "'00000101'::\"bit\"",
            ),
            (
                ValueKind::Enum,
                "\"chosen\".\"State\"",
                "E'a,b'",
                "'a,b'::chosen.\"State\"",
            ),
            (
                ValueKind::Set,
                "text[]",
                r#"E'{"a,b","NULL",""}'"#,
                r#"'{"a,b","NULL",""}'::text[]"#,
            ),
            (
                ValueKind::Timestamp,
                "timestamptz(6)",
                "CURRENT_TIMESTAMP(6)",
                "CURRENT_TIMESTAMP(6)",
            ),
        ] {
            assert_eq!(
                compare(&column(kind, target, expected), actual, &settings),
                DefaultComparison::Equal,
                "{expected} / {actual}"
            );
        }
    }
    #[test]
    fn mismatches_and_unknown_expressions_do_not_pass() {
        let settings = DeparseSettings {
            exact_float_output: true,
        };
        for (kind, target, expected, actual) in [
            (ValueKind::Binary, "bytea", r"E'\\x00'", r"'\xff'::bytea"),
            (
                ValueKind::Set,
                "text[]",
                r#"E'{"NULL"}'"#,
                "'{NULL}'::text[]",
            ),
            (
                ValueKind::Float,
                "double precision",
                "-0.0",
                "'0'::double precision",
            ),
            (
                ValueKind::Timestamp,
                "timestamptz(6)",
                "CURRENT_TIMESTAMP(6)",
                "CURRENT_TIMESTAMP(0)",
            ),
            (
                ValueKind::Float,
                "double precision",
                "-0.0",
                "'-0.0'::numeric",
            ),
        ] {
            assert_eq!(
                compare(&column(kind, target, expected), actual, &settings),
                DefaultComparison::Different
            );
        }
        for expression in [
            "random()",
            "now()",
            "1+2",
            "'1'::text::integer",
            "'abc'::varchar(2)",
            "E'\\x41'",
            "'unclosed",
            "('a') || ('b')",
            "'a'; SELECT 1",
        ] {
            assert_eq!(
                compare(
                    &column(ValueKind::Text, "varchar(20)", expression),
                    expression,
                    &settings
                ),
                DefaultComparison::Unsupported,
                "{expression}"
            );
        }
        assert_eq!(
            compare(
                &column(ValueKind::Float, "real", "0.1"),
                "'0.1'::real",
                &DeparseSettings {
                    exact_float_output: false
                }
            ),
            DefaultComparison::Unsupported
        );
        assert_eq!(
            compare(
                &column(ValueKind::Json, "jsonb", "E'{}'"),
                "'{}'::jsonb",
                &settings
            ),
            DefaultComparison::Equal
        );
    }
    #[test]
    fn lexical_and_domain_boundaries_remain_explicit() {
        let settings = DeparseSettings {
            exact_float_output: true,
        };
        for (kind, target, expected, actual) in [
            (
                ValueKind::Text,
                "text",
                "E'it''s::ok'",
                "(('it''s::ok'::text))",
            ),
            (ValueKind::Decimal, "numeric(65,30)", "-000.000", "0"),
            (
                ValueKind::Float,
                "real",
                "1.17549435e-38",
                "'1.1754944e-38'::real",
            ),
            (
                ValueKind::Binary,
                "bytea",
                r"E'\\x00FF5c00'",
                r"'\x00ff5c00'::bytea",
            ),
            (
                ValueKind::Set,
                "text[]",
                r#"E'{"slash\\\\label","quote\\"label"}'"#,
                r#"'{"slash\\label","quote\"label"}'::text[]"#,
            ),
            (
                ValueKind::Timestamp,
                "timestamptz(6)",
                "E'2024-02-29 01:02:03.000000+00'",
                "'2024-02-29 01:02:03+00:00'::timestamptz",
            ),
        ] {
            assert_eq!(
                compare(&column(kind, target, expected), actual, &settings),
                DefaultComparison::Equal,
                "{expected} / {actual}"
            );
        }
        for (kind, target, expected, actual) in [
            (
                ValueKind::Date,
                "date",
                "E'2024-02-29'",
                "'2023-02-29'::date",
            ),
            (
                ValueKind::Timestamp,
                "timestamptz(6)",
                "E'2024-02-29 01:02:03+00'",
                "'2024-02-29 01:02:03+01'::timestamptz",
            ),
            (
                ValueKind::Datetime,
                "timestamp(3)",
                "E'2024-02-29 01:02:03.000000'",
                "'2024-02-29 01:02:03.123456'::timestamp",
            ),
            (
                ValueKind::Interval,
                "interval(6)",
                "E'00:00:00.000000'",
                "'1 month'::interval",
            ),
            (ValueKind::Bit(8), "bit(8)", "B'00000101'", "'101'::bit(3)"),
            (ValueKind::Enum, "chosen.state", "E'a,b'", "'a,b'::state"),
            (ValueKind::Set, "text[]", "E'{}'", "'[0:0]={alpha}'::text[]"),
            (ValueKind::Float, "real", "0.1", "'NaN'::real"),
            (ValueKind::Float, "real", "0.0", "'1e-10000'::numeric"),
            (ValueKind::Float, "real", "0.1", "'0.1'::numeric(1,0)"),
        ] {
            assert_eq!(
                compare(&column(kind, target, expected), actual, &settings),
                DefaultComparison::Unsupported,
                "{actual}"
            );
        }
    }
    #[test]
    fn exact_jsonb_semantics_do_not_collapse_objects_or_arrays() {
        let settings = DeparseSettings {
            exact_float_output: true,
        };
        for (expected, actual) in [
            (
                r#"E'{"b":1e3,"a":[1.00,null,true,"x"],"b":1000.0}'"#,
                r#"'{"a":[1,null,true,"x"],"b":1000}'::jsonb"#,
            ),
            (
                r#"E'{"$serde_json::private::Number":"1"}'"#,
                r#"'{"$serde_json::private::Number":"1"}'::jsonb"#,
            ),
            ("E'1e131071'", "'1e131071'::jsonb"),
            ("E'-0.000'", "'0'::jsonb"),
        ] {
            assert_eq!(
                compare(
                    &column(ValueKind::Json, "jsonb", expected),
                    actual,
                    &settings
                ),
                DefaultComparison::Equal
            );
        }
        for (expected, actual) in [
            (r#"E'{"$serde_json::private::Number":"1"}'"#, "'1'::jsonb"),
            (r#"E'{"a":null}'"#, "'{}'::jsonb"),
            ("E'[1,1,2]'", "'[1,2]'::jsonb"),
            ("E'[1,2]'", "'[2,1]'::jsonb"),
            ("E'18446744073709551615'", "'18446744073709551614'::jsonb"),
        ] {
            assert_eq!(
                compare(
                    &column(ValueKind::Json, "jsonb", expected),
                    actual,
                    &settings
                ),
                DefaultComparison::Different
            );
        }
        assert_eq!(
            compare(
                &column(ValueKind::Json, "json", r#"E'{"a":1,"a":2}'"#),
                r#"'{"a":2}'::json"#,
                &settings
            ),
            DefaultComparison::Different
        );
        for value in ["01", "1e131072", "1e-16384", "[1,]", "null trailing"] {
            assert!(json(value).is_none(), "{value}");
        }
        for value in [r#""\u0000""#, r#""\ud800""#, r#"{"\u0000":1}"#] {
            assert!(json(value).is_none(), "{value}");
        }
        // Match the existing source JSON decoder's 127-container domain.
        let deepest = format!("{}0{}", "[".repeat(127), "]".repeat(127));
        assert!(serde_json::from_str::<serde_json::Value>(&deepest).is_ok());
        assert!(json(&deepest).is_some());
        let too_deep = format!("[{deepest}]");
        assert!(serde_json::from_str::<serde_json::Value>(&too_deep).is_err());
        assert!(json(&too_deep).is_none());
    }
    #[test]
    fn catalog_proofs_and_native_output_styles_are_concrete() {
        let settings = DeparseSettings {
            exact_float_output: true,
        };
        let context = DefaultCatalogContext {
            target_type_proven: true,
            enum_identity: Some(EnumIdentity {
                expected_oid: 42,
                column_oid: 42,
                cast_oid: Some(42),
            }),
            ..Default::default()
        };
        assert_eq!(
            compare_with_catalog(
                &column(ValueKind::Enum, "chosen.state", "E'a,b'"),
                "'a,b'::state",
                &settings,
                &context
            ),
            DefaultComparison::Equal
        );
        let wrong = DefaultCatalogContext {
            enum_identity: Some(EnumIdentity {
                expected_oid: 42,
                column_oid: 99,
                cast_oid: Some(99),
            }),
            ..Default::default()
        };
        assert_eq!(
            compare_with_catalog(
                &column(ValueKind::Enum, "chosen.state", "E'a,b'"),
                "'a,b'::state",
                &settings,
                &wrong
            ),
            DefaultComparison::Different
        );
        for (style, actual) in [
            (
                IntervalStyle::Verbose,
                "'@ 838 hours 59 mins 58.123456 secs ago'::interval(6)",
            ),
            (IntervalStyle::Iso, "'PT-838H-59M-58.123456S'::interval(6)"),
            (IntervalStyle::Sql, "'-838:59:58.123456'::interval(6)"),
        ] {
            assert_eq!(
                compare_with_catalog(
                    &column(ValueKind::Interval, "interval(6)", "E'-838:59:58.123456'"),
                    actual,
                    &settings,
                    &DefaultCatalogContext {
                        interval_style: style,
                        ..Default::default()
                    }
                ),
                DefaultComparison::Equal
            );
        }
        assert_eq!(
            compare(
                &column(ValueKind::Binary, "bytea", r"E'\\x00095c7fff'"),
                r"'\000\011\\\177\377'::bytea",
                &settings
            ),
            DefaultComparison::Equal
        );
        assert_eq!(
            compare(
                &column(ValueKind::Text, "inet", "E'2001:0DB8:0:0:0:0:0:1/64'"),
                "'2001:db8::1/64'::inet",
                &settings
            ),
            DefaultComparison::Equal
        );
        assert_eq!(
            compare_with_catalog(
                &column(ValueKind::Timestamp, "timestamptz(0)", "CURRENT_TIMESTAMP"),
                "CURRENT_TIMESTAMP(0)",
                &settings,
                &context
            ),
            DefaultComparison::Equal
        );
        assert_eq!(
            compare_with_catalog(
                &column(ValueKind::Timestamp, "timestamptz(0)", "CURRENT_TIMESTAMP"),
                "CURRENT_TIMESTAMP(3)",
                &settings,
                &context
            ),
            DefaultComparison::Different
        );
        for value in ["'a,b'::state;SELECT 1", "'a,b'::f()", "'a,b'::state::text"] {
            assert!(enum_cast(value).is_none());
        }
    }
}
