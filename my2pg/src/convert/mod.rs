//! Pure typed conversion. Values are represented once, then COPY escaping is applied.
use crate::model::{ColumnPlan, RawValue, TablePlan, ValueKind};
use std::fmt::{self, Write};

mod defaults;
mod json;
mod text;

/// Parse a source catalog temporal literal; encode_value applies domain, precision and NULL policy.
pub fn temporal_default(column: &ColumnPlan, text: &str) -> Result<RawValue, ConversionError> {
    defaults::temporal(column, text)
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("column {column:?}: {reason}")]
pub struct ConversionError {
    pub column: Option<String>,
    pub reason: String,
}

fn error(column: &ColumnPlan, reason: &str) -> ConversionError {
    ConversionError {
        column: Some(column.source_name.clone()),
        reason: reason.into(),
    }
}

const SCALAR_BYTES: usize = 512;
struct Scalar {
    bytes: [u8; SCALAR_BYTES],
    length: usize,
}
impl Scalar {
    fn new() -> Self {
        Self {
            bytes: [0; SCALAR_BYTES],
            length: 0,
        }
    }
    fn text(&self) -> &str {
        // fmt::Write only copies complete UTF-8 strings.
        std::str::from_utf8(&self.bytes[..self.length]).expect("formatter UTF-8")
    }
}
impl fmt::Write for Scalar {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let next = self.length.checked_add(text.len()).ok_or(fmt::Error)?;
        if next > SCALAR_BYTES {
            return Err(fmt::Error);
        }
        self.bytes[self.length..next].copy_from_slice(text.as_bytes());
        self.length = next;
        Ok(())
    }
}
fn scalar(column: &ColumnPlan, arguments: fmt::Arguments<'_>) -> Result<Scalar, ConversionError> {
    let mut scalar = Scalar::new();
    scalar
        .write_fmt(arguments)
        .map_err(|_| error(column, "scalar representation exceeds fixed workspace"))?;
    Ok(scalar)
}

// Inline scalar workspace avoids per-column heap allocation in the row encoder.
#[allow(clippy::large_enum_variant)]
enum Representation<'a> {
    Null,
    Text(&'a str),
    Scalar(Scalar),
    Binary(&'a [u8]),
    Decoded(text::Text<'a>),
    Array(&'a [String], u64),
    Hex(&'a str),
}
impl Representation<'_> {
    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Scalar(text) => Some(text.text()),
            _ => None,
        }
    }
    fn visit(&self, mut output: impl FnMut(u8)) {
        match self {
            Self::Null => {}
            Self::Binary(bytes) => {
                output(b'\\');
                output(b'x');
                const HEX: &[u8] = b"0123456789abcdef";
                for byte in *bytes {
                    output(HEX[usize::from(byte >> 4)]);
                    output(HEX[usize::from(byte & 15)]);
                }
            }
            Self::Hex(hex) => {
                output(b'\\');
                output(b'x');
                for byte in hex.bytes() {
                    output(byte.to_ascii_lowercase());
                }
            }
            Self::Decoded(text) => text.visit(output),
            Self::Array(labels, mask) => {
                output(b'{');
                let mut first = true;
                for (index, label) in labels.iter().enumerate() {
                    if mask & (1u64 << index) == 0 {
                        continue;
                    }
                    if !first {
                        output(b',');
                    }
                    first = false;
                    output(b'"');
                    for byte in label.bytes() {
                        if matches!(byte, b'"' | b'\\') {
                            output(b'\\');
                        }
                        output(byte);
                    }
                    output(b'"');
                }
                output(b'}');
            }
            _ => {
                for byte in self.text().expect("text representation").bytes() {
                    output(byte);
                }
            }
        }
    }
    fn encoded_length(&self) -> Option<usize> {
        if matches!(self, Self::Null) {
            return Some(2);
        }
        let mut length = Some(0usize);
        self.visit(|byte| {
            length = length
                .and_then(|length| length.checked_add(if escaped(byte).is_some() { 2 } else { 1 }))
        });
        length
    }
    fn write_copy(&self, row: &mut Vec<u8>) {
        if matches!(self, Self::Null) {
            row.extend_from_slice(b"\\N");
            return;
        }
        self.visit(|byte| {
            if let Some(escape) = escaped(byte) {
                row.extend_from_slice(escape);
            } else {
                row.push(byte);
            }
        });
    }
}
fn escaped(byte: u8) -> Option<&'static [u8; 2]> {
    match byte {
        b'\\' => Some(b"\\\\"),
        b'\t' => Some(b"\\t"),
        b'\n' => Some(b"\\n"),
        b'\r' => Some(b"\\r"),
        8 => Some(b"\\b"),
        12 => Some(b"\\f"),
        11 => Some(b"\\v"),
        _ => None,
    }
}

