//! `[host.usage.limits.NAME]`: a rule of the host that warns, and may stop new turns, at a share of what an account, a profile or
//! every session has used. What the host reads of it is parsed here; what it does with it is `limits`.

use alloc::string::String;
use indexmap::IndexMap;

use super::ConfigError;
use super::raw::{RawAmount, RawLimit};

/// What `[usage] warn_permille` is when the file does not say, and so what a limit warns at unless it names its own.
pub const DEFAULT_WARN_PERMILLE: u32 = 800;
/// The longest name of a limit.
pub const MAX_LIMIT_NAME: usize = 32;

/// How long a window of recorded spend is. Windows are in UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitWindow {
    /// From midnight to the next.
    Day,
    /// From Monday's midnight to the next Monday's.
    Week,
    /// From the first of the month to the first of the next.
    Month,
}

impl LimitWindow {
    /// The word `window = ` takes.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    /// The window that holds `now` (Unix seconds): its first second and the first second after it.
    #[must_use]
    pub fn bounds(self, now: i64) -> (i64, i64) {
        const DAY: i64 = 86_400;
        let days = now.div_euclid(DAY);
        match self {
            Self::Day => (days * DAY, (days + 1) * DAY),
            Self::Week => {
                // 1970-01-01 was a Thursday, so the day count plus three is a count from a Monday.
                let monday = days - (days + 3).rem_euclid(7);
                (monday * DAY, (monday + 7) * DAY)
            }
            Self::Month => {
                let (year, month, _) = civil(days);
                let (next_year, next_month) = if month == 12 {
                    (year + 1, 1)
                } else {
                    (year, month + 1)
                };
                (
                    days_from_civil(year, month, 1) * DAY,
                    days_from_civil(next_year, next_month, 1) * DAY,
                )
            }
        }
    }
}

/// The year, month and day of the day count `days` since 1970-01-01 (Howard Hinnant's civil-from-days).
const fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

/// The day count since 1970-01-01 of a civil date.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Whose use a limit counts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitScope {
    /// Every session.
    Everyone,
    /// One account: its name in `[host.accounts]`, or `local:<harness>` for a harness's own login.
    Account(String),
    /// One `[host.profiles]` profile, `default` for the host's own sources.
    Profile(String),
}

/// What a limit counts.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitFigure {
    /// A bucket an agent or a reader reported, by id, or `any` for every bucket that covers the session's model.
    Bucket(String),
    /// Money the host recorded, in millionths of a dollar, in a window.
    Usd {
        /// The amount the limit is set at.
        micro: u64,
        /// The window it is counted in.
        window: LimitWindow,
    },
    /// Input and output tokens the host recorded in a window.
    Tokens {
        /// The amount the limit is set at.
        count: u64,
        /// The window it is counted in.
        window: LimitWindow,
    },
}

/// One `[host.usage.limits.NAME]`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LimitConfig {
    /// Whose use it counts.
    pub scope: LimitScope,
    /// What it counts.
    pub figure: LimitFigure,
    /// From this many thousandths of the figure the limit warns: its own `warn_permille`, else `[usage] warn_permille`.
    pub warn_permille: u32,
    /// From this many thousandths the host refuses new turns; `None` never stops, only warns.
    pub stop_permille: Option<u32>,
}

impl LimitConfig {
    /// The window a recorded figure is counted in; a bucket has the one its provider reports.
    #[must_use]
    pub const fn window(&self) -> Option<LimitWindow> {
        match self.figure {
            LimitFigure::Usd { window, .. } | LimitFigure::Tokens { window, .. } => Some(window),
            LimitFigure::Bucket(_) => None,
        }
    }
}

fn refused(name: &str, what: &str) -> ConfigError {
    ConfigError::Invalid {
        entry: None,
        what: format!("host.usage.limits.{name}: {what}"),
    }
}

/// A limit's name: 1 to 32 lower-case letters, digits, `-` and `_`.
fn check_name(name: &str) -> Result<(), ConfigError> {
    let fine = !name.is_empty()
        && name.len() <= MAX_LIMIT_NAME
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    if fine {
        Ok(())
    } else {
        Err(ConfigError::Invalid {
            entry: None,
            what: format!(
                "host.usage.limits: a name is 1 to {MAX_LIMIT_NAME} lower-case letters, digits, - and _"
            ),
        })
    }
}

