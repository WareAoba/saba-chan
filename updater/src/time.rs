//! ISO 8601 시각 문자열 (chrono 없이)

/// 지금 (UTC)
pub(crate) fn now_iso() -> String {
    format_unix_timestamp(unix_now())
}

/// 지금부터 `hours` 시간 뒤 (UTC)
pub(crate) fn iso_after_hours(hours: u32) -> String {
    format_unix_timestamp(unix_now() + hours as u64 * 3600)
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn format_unix_timestamp(secs: u64) -> String {
    // 단순 UTC 문자열 포맷팅
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Unix epoch (1970-01-01) 기준 날짜 변환
    let (year, month, day) = days_to_date(days);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

fn days_to_date(days: u64) -> (u64, u64, u64) {
    // 단순하게 윤년 판정과 월별 일수 계산
    let mut y = 1970;
    let mut remaining = days as i64;

    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let days_in_months: [i64; 12] = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0;
    for (i, &dim) in days_in_months.iter().enumerate() {
        if remaining < dim {
            m = i;
            break;
        }
        remaining -= dim;
    }

    (y as u64, (m + 1) as u64, (remaining + 1) as u64)
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}