/// Compatibility wrapper. Pipelines must use encode_row_limited with their admitted limit.
pub fn encode_row(plan: &TablePlan, values: &[RawValue]) -> Result<Vec<u8>, ConversionError> {
    encode_row_limited(plan, values, usize::MAX)
}

/// Measure the complete escaped row before allocating output; no binary/text copy workspace.
pub fn encode_row_limited(
    plan: &TablePlan,
    values: &[RawValue],
    maximum: usize,
) -> Result<Vec<u8>, ConversionError> {
    let columns = || plan.columns.iter().filter(|column| column.copy);
    if columns().count() != values.len() {
        return Err(ConversionError {
            column: None,
            reason: "row width differs from resolved COPY columns".into(),
        });
    }
    let oversized = || ConversionError {
        column: None,
        reason: "encoded COPY row exceeds admitted byte limit".into(),
    };
    let mut length = 1usize; // final newline
    if length > maximum {
        return Err(oversized());
    }
    for (index, (column, value)) in columns().zip(values).enumerate() {
        if let RawValue::Bytes(bytes) = value
            && !matches!(
                column.transform.as_deref(),
                Some("right-trim" | "remove-null-characters" | "empty-string-to-null")
            )
            && matches!(
                column.kind,
                ValueKind::Text | ValueKind::Decimal | ValueKind::Json | ValueKind::Binary
            )
            && bytes.len()
                > maximum
                    .saturating_sub(length)
                    .saturating_sub(usize::from(index > 0))
        {
            return Err(oversized());
        }
        let representation = represent(column, value, maximum, true)?;
        length = length
            .checked_add(usize::from(index > 0))
            .and_then(|length| length.checked_add(representation.encoded_length()?))
            .ok_or_else(oversized)?;
        if length > maximum {
            return Err(oversized());
        }
    }
    let mut row = Vec::new();
    row.try_reserve_exact(length).map_err(|_| ConversionError {
        column: None,
        reason: "cannot allocate admitted COPY row".into(),
    })?;
    for (index, (column, value)) in columns().zip(values).enumerate() {
        if index > 0 {
            row.push(b'\t');
        }
        // Inputs are immutably borrowed for both passes; expensive JSON validation
        // has already succeeded before output allocation.
        represent(column, value, maximum, false)?.write_copy(&mut row);
    }
    row.push(b'\n');
    debug_assert_eq!(row.len(), length);
    Ok(row)
}

/// Conservative retained parser heap for an admitted raw JSON field of at most maximum bytes.
/// serde_json1.0.151 retains a decoded-string scratch Vec and one arbitrary-precision
/// number String. Rust1.99 grows either buffer to max(2*old_capacity,required), with
/// byte minimum8, number initial16; Unicode scratch can request four spare bytes.
/// Therefore scratch<=max(8,2*(maximum+4)) and token<=max(16,2*maximum).
/// Output capacity is accounted separately. Scalar payloads use a fixed512-byte stack.
/// Allocator reallocation/rounding, recursive stack and driver buffers are separate RSS costs.
pub fn encoder_workspace_bytes(maximum: usize) -> Result<usize, ConversionError> {
    let overflow = || ConversionError {
        column: None,
        reason: "encoder workspace arithmetic overflow".into(),
    };
    let scratch = maximum
        .checked_add(4)
        .and_then(|size| size.checked_mul(2))
        .ok_or_else(overflow)?
        .max(8);
    let number = maximum.checked_mul(2).ok_or_else(overflow)?.max(16);
    scratch.checked_add(number).ok_or_else(overflow)
}

