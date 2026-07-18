use std::collections::BTreeSet;

use chrono::DateTime;
use chrono::Datelike;
use chrono::Local;
use chrono::TimeZone;
use chrono::Timelike;

const MS_PER_MINUTE: i64 = 60_000;
const SEARCH_WINDOW_MINUTES: i64 = 5 * 366 * 24 * 60;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CronExpression {
    pub(crate) raw: String,
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    days_of_month: BTreeSet<u32>,
    months: BTreeSet<u32>,
    days_of_week: BTreeSet<u32>,
    days_of_month_wildcard: bool,
    days_of_week_wildcard: bool,
}

pub(crate) fn parse_cron_expression(raw: &str) -> Result<CronExpression, String> {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Err("cron expression is empty".to_string());
    }
    let fields = normalized.split(' ').collect::<Vec<_>>();
    if fields.len() != 5 {
        return Err(format!(
            "cron expression must have exactly 5 fields (minute hour day-of-month month day-of-week); got {}",
            fields.len()
        ));
    }

    let minutes = parse_field(fields[0], /*min*/ 0, /*max*/ 59, "minute")?;
    let hours = parse_field(fields[1], /*min*/ 0, /*max*/ 23, "hour")?;
    let days_of_month = parse_field(fields[2], /*min*/ 1, /*max*/ 31, "day-of-month")?;
    let months = parse_field(fields[3], /*min*/ 1, /*max*/ 12, "month")?;
    let days_of_week = parse_field(fields[4], /*min*/ 0, /*max*/ 7, "day-of-week")?
        .into_iter()
        .map(|value| if value == 7 { 0 } else { value })
        .collect();
    let days_of_month_wildcard = fields[2] == "*";
    let days_of_week_wildcard = fields[4] == "*";

    Ok(CronExpression {
        raw: normalized,
        minutes,
        hours,
        days_of_month,
        months,
        days_of_week,
        days_of_month_wildcard,
        days_of_week_wildcard,
    })
}

fn parse_field(raw: &str, min: u32, max: u32, name: &str) -> Result<BTreeSet<u32>, String> {
    let mut values = BTreeSet::new();
    for term in raw.split(',') {
        if term.is_empty() {
            return Err(format!("cron {name} field has empty term in list"));
        }
        add_term(&mut values, term, min, max, name)?;
    }
    if values.is_empty() {
        return Err(format!("cron {name} field matches no values"));
    }
    Ok(values)
}

fn add_term(
    values: &mut BTreeSet<u32>,
    term: &str,
    min: u32,
    max: u32,
    name: &str,
) -> Result<(), String> {
    let mut parts = term.split('/');
    let range = parts.next().unwrap_or_default();
    let step = match parts.next() {
        Some("") => return Err(format!("cron {name} step is empty in \"{term}\"")),
        Some(raw_step) => {
            if parts.next().is_some() {
                return Err(format!("cron {name} step is invalid in \"{term}\""));
            }
            let step = parse_cron_int(raw_step, name, "step")?;
            if step == 0 {
                return Err(format!(
                    "cron {name} step must be a positive integer (got \"{raw_step}\")"
                ));
            }
            step
        }
        None => 1,
    };
    if range.is_empty() {
        return Err(format!(
            "cron {name} step needs a range or \"*\" before \"/\" in \"{term}\""
        ));
    }

    let (lo, hi) = if range == "*" {
        (min, max)
    } else if let Some((lo, hi)) = range.split_once('-') {
        let lo = parse_cron_int(lo, name, "range lower bound")?;
        let hi = parse_cron_int(hi, name, "range upper bound")?;
        if lo < min || hi > max || lo > hi {
            return Err(format!(
                "cron {name} range {lo}-{hi} out of bounds (must be {min}..{max}, ascending)"
            ));
        }
        (lo, hi)
    } else {
        let value = parse_cron_int(range, name, "value")?;
        if value < min || value > max {
            return Err(format!(
                "cron {name} value {value} out of range {min}..{max}"
            ));
        }
        if term.contains('/') {
            (value, max)
        } else {
            values.insert(value);
            return Ok(());
        }
    };

    for value in (lo..=hi).step_by(step as usize) {
        values.insert(value);
    }
    Ok(())
}