/// Money as millionths: `usd` is a decimal of up to six places, whole or fractional.
fn micro_of(name: &str, amount: &RawAmount) -> Result<u64, ConfigError> {
    let text = match amount {
        RawAmount::Whole(whole) => format!("{whole}"),
        RawAmount::Decimal(decimal) if decimal.is_finite() && *decimal > 0.0 => {
            format!("{decimal:.6}")
        }
        RawAmount::Decimal(_) => return Err(refused(name, "usd is more than 0")),
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let micro = whole
        .parse::<u64>()
        .ok()
        .and_then(|whole| whole.checked_mul(1_000_000))
        .and_then(|whole| {
            let fraction = format!("{fraction:0<6}");
            let fraction = fraction.get(..6)?.parse::<u64>().ok()?;
            whole.checked_add(fraction)
        })
        .ok_or_else(|| {
            refused(
                name,
                "usd is a decimal amount of dollars, in six places or fewer",
            )
        })?;
    if micro == 0 {
        return Err(refused(name, "usd is more than 0"));
    }
    Ok(micro)
}

/// The limits `[host.usage.limits]` holds, checked: a key a limit may not hold, two scopes, two figures or a window with a bucket
/// are refused naming the limit and the key.
pub(super) fn limits_of(
    raw: IndexMap<String, RawLimit>,
    default_warn: u32,
) -> Result<IndexMap<String, LimitConfig>, ConfigError> {
    let mut out = IndexMap::new();
    for (name, limit) in raw {
        check_name(&name)?;
        let scope = match (limit.account, limit.profile) {
            (Some(_), Some(_)) => {
                return Err(refused(
                    &name,
                    "account and profile are two scopes: name one",
                ));
            }
            (Some(account), None) if account.trim().is_empty() => {
                return Err(refused(&name, "account is empty"));
            }
            (None, Some(profile)) if profile.trim().is_empty() => {
                return Err(refused(&name, "profile is empty"));
            }
            (Some(account), None) => LimitScope::Account(account),
            (None, Some(profile)) => LimitScope::Profile(profile),
            (None, None) => LimitScope::Everyone,
        };
        let window = match limit.window.as_deref() {
            None => None,
            Some("day") => Some(LimitWindow::Day),
            Some("week") => Some(LimitWindow::Week),
            Some("month") => Some(LimitWindow::Month),
            Some(_) => return Err(refused(&name, "window is day, week or month")),
        };
        let need_window = |what: &str| {
            window.ok_or_else(|| {
                refused(&name, &format!("{what} needs a window: day, week or month"))
            })
        };
        let figure = match (limit.bucket, limit.usd, limit.tokens) {
            (Some(bucket), None, None) => {
                if window.is_some() {
                    return Err(refused(
                        &name,
                        "window goes with usd or tokens: a bucket's window is the one its provider reports",
                    ));
                }
                if bucket.trim().is_empty() {
                    return Err(refused(&name, "bucket is empty"));
                }
                if matches!(scope, LimitScope::Profile(_)) {
                    return Err(refused(
                        &name,
                        "a bucket belongs to an account: use account, or no scope, with bucket",
                    ));
                }
                LimitFigure::Bucket(bucket)
            }
            (None, Some(amount), None) => LimitFigure::Usd {
                micro: micro_of(&name, &amount)?,
                window: need_window("usd")?,
            },
            (None, None, Some(count)) => {
                if count == 0 {
                    return Err(refused(&name, "tokens is more than 0"));
                }
                LimitFigure::Tokens {
                    count,
                    window: need_window("tokens")?,
                }
            }
            _ => {
                return Err(refused(&name, "name exactly one of bucket, usd and tokens"));
            }
        };
        let warn_permille = limit.warn_permille.unwrap_or(default_warn);
        if limit.stop_permille.is_some_and(|stop| stop < warn_permille) {
            return Err(refused(
                &name,
                "stop_permille is at least warn_permille (800 unless [usage] warn_permille or the limit says)",
            ));
        }
        out.insert(
            name,
            LimitConfig {
                scope,
                figure,
                warn_permille,
                stop_permille: limit.stop_permille,
            },
        );
    }
    Ok(out)
}
