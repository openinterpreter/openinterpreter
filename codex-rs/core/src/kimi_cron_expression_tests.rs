use chrono::Local;
use chrono::TimeZone;

use super::next_cron_run;
use super::parse_cron_expression;

#[test]
fn parses_and_renders_common_schedules() {
    let every_minute = parse_cron_expression("* * * * *").expect("valid cron");
    let every_five = parse_cron_expression("*/5 * * * *").expect("valid cron");

    assert_eq!(every_minute.human_schedule(), "every minute");
    assert_eq!(every_five.human_schedule(), "every 5 minutes");
}

#[test]
fn rejects_an_invalid_expression() {
    assert_eq!(
        parse_cron_expression("not a cron"),
        Err(
            "cron expression must have exactly 5 fields (minute hour day-of-month month day-of-week); got 3"
                .to_string()
        )
    );
}

#[test]
fn computes_the_next_local_minute() {
    let expression = parse_cron_expression("* * * * *").expect("valid cron");
    let from = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 50, 12)
        .single()
        .expect("unambiguous local timestamp")
        .timestamp_millis();
    let expected = Local
        .with_ymd_and_hms(2026, 7, 17, 14, 51, 0)
        .single()
        .expect("unambiguous local timestamp")
        .timestamp_millis();

    assert_eq!(next_cron_run(&expression, from), Some(expected));
}