fn parse_cron_int(raw: &str, name: &str, role: &str) -> Result<u32, String> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "cron {name} {role} must be a non-negative integer with digits only (got {raw:?})"
        ));
    }
    raw.parse::<u32>().map_err(|_| {
        format!("cron {name} {role} must be a non-negative integer with digits only (got {raw:?})")
    })
}

pub(crate) fn next_cron_run(expression: &CronExpression, from_ms: i64) -> Option<i64> {
    let first_minute = from_ms.div_euclid(MS_PER_MINUTE).saturating_add(1);
    for offset in 0..SEARCH_WINDOW_MINUTES {
        let timestamp_ms = first_minute
            .saturating_add(offset)
            .saturating_mul(MS_PER_MINUTE);
        let timestamp = Local.timestamp_millis_opt(timestamp_ms).single()?;
        if expression.matches(timestamp) {
            return Some(timestamp_ms);
        }
    }
    None
}

impl CronExpression {
    fn matches(&self, timestamp: DateTime<Local>) -> bool {
        if !self.months.contains(&timestamp.month())
            || !self.hours.contains(&timestamp.hour())
            || !self.minutes.contains(&timestamp.minute())
        {
            return false;
        }
        let day_of_month_matches = self.days_of_month.contains(&timestamp.day());
        let day_of_week_matches = self
            .days_of_week
            .contains(&timestamp.weekday().num_days_from_sunday());
        match (self.days_of_month_wildcard, self.days_of_week_wildcard) {
            (true, true) => true,
            (true, false) => day_of_week_matches,
            (false, true) => day_of_month_matches,
            (false, false) => day_of_month_matches || day_of_week_matches,
        }
    }

    pub(crate) fn human_schedule(&self) -> String {
        let all_minutes = is_full_range(&self.minutes, /*min*/ 0, /*max*/ 59);
        let all_hours = is_full_range(&self.hours, /*min*/ 0, /*max*/ 23);
        let all_months = is_full_range(&self.months, /*min*/ 1, /*max*/ 12);
        let all_days_of_month = self.days_of_month_wildcard;
        let all_days_of_week = self.days_of_week_wildcard;

        if all_hours && all_days_of_month && all_months && all_days_of_week {
            if let Some(step) = detect_step(&self.minutes, /*min*/ 0, /*max*/ 59)
                && step > 1
            {
                return format!("every {step} minutes");
            }
            if all_minutes {
                return "every minute".to_string();
            }
            if let Some(minute) = only_value(&self.minutes) {
                return format!("at minute {minute} of every hour");
            }
        }

        if let Some(minute) = only_value(&self.minutes)
            && all_days_of_month
            && all_months
            && all_days_of_week
            && let Some(step) = detect_step(&self.hours, /*min*/ 0, /*max*/ 23)
            && step > 1
        {
            return format!("every {step} hours at minute {minute:02}");
        }

        if let (Some(minute), Some(hour)) = (only_value(&self.minutes), only_value(&self.hours))
            && all_days_of_month
            && all_months
        {
            if all_days_of_week {
                return format!("at {hour:02}:{minute:02} every day");
            }
            return format!(
                "at {hour:02}:{minute:02} on {}",
                format_days_of_week(&self.days_of_week)
            );
        }

        self.raw.clone()
    }
}

fn only_value(values: &BTreeSet<u32>) -> Option<u32> {
    (values.len() == 1)
        .then(|| values.iter().next().copied())
        .flatten()
}

fn is_full_range(values: &BTreeSet<u32>, min: u32, max: u32) -> bool {
    values.len() == (max - min + 1) as usize && (min..=max).all(|value| values.contains(&value))
}

fn detect_step(values: &BTreeSet<u32>, min: u32, max: u32) -> Option<u32> {
    let mut values = values.iter().copied();
    let first = values.next()?;
    let second = values.next()?;
    if first != min || second <= first {
        return None;
    }
    let step = second - first;
    let mut expected = second.saturating_add(step);
    for value in values {
        if value != expected {
            return None;
        }
        expected = expected.saturating_add(step);
    }
    (expected.saturating_sub(step) <= max).then_some(step)
}

fn format_days_of_week(values: &BTreeSet<u32>) -> String {
    if values.iter().copied().eq(1..=5) {
        return "weekdays".to_string();
    }
    if values.iter().copied().eq([0, 6]) {
        return "weekends".to_string();
    }
    const NAMES: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    values
        .iter()
        .map(|value| NAMES[*value as usize])
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "kimi_cron_expression_tests.rs"]
mod tests;