/// Unescaped PostgreSQL input text; None is SQL NULL. This owned API allocates its representation.
pub fn encode_value(
    column: &ColumnPlan,
    value: &RawValue,
) -> Result<Option<String>, ConversionError> {
    let represented = represent(column, value, usize::MAX, true)?;
    if matches!(represented, Representation::Null) {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    represented.visit(|byte| bytes.push(byte));
    Ok(Some(
        String::from_utf8(bytes).expect("representation UTF-8"),
    ))
}

fn represent<'a>(
    column: &'a ColumnPlan,
    value: &'a RawValue,
    maximum: usize,
    validate_json: bool,
) -> Result<Representation<'a>, ConversionError> {
    let enum_label = if !column.enum_labels.is_empty() && !matches!(value, RawValue::Null) {
        Some(enum_label(column, value)?)
    } else {
        None
    };
    let transformed_null = match column.transform.as_deref() {
        Some("zero-dates-to-null") => matches!(
            value,
            RawValue::Date { year: 0, .. }
                | RawValue::Date { month: 0, .. }
                | RawValue::Date { day: 0, .. }
        ),
        Some("empty-string-to-null") => {
            enum_label.is_some_and(str::is_empty)
                || matches!(value, RawValue::Bytes(bytes) if bytes.is_empty())
        }
        Some(
            "tinyint-to-boolean"
            | "bits-to-boolean"
            | "remove-null-characters"
            | "right-trim"
            | "byte-vector-to-bytea"
            | "bytes-to-pg-bytea"
            | "hex-to-bytea"
            | "set-to-array"
            | "year-to-integer"
            | "tinyint-to-integer"
            | "int-to-ip",
        )
        | None => false,
        Some(_) => return Err(error(column, "named transform is not yet implemented")),
    };
    if matches!(value, RawValue::Null) || transformed_null {
        return if column.nullable {
            Ok(Representation::Null)
        } else {
            Err(error(
                column,
                "NULL contradicts the resolved NOT NULL policy",
            ))
        };
    }
    if column.transform.as_deref() == Some("int-to-ip") {
        let number = integer(column, value)?;
        let ip = u32::try_from(number)
            .map_err(|_| error(column, "IPv4 numeric value exceeds u32 domain"))?;
        return Ok(Representation::Scalar(scalar(
            column,
            format_args!("{}", std::net::Ipv4Addr::from(ip)),
        )?));
    }
    let represented = match column.kind {
        ValueKind::SignedInteger | ValueKind::UnsignedInteger => {
            let number = integer(column, value)?;
            if (column.kind == ValueKind::UnsignedInteger
                || column.source_type.to_ascii_lowercase().contains("unsigned"))
                && number < 0
            {
                return Err(error(column, "negative value in an unsigned column"));
            }
            if column.source_type.eq_ignore_ascii_case("year")
                && number != 0
                && !(1901..=2155).contains(&number)
            {
                return Err(error(column, "YEAR exceeds the MySQL domain"));
            }
            let range = match column.target_type.as_str() {
                "smallint" => Some((i16::MIN as i128, i16::MAX as i128)),
                "integer" | "int" => Some((i32::MIN as i128, i32::MAX as i128)),
                "bigint" => Some((i64::MIN as i128, i64::MAX as i128)),
                _ => None,
            };
            if range.is_some_and(|(min, max)| number < min || number > max) {
                return Err(error(column, "integer exceeds the target range"));
            }
            Representation::Scalar(scalar(column, format_args!("{number}"))?)
        }
        ValueKind::Decimal => {
            let represented = scalar_text(column, value)?;
            decimal(column, represented.text().unwrap())?;
            represented
        }
        ValueKind::Float => Representation::Scalar(float(column, value)?),
        ValueKind::Binary => match value {
            RawValue::Bytes(bytes) if column.transform.as_deref() == Some("hex-to-bytea") => {
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| error(column, "hex transform requires ASCII hexadecimal"))?;
                let hex = text
                    .strip_prefix("0x")
                    .or_else(|| text.strip_prefix("\\x"))
                    .unwrap_or(text);
                if !hex.len().is_multiple_of(2) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(error(
                        column,
                        "hex transform requires even valid hexadecimal digits",
                    ));
                }
                Representation::Hex(hex)
            }
            RawValue::Bytes(bytes) => Representation::Binary(bytes),
            _ => return Err(error(column, "expected exact binary bytes")),
        },
        ValueKind::Date | ValueKind::Datetime | ValueKind::Timestamp => {
            Representation::Scalar(date(column, value)?)
        }
        ValueKind::Interval => match value {
            RawValue::Time {
                negative,
                days,
                hour,
                minute,
                second,
                micros,
            } => {
                if *hour >= 24 || *minute >= 60 || *second >= 60 || *micros >= 1_000_000 {
                    return Err(error(column, "invalid temporal components"));
                }
                let hours = u64::from(*days) * 24 + u64::from(*hour);
                if hours > 838 {
                    return Err(error(column, "duration exceeds the MySQL TIME domain"));
                }
                precision(column, *micros)?;
                if column.target_type.starts_with("time") && (*negative || hours >= 24) {
                    return Err(error(
                        column,
                        "duration cannot be represented as time of day",
                    ));
                }
                Representation::Scalar(scalar(
                    column,
                    format_args!(
                        "{}{hours:02}:{minute:02}:{second:02}.{micros:06}",
                        if *negative { "-" } else { "" }
                    ),
                )?)
            }
            _ => return Err(error(column, "expected MySQL duration components")),
        },
        ValueKind::Boolean => {
            let number = if column.transform.as_deref() == Some("bits-to-boolean")
                || column.source_type.to_ascii_lowercase().starts_with("bit(")
            {
                i128::from(bit_integer(column, value)?)
            } else {
                integer(column, value)?
            };
            if column.transform.as_deref() != Some("tinyint-to-boolean") && !matches!(number, 0 | 1)
            {
                return Err(error(
                    column,
                    "boolean accepts only 0 or 1 without an explicit legacy transform",
                ));
            }
            Representation::Text(if number == 0 { "false" } else { "true" })
        }
        ValueKind::Bit(width) => {
            if width == 0 || width > 64 {
                return Err(error(column, "BIT width must be within 1..=64"));
            }
            let number = bit_integer(column, value)?;
            if width < 64 && number >= (1u64 << width) {
                return Err(error(column, "BIT value exceeds resolved width"));
            }
            Representation::Scalar(scalar(
                column,
                format_args!("{number:0width$b}", width = usize::from(width)),
            )?)
        }
        ValueKind::Text => {
            if let Some(label) = enum_label {
                let text = text::Text::metadata(column, label.as_bytes())?;
                if let Some(limit) = type_parameter(&column.target_type, 0)
                    && text.characters() > limit as usize
                {
                    return Err(error(column, "text exceeds target character length"));
                }
                Representation::Decoded(text)
            } else if let RawValue::Bytes(bytes) = value {
                let text = text::Text::new(column, bytes)?;
                if let Some(limit) = type_parameter(&column.target_type, 0)
                    && text.characters() > limit as usize
                {
                    return Err(error(column, "text exceeds target character length"));
                }
                Representation::Decoded(text)
            } else {
                let represented = scalar_text(column, value)?;
                if let Some(limit) = type_parameter(&column.target_type, 0)
                    && represented.text().unwrap().chars().count() > limit as usize
                {
                    return Err(error(column, "text exceeds target character length"));
                }
                represented
            }
        }
        ValueKind::Json => {
            let represented = scalar_text(column, value)?;
            if represented.text().unwrap().len() > maximum {
                return Err(error(
                    column,
                    "encoded COPY row exceeds admitted byte limit",
                ));
            }
            if validate_json {
                json::validate(represented.text().unwrap())
                    .map_err(|_| error(column, "invalid JSON or NUL not representable by jsonb"))?;
            }
            represented
        }
        ValueKind::Enum => Representation::Decoded(text::Text::metadata(
            column,
            enum_label
                .ok_or_else(|| error(column, "missing declared ENUM labels"))?
                .as_bytes(),
        )?),
        ValueKind::Set => {
            let mask = bit_integer(column, value)?;
            let labels = &column.set_labels;
            if labels.is_empty()
                || labels.len() > 64
                || (labels.len() < 64 && mask >> labels.len() != 0)
            {
                return Err(error(column, "SET membership exceeds declared labels"));
            }
            if labels.iter().any(|label| label.contains('\0')) {
                return Err(error(column, "NUL is not representable in SET labels"));
            }
            Representation::Array(labels, mask)
        }
        ValueKind::Geometry => {
            let represented = scalar_text(column, value)?;
            let text = represented.text().ok_or_else(|| {
                error(
                    column,
                    "expected EWKT text from the MySQL source projection",
                )
            })?;
            let Some((srid, wkt)) = text
                .strip_prefix("SRID=")
                .and_then(|text| text.split_once(';'))
            else {
                return Err(error(
                    column,
                    "invalid EWKT value from the MySQL source projection",
                ));
            };
            if srid.is_empty() || !srid.bytes().all(|byte| byte.is_ascii_digit()) || wkt.is_empty()
            {
                return Err(error(
                    column,
                    "invalid EWKT value from the MySQL source projection",
                ));
            }
            represented
        }
    };
    if represented.text().is_some_and(|text| text.contains('\0')) {
        return Err(error(
            column,
            "NUL is not representable in PostgreSQL text input",
        ));
    }
    Ok(represented)
}

