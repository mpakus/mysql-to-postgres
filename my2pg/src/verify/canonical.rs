//! Canonical row encodings for the narrowly supported cross-engine type subset.
//!
//! PostgreSQL values must come from explicit `column::text` projections. This
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
/// `columns` and `values` must be in the exact source projection order. Only
/// signed/unsigned integers, exact decimals, text, and booleans are supported.
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
            text.map(|text| payload(&column.kind, &text)).transpose()?,
        );
    }
    Ok(row)
}

/// Encode a target row whose selected expressions are explicitly cast to `text`.
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
        ValueKind::Boolean => match text {
            "true" | "t" => Ok(vec![1]),
            "false" | "f" => Ok(vec![0]),
            _ => Err(CanonicalError::InvalidText(kind.clone())),
        },
        _ => Err(CanonicalError::UnsupportedKind(kind.clone())),
    }
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
