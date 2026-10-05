//! MySQL catalog literals are parsed as typed values before PostgreSQL DDL is emitted.
use super::{ConversionError, error};
use crate::model::{ColumnPlan, RawValue, ValueKind};

pub(super) fn temporal(column: &ColumnPlan, text: &str) -> Result<RawValue, ConversionError> {
    let invalid = || error(column, "invalid typed temporal default");
    let parse = |text: &str| text.parse::<u32>().map_err(|_| invalid());
    let clock = |text: &str| -> Result<(u32, u32, u32, u32), ConversionError> {
        let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
        let mut fields = whole.split(':');
        let hour = parse(fields.next().ok_or_else(invalid)?)?;
        let minute = parse(fields.next().ok_or_else(invalid)?)?;
        let second = parse(fields.next().ok_or_else(invalid)?)?;
        if fields.next().is_some()
            || fraction.len() > 6
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
            || (text.ends_with('.') && fraction.is_empty())
        {
            return Err(invalid());
        }
        let micros = if fraction.is_empty() {
            0
        } else {
            parse(fraction)? * 10u32.pow(6 - fraction.len() as u32)
        };
        Ok((hour, minute, second, micros))
    };
    if column.kind == ValueKind::Interval {
        let negative = text.starts_with('-');
        let (hour, minute, second, micros) = clock(text.strip_prefix('-').unwrap_or(text))?;
        return Ok(RawValue::Time {
            negative,
            days: hour / 24,
            hour: (hour % 24) as u8,
            minute: minute.try_into().map_err(|_| invalid())?,
            second: second.try_into().map_err(|_| invalid())?,
            micros,
        });
    }
    let (calendar, time) = text.split_once(' ').unwrap_or((text, ""));
    let mut fields = calendar.split('-');
    let year = parse(fields.next().ok_or_else(invalid)?)?;
    let month = parse(fields.next().ok_or_else(invalid)?)?;
    let day = parse(fields.next().ok_or_else(invalid)?)?;
    if fields.next().is_some() || (column.kind != ValueKind::Date && time.is_empty()) {
        return Err(invalid());
    }
    let (hour, minute, second, micros) = if time.is_empty() {
        (0, 0, 0, 0)
    } else {
        clock(time)?
    };
    Ok(RawValue::Date {
        year: year.try_into().map_err(|_| invalid())?,
        month: month.try_into().map_err(|_| invalid())?,
        day: day.try_into().map_err(|_| invalid())?,
        hour: hour.try_into().map_err(|_| invalid())?,
        minute: minute.try_into().map_err(|_| invalid())?,
        second: second.try_into().map_err(|_| invalid())?,
        micros,
    })
}