fn bit_integer(column: &ColumnPlan, value: &RawValue) -> Result<u64, ConversionError> {
    match value {
        RawValue::Bytes(bytes) if bytes.len() <= 8 => Ok(bytes
            .iter()
            .fold(0u64, |number, byte| (number << 8) | u64::from(*byte))),
        RawValue::UInt(number) => Ok(*number),
        RawValue::Int(number) if *number >= 0 => Ok(*number as u64),
        _ => Err(error(
            column,
            "expected binary bits or unsigned membership mask",
        )),
    }
}
fn enum_label<'a>(column: &'a ColumnPlan, value: &RawValue) -> Result<&'a str, ConversionError> {
    let ordinal = match value {
        RawValue::UInt(number) => *number,
        RawValue::Int(number) if *number >= 0 => *number as u64,
        _ => return Err(error(column, "ENUM requires numeric source ordinal")),
    };
    let index = ordinal
        .checked_sub(1)
        .and_then(|number| usize::try_from(number).ok())
        .ok_or_else(|| error(column, "invalid MySQL ENUM sentinel ordinal"))?;
    column
        .enum_labels
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| error(column, "ENUM ordinal exceeds declared labels"))
}

/// Counts explicit changes to a value; ordinary representation changes are not transformations.
/// NULL values are unchanged. Validation/encoding remains authoritative for rejected values.
pub fn transformation_applied(
    column: &ColumnPlan,
    value: &RawValue,
) -> Result<bool, ConversionError> {
    if matches!(value, RawValue::Null) {
        return Ok(false);
    }
    Ok(match column.transform.as_deref() {
        Some("zero-dates-to-null") => matches!(
            value,
            RawValue::Date { year: 0, .. }
                | RawValue::Date { month: 0, .. }
                | RawValue::Date { day: 0, .. }
        ),
        Some("empty-string-to-null") => {
            if column.enum_labels.is_empty() {
                matches!(value,RawValue::Bytes(bytes) if bytes.is_empty())
            } else {
                enum_label(column, value)?.is_empty()
            }
        }
        Some("remove-null-characters") => {
            if !column.enum_labels.is_empty() {
                enum_label(column, value)?.contains('\0')
            } else {
                matches!(value,RawValue::Bytes(bytes) if bytes.contains(&0))
            }
        }
        Some("right-trim") => {
            if !column.enum_labels.is_empty() {
                let label = enum_label(column, value)?;
                text::Text::metadata(column, label.as_bytes())?.retained_bytes() != label.len()
            } else if let RawValue::Bytes(bytes) = value {
                let after = text::Text::new(column, bytes)?;
                after.retained_bytes() != bytes.len()
            } else {
                false
            }
        }
        Some("tinyint-to-boolean") => !matches!(integer(column, value)?, 0 | 1),
        Some("hex-to-bytea" | "int-to-ip") => true,
        Some(
            "bits-to-boolean"
            | "byte-vector-to-bytea"
            | "bytes-to-pg-bytea"
            | "set-to-array"
            | "year-to-integer"
            | "tinyint-to-integer",
        )
        | None => false,
        Some(_) => return Err(error(column, "unknown named transform")),
    })
}

