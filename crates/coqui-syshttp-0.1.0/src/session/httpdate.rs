//! HTTP dates (IMF-fixdate, RFC 850, asctime, and the dashed cookie dates servers send), read as
//! unix seconds. A date without a zone is UTC (RFC 9110 section 5.6.7).

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// Days from 1970-01-01 to the civil date (proleptic Gregorian).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted = (month + 9) % 12;
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn month_of(token: &str) -> Option<i64> {
    let lower = token.to_ascii_lowercase();
    let head = lower.get(..3)?;
    MONTHS
        .iter()
        .position(|name| *name == head)
        .map(|index| index as i64 + 1)
}

fn is_weekday(token: &str) -> bool {
    token
        .to_ascii_lowercase()
        .get(..3)
        .is_some_and(|head| WEEKDAYS.contains(&head))
        && token
            .chars()
            .all(|character| character.is_ascii_alphabetic())
}

fn number(token: &str) -> Option<i64> {
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

fn zone_offset(token: &str) -> i64 {
    let hours = match token.to_ascii_uppercase().as_str() {
        "EST" => Some(-5),
        "EDT" | "CDT" => Some(if token.eq_ignore_ascii_case("EDT") {
            -4
        } else {
            -5
        }),
        "CST" => Some(-6),
        "MST" | "PDT" => Some(-7),
        "MDT" => Some(-6),
        "PST" => Some(-8),
        _ => None,
    };
    if let Some(hours) = hours {
        return hours * 3600;
    }
    let sign = match token.as_bytes().first() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return 0,
    };
    let digits = &token[1..];
    match (digits.len(), number(digits)) {
        (4, Some(value)) => sign * ((value / 100) * 3600 + (value % 100) * 60),
        _ => 0,
    }
}

/// An HTTP date as unix seconds, or `None` when it is unreadable.
pub(crate) fn parse(value: &str) -> Option<i64> {
    let mut tokens: Vec<String> = Vec::new();
    for token in value.replace(',', " ").split_whitespace() {
        let dashed = token.contains('-')
            && token.chars().next().is_some_and(|c| c.is_ascii_digit())
            && token.chars().any(|c| c.is_ascii_alphabetic());
        if dashed {
            tokens.extend(
                token
                    .split('-')
                    .filter(|part| !part.is_empty())
                    .map(str::to_string),
            );
        } else {
            tokens.push(token.to_string());
        }
    }
    if tokens.first().is_some_and(|first| is_weekday(first)) {
        tokens.remove(0);
    }
    let (day, month, year, time, zone): (i64, i64, i64, &str, Option<&str>) =
        if tokens.first().is_some_and(|first| number(first).is_some()) {
            // 06 Nov 1994 08:49:37 GMT, 06-Nov-94 08:49:37 GMT
            let [day, month, year, time, rest @ ..] = tokens.as_slice() else {
                return None;
            };
            (
                number(day)?,
                month_of(month)?,
                number(year)?,
                time,
                rest.first().map(String::as_str),
            )
        } else {
            // Nov  6 08:49:37 1994
            let [month, day, time, year, rest @ ..] = tokens.as_slice() else {
                return None;
            };
            (
                number(day)?,
                month_of(month)?,
                number(year)?,
                time,
                rest.first().map(String::as_str),
            )
        };
    let year = match year {
        0..=68 => year + 2000,
        69..=99 => year + 1900,
        other => other,
    };
    let mut parts = time.split(':');
    let hour = number(parts.next()?)?;
    let minute = number(parts.next()?)?;
    let second = parts.next().map_or(Some(0), number)?;
    if parts.next().is_some() || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second.min(59);
    Some(seconds - zone.map_or(0, zone_offset))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn the_three_http_formats_and_the_cookie_dashes() {
        let expected = Some(784_111_777);
        assert_eq!(parse("Sun, 06 Nov 1994 08:49:37 GMT"), expected);
        assert_eq!(parse("Sunday, 06-Nov-94 08:49:37 GMT"), expected);
        assert_eq!(parse("Sun Nov  6 08:49:37 1994"), expected);
        assert_eq!(parse("Sun, 06-Nov-1994 08:49:37 GMT"), expected);
        assert_eq!(parse("06 Nov 1994 08:49:37"), expected);
    }

    #[test]
    fn a_numeric_zone_shifts_the_time() {
        assert_eq!(parse("Sun, 06 Nov 1994 03:49:37 -0500"), Some(784_111_777));
    }

    #[test]
    fn nonsense_is_none() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("tomorrow"), None);
        assert_eq!(parse("Sun, 32 Nov 1994 08:49:37 GMT"), None);
        assert_eq!(parse("Sun, 06 Nov 1994 25:49:37 GMT"), None);
    }

    #[test]
    fn the_epoch_and_before() {
        assert_eq!(parse("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse("Wed, 31 Dec 1969 23:59:59 GMT"), Some(-1));
    }
}
