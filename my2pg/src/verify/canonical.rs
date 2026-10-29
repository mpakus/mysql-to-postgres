//! Canonical row encodings for the narrowly supported cross-engine type subset.
//!
//! PostgreSQL values come from explicit text/hex projections. This
//! module never treats driver formatting as a conversion policy.

use crate::{
    convert,
    model::{ColumnPlan, RawValue, ValueKind},
};

#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    #[error("canonical row width differs from the projected column count")]
    RowWidth,
    #[error("content canonicalization does not support {0:?}")]
    UnsupportedKind(ValueKind),
    #[error("source value could not be represented under the resolved conversion")]
    SourceConversion(#[source] convert::ConversionError),
    #[error("typed value text is malformed for {0:?}")]
    InvalidText(ValueKind),
}

/// Encode a source row using the migration's resolved conversion rules.
///
/// `columns` and `values` must be in the exact source projection order. Supported values
/// are integers, exact decimals, text, booleans, binary, dates and microsecond timestamps.
pub fn mysql_row(columns: &[ColumnPlan], values: &[RawValue]) -> Result<Vec<u8>, CanonicalError> {
    if columns.len() != values.len() {
        return Err(CanonicalError::RowWidth);
    }
    let mut row = Vec::new();
    for (column, value) in columns.iter().zip(values) {
        let tag = kind_tag(column)?;
        let text =
            convert::encode_value(column, value).map_err(CanonicalError::SourceConversion)?;
        append_field(
            &mut row,
            tag,
            text.map(|text| {
                let text = if column.kind == ValueKind::Binary {
                    text.strip_prefix("\\x")
                        .ok_or(CanonicalError::InvalidText(ValueKind::Binary))?
                } else {
                    &text
                };
                payload(&column.kind, text)
            })
            .transpose()?,
        );
    }
    Ok(row)
}

/// Encode a target row projected as text, with bytea projected by `encode(col,'hex')`.
///
/// The caller is responsible for constructing those projections in the same
/// resolved column order; non-text PostgreSQL OIDs are not accepted here.
pub fn postgres_text_row(
    columns: &[ColumnPlan],
    values: &[Option<&str>],
) -> Result<Vec<u8>, CanonicalError> {
    if columns.len() != values.len() {
        return Err(CanonicalError::RowWidth);
    }
    let mut row = Vec::new();
    for (column, value) in columns.iter().zip(values) {
        let tag = kind_tag(column)?;
        append_field(
            &mut row,
            tag,
            value.map(|text| payload(&column.kind, text)).transpose()?,
        );
    }
    Ok(row)
}

fn kind_tag(column: &ColumnPlan) -> Result<u8, CanonicalError> {
    let transform_supported = match column.kind {
        ValueKind::SignedInteger | ValueKind::UnsignedInteger => matches!(
            column.transform.as_deref(),
            None | Some("tinyint-to-integer" | "year-to-integer")
        ),
        ValueKind::Decimal => column.transform.is_none(),
        ValueKind::Text => matches!(
            column.transform.as_deref(),
            None | Some("right-trim" | "remove-null-characters" | "empty-string-to-null")
        ),
        ValueKind::Boolean => matches!(
            column.transform.as_deref(),
            None | Some("tinyint-to-boolean" | "bits-to-boolean")
        ),
        ValueKind::Binary => matches!(
            column.transform.as_deref(),
            None | Some(
                "binary-to-bytea" | "byte-vector-to-bytea" | "bytes-to-pg-bytea" | "hex-to-bytea"
            )
        ),
        ValueKind::Date | ValueKind::Datetime | ValueKind::Timestamp => matches!(
            column.transform.as_deref(),
            None | Some("zero-dates-to-null" | "zero-dates-to-epoch")
        ),
        _ => false,
    };
    if !transform_supported {
        return Err(CanonicalError::UnsupportedKind(column.kind.clone()));
    }
    Ok(match column.kind {
        ValueKind::SignedInteger => 1,
        ValueKind::UnsignedInteger => 2,
        ValueKind::Decimal => 3,
        ValueKind::Text => 4,
        ValueKind::Boolean => 5,
        ValueKind::Binary => 6,
        ValueKind::Date => 7,
        ValueKind::Datetime => 8,
        ValueKind::Timestamp => 9,
        _ => return Err(CanonicalError::UnsupportedKind(column.kind.clone())),
    })
}