fn integer(column: &ColumnPlan, value: &RawValue) -> Result<i128, ConversionError> {
    match value {
        RawValue::Int(value) => Ok(i128::from(*value)),
        RawValue::UInt(value) => Ok(i128::from(*value)),
        RawValue::Bytes(bytes) => std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| error(column, "invalid exact integer")),
        _ => Err(error(column, "expected an exact integer")),
    }
}
fn scalar_text<'a>(
    column: &ColumnPlan,
    value: &'a RawValue,
) -> Result<Representation<'a>, ConversionError> {
    match value {
        RawValue::Bytes(bytes) => std::str::from_utf8(bytes)
            .map(Representation::Text)
            .map_err(|_| {
                error(
                    column,
                    "invalid UTF-8; an explicit strict charset policy is required",
                )
            }),
        RawValue::Int(value) => Ok(Representation::Scalar(scalar(
            column,
            format_args!("{value}"),
        )?)),
        RawValue::UInt(value) => Ok(Representation::Scalar(scalar(
            column,
            format_args!("{value}"),
        )?)),
        _ => Err(error(column, "expected bytes or an exact integer")),
    }
}
fn type_parameter(kind: &str, index: usize) -> Option<u32> {
    kind.split_once('(')?
        .1
        .split_once(')')?
        .0
        .split(',')
        .nth(index)?
        .trim()
        .parse()
        .ok()
}
fn decimal(column: &ColumnPlan, text: &str) -> Result<(), ConversionError> {
    if column.source_type.to_ascii_lowercase().contains("unsigned") && text.starts_with('-') {
        return Err(error(column, "negative unsigned decimal"));
    }
    let unsigned = text
        .strip_prefix('-')
        .or_else(|| text.strip_prefix('+'))
        .unwrap_or(text);
    let mut pieces = unsigned.split('.');
    let integral = pieces.next().unwrap_or_default();
    let fractional = pieces.next().unwrap_or_default();
    if pieces.next().is_some()
        || integral.is_empty()
        || (unsigned.ends_with('.') && fractional.is_empty())
        || !integral.bytes().all(|byte| byte.is_ascii_digit())
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(error(
            column,
            "decimal requires exact base-ten digits without an exponent",
        ));
    }
    if integral.trim_start_matches('0').len() + fractional.len() > 65 || fractional.len() > 30 {
        return Err(error(
            column,
            "decimal exceeds the MySQL precision or scale domain",
        ));
    }
    if let Some(precision) = type_parameter(&column.target_type, 0) {
        let scale = type_parameter(&column.target_type, 1).unwrap_or(0);
        if fractional.len() > scale as usize
            || integral.trim_start_matches('0').len() > precision.saturating_sub(scale) as usize
        {
            return Err(error(
                column,
                "decimal exceeds the resolved precision or scale",
            ));
        }
    }
    Ok(())
}
fn date(column: &ColumnPlan, value: &RawValue) -> Result<Scalar, ConversionError> {
    let RawValue::Date {
        year,
        month,
        day,
        hour,
        minute,
        second,
        micros,
    } = value
    else {
        return Err(error(column, "expected raw date components"));
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let limit = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if *year == 0
        || *year > 9999
        || *day == 0
        || *day > limit
        || *hour > 23
        || *minute > 59
        || *second > 59
        || *micros >= 1_000_000
    {
        return Err(error(column, "invalid or zero calendar date"));
    }
    if column.kind == ValueKind::Date
        && (*hour != 0 || *minute != 0 || *second != 0 || *micros != 0)
    {
        return Err(error(
            column,
            "DATE cannot silently discard time components",
        ));
    }
    precision(column, *micros)?;
    let mut text = scalar(column, format_args!("{year:04}-{month:02}-{day:02}"))?;
    if column.kind != ValueKind::Date {
        write!(text, " {hour:02}:{minute:02}:{second:02}.{micros:06}")
            .map_err(|_| error(column, "date exceeds fixed workspace"))?;
    }
    if column.kind == ValueKind::Timestamp {
        text.write_str("+00")
            .map_err(|_| error(column, "timestamp exceeds fixed workspace"))?;
    }
    Ok(text)
}

fn precision(column: &ColumnPlan, micros: u32) -> Result<(), ConversionError> {
    let digits = type_parameter(&column.target_type, 0).unwrap_or(6);
    if digits > 6 || !micros.is_multiple_of(10u32.pow(6 - digits)) {
        return Err(error(
            column,
            "fractional seconds cannot be narrowed without rounding",
        ));
    }
    Ok(())
}

fn float(column: &ColumnPlan, value: &RawValue) -> Result<Scalar, ConversionError> {
    let number = match value {
        RawValue::Float(value) => f64::from(*value),
        RawValue::Double(value) => *value,
        RawValue::Int(value) => {
            let float = *value as f64;
            if float as i128 != i128::from(*value) {
                return Err(error(
                    column,
                    "integer cannot be represented exactly as double precision",
                ));
            }
            float
        }
        RawValue::UInt(value) => {
            let float = *value as f64;
            if float as u128 != u128::from(*value) {
                return Err(error(
                    column,
                    "integer cannot be represented exactly as double precision",
                ));
            }
            float
        }
        _ => return Err(error(column, "expected a finite numeric value")),
    };
    if !number.is_finite() {
        return Err(error(column, "expected a finite numeric value"));
    }
    if column.target_type == "real" {
        let narrowed = number as f32;
        if !narrowed.is_finite() || f64::from(narrowed) != number {
            return Err(error(
                column,
                "double precision cannot be narrowed exactly to real",
            ));
        }
        scalar(column, format_args!("{narrowed}"))
    } else {
        scalar(column, format_args!("{number}"))
    }
}

#[cfg(test)]
mod tests;