/// Validate a table's selected projection before opening either row stream.
pub fn validate_columns(columns: &[ColumnPlan]) -> Result<(), CanonicalError> {
    if columns.is_empty() {
        return Err(CanonicalError::RowWidth);
    }
    for column in columns {
        kind_tag(column)?;
    }
    Ok(())
}

fn payload(kind: &ValueKind, text: &str) -> Result<Vec<u8>, CanonicalError> {
    match kind {
        ValueKind::SignedInteger | ValueKind::UnsignedInteger => text
            .parse::<i128>()
            .map(i128::to_be_bytes)
            .map(Vec::from)
            .map_err(|_| CanonicalError::InvalidText(kind.clone())),
        ValueKind::Decimal => decimal(text, kind),
        ValueKind::Text => Ok(text.as_bytes().to_vec()),
        ValueKind::Binary => {
            if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(CanonicalError::InvalidText(ValueKind::Binary));
            }
            Ok(text
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect())
        }
        ValueKind::Boolean => match text {
            "true" | "t" => Ok(vec![1]),
            "false" | "f" => Ok(vec![0]),
            _ => Err(CanonicalError::InvalidText(kind.clone())),
        },
        ValueKind::Date | ValueKind::Datetime | ValueKind::Timestamp => temporal(text, kind),
        _ => Err(CanonicalError::UnsupportedKind(kind.clone())),
    }
}

fn temporal(text: &str, kind: &ValueKind) -> Result<Vec<u8>, CanonicalError> {
    let invalid = || CanonicalError::InvalidText(kind.clone());
    // Both sessions use ISO dates and UTC; offsets must never be silently discarded.
    let text = if *kind == ValueKind::Timestamp {
        text.strip_suffix("+00").ok_or_else(invalid)?
    } else {
        text
    };
    let (date, time) = if *kind == ValueKind::Date {
        (text, None)
    } else {
        let (date, time) = text.split_once(' ').ok_or_else(invalid)?;
        (date, Some(time))
    };
    if date.len() != 10
        || !date.bytes().enumerate().all(|(i, byte)| {
            if i == 4 || i == 7 {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return Err(invalid());
    }
    let year: u16 = date[..4].parse().unwrap();
    let month: u8 = date[5..7].parse().unwrap();
    let day: u8 = date[8..].parse().unwrap();
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let last_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if year == 0 || day == 0 || day > last_day {
        return Err(invalid());
    }
    let Some(time) = time else {
        return Ok(date.as_bytes().to_vec());
    };
    let (clock, fraction) = time.split_once('.').unwrap_or((time, ""));
    if clock.len() != 8
        || !clock.bytes().enumerate().all(|(i, byte)| {
            if i == 2 || i == 5 {
                byte == b':'
            } else {
                byte.is_ascii_digit()
            }
        })
        || fraction.len() > 6
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        || time.ends_with('.')
    {
        return Err(invalid());
    }
    if clock[..2].parse::<u8>().unwrap() > 23
        || clock[3..5].parse::<u8>().unwrap() > 59
        || clock[6..].parse::<u8>().unwrap() > 59
    {
        return Err(invalid());
    }
    Ok(format!("{date} {clock}.{fraction:0<6}").into_bytes())
}

fn decimal(text: &str, kind: &ValueKind) -> Result<Vec<u8>, CanonicalError> {
    let (negative, unsigned) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let mut pieces = unsigned.split('.');
    let integer = pieces.next().unwrap_or_default();
    let fraction = pieces.next().unwrap_or_default();
    if pieces.next().is_some()
        || (integer.is_empty() && fraction.is_empty())
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CanonicalError::InvalidText(kind.clone()));
    }

    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = fraction.trim_end_matches('0');
    let is_zero = integer == "0" && fraction.is_empty();
    let mut normalized = String::with_capacity(text.len());
    if negative && !is_zero {
        normalized.push('-');
    }
    normalized.push_str(integer);
    if !fraction.is_empty() {
        normalized.push('.');
        normalized.push_str(fraction);
    }
    Ok(normalized.into_bytes())
}

fn append_field(row: &mut Vec<u8>, tag: u8, value: Option<Vec<u8>>) {
    row.push(tag);
    match value {
        None => row.push(0),
        Some(value) => {
            row.push(1);
            row.extend_from_slice(&(value.len() as u64).to_be_bytes());
            row.extend_from_slice(&value);
        }
    }
}
