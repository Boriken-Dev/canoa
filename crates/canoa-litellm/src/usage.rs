//! What a LiteLLM key has used, read from the proxy's `/key/info`: its spend against its budget, and its request and token limits.
//!
//! One bucket covers every model: the budget (`spend` against `max_budget`, in millionths of a dollar, refilling at
//! `budget_reset_at`), and a bucket each for `rpm_limit` and `tpm_limit` when the key has them. The proxy reports a limit and not what
//! is left of it, so those two say their limit and their refill and no more. Every field is optional and a missing one is left out,
//! never guessed.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a clock is std's"
)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use canoa::api_gateway::http;
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::{AccountUsage, Bucket, Coverage, Refill, Source, Status as BucketStatus, Unit};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The id of the budget bucket.
pub const BUDGET: &str = "budget";
/// The id of the request limit bucket.
pub const RPM: &str = "rpm";
/// The id of the token limit bucket.
pub const TPM: &str = "tpm";

/// Seconds since the Unix epoch.
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

/// A dollar amount in millionths, rounded; a negative or non-finite amount is none.
fn micro(dollars: f64) -> Option<u64> {
    (dollars.is_finite() && dollars >= 0.0).then(|| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "checked finite and non-negative; a budget beyond u64 millionths is not one"
        )]
        let millionths = (dollars * 1_000_000.0).round() as u64;
        millionths
    })
}

/// The seconds a duration like `30d`, `12h`, `90m`, `45s` or `1mo` (LiteLLM's `budget_duration`) names.
fn duration_of(text: &str) -> Option<Duration> {
    let text = text.trim();
    let split = text.find(|c: char| !c.is_ascii_digit())?;
    let (count, unit) = text.split_at(split);
    let count: u64 = count.parse().ok()?;
    let each = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        "mo" => 30 * 86_400,
        _ => return None,
    };
    Some(Duration::from_secs(count.checked_mul(each)?))
}

fn whole(info: &Value, key: &str) -> Option<u64> {
    let found = info.get(key)?;
    found
        .as_u64()
        .or_else(|| {
            found
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0)
                .map(|n| {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "checked finite and non-negative"
                    )]
                    let whole = n as u64;
                    whole
                })
        })
        .filter(|limit| *limit > 0)
}

fn dollars(info: &Value, key: &str) -> Option<f64> {
    info.get(key).and_then(Value::as_f64)
}

fn when(info: &Value, key: &str) -> Option<i64> {
    let text = info.get(key)?.as_str()?;
    OffsetDateTime::parse(text, &Rfc3339)
        .ok()
        .map(OffsetDateTime::unix_timestamp)
}

/// The buckets a key's `info` object says, as of `observed` (Unix seconds).
#[must_use]
pub fn buckets_of(info: &Value, observed: i64) -> Vec<Bucket> {
    let mut buckets = Vec::new();
    let mut budget = Bucket::new(BUDGET);
    budget.label = Some("Budget".to_owned());
    budget.models = Coverage::All;
    budget.unit = Unit::MicroUsd;
    budget.observed_at = observed;
    budget.source = Source::Reported;
    let used = dollars(info, "spend").and_then(micro);
    let limit = dollars(info, "max_budget").and_then(micro);
    if used.is_some() || limit.is_some() {
        budget.used = used;
        budget.limit = limit;
        budget.remaining = used
            .zip(limit)
            .map(|(used, limit)| limit.saturating_sub(used));
        let length = info
            .get("budget_duration")
            .and_then(Value::as_str)
            .and_then(duration_of);
        let resets_at = when(info, "budget_reset_at");
        budget.refill = if resets_at.is_some() || length.is_some() {
            Refill::FixedWindow { length, resets_at }
        } else {
            Refill::Never
        };
        budget.status = match (used, limit) {
            (Some(used), Some(limit)) if used >= limit => BucketStatus::Exhausted,
            (_, Some(_)) | (Some(_), None) => BucketStatus::Ok,
            (None, None) => BucketStatus::Unknown,
        };
        buckets.push(budget);
    }
    for (id, label, unit, key) in [
        (RPM, "Requests per minute", Unit::Requests, "rpm_limit"),
        (TPM, "Tokens per minute", Unit::Tokens, "tpm_limit"),
    ] {
        let Some(limit) = whole(info, key) else {
            continue;
        };
        let mut bucket = Bucket::new(id);
        bucket.label = Some(label.to_owned());
        bucket.models = Coverage::All;
        bucket.unit = unit;
        bucket.limit = Some(limit);
        bucket.refill = Refill::TokenBucket {
            amount: limit,
            per: Duration::from_secs(60),
            max: limit,
        };
        bucket.observed_at = observed;
        buckets.push(bucket);
    }
    buckets
}

/// What the key `key` has used at the proxy `base`: its `/key/info`.
///
/// # Errors
///
/// `GONE` when the proxy no longer knows the key (it expired or was revoked), so a host renews it; `INTERNAL` with the reason when
/// the proxy does not answer or answers what is not a key's information. A proxy that has no such route, or does not let the key read
/// its own, is the answer "not offered", not an error.
pub fn read(base: &str, key: &str, account: &str) -> Result<AccountUsage, ProviderError> {
    let reply = http::get_json(&format!("{base}/key/info"), Some(key))
        .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &why))?;
    let mut usage = match reply.status {
        200..=299 => {
            let info = reply
                .body
                .get("info")
                .filter(|info| info.is_object())
                .unwrap_or(&reply.body);
            if !info.is_object() {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_INTERNAL,
                    "the proxy's /key/info is not a key's information",
                ));
            }
            let observed = now();
            let mut usage = AccountUsage::new();
            usage.sources = vec![Source::Reported];
            usage.observed_at = observed;
            usage.buckets = buckets_of(info, observed);
            usage
        }
        401 => {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                "the proxy no longer accepts the key: it has expired or was revoked",
            ));
        }
        403..=405 => AccountUsage::unsupported(format!(
            "the proxy does not let this key read its own limits ({})",
            reply.status
        )),
        other => {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_INTERNAL,
                &format!("the proxy answered {other} to /key/info"),
            ));
        }
    };
    usage.account = Some(account.to_owned());
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const AT: i64 = 1_800_000_000;

    #[test]
    fn spend_against_a_budget_is_one_bucket_with_its_refill_and_its_status() {
        let info = json!({
            "spend": 12.5, "max_budget": 50.0, "budget_duration": "30d",
            "budget_reset_at": "2027-02-01T00:00:00Z",
        });
        let buckets = buckets_of(&info, AT);
        assert_eq!(buckets.len(), 1);
        let budget = &buckets[0];
        assert_eq!(budget.id, BUDGET);
        assert_eq!(budget.unit, Unit::MicroUsd);
        assert_eq!(
            (budget.used, budget.limit, budget.remaining),
            (Some(12_500_000), Some(50_000_000), Some(37_500_000))
        );
        assert_eq!(budget.models, Coverage::All);
        assert_eq!(budget.status, BucketStatus::Ok);
        assert_eq!(
            budget.refill,
            Refill::FixedWindow {
                length: Some(Duration::from_secs(30 * 86_400)),
                resets_at: Some(1_801_440_000),
            }
        );
        // Spent to the budget is exhausted until it resets.
        let spent = buckets_of(&json!({"spend": 50.0, "max_budget": 50.0}), AT);
        assert_eq!(spent[0].status, BucketStatus::Exhausted);
        assert_eq!(spent[0].refill, Refill::Never);
        assert_eq!(spent[0].remaining, Some(0));
    }

    #[test]
    fn a_key_with_no_budget_reports_its_spend_and_no_limit() {
        let buckets = buckets_of(&json!({"spend": 0.0001065, "max_budget": null}), AT);
        assert_eq!(buckets.len(), 1);
        assert_eq!((buckets[0].used, buckets[0].limit), (Some(107), None));
        assert_eq!(buckets[0].status, BucketStatus::Ok);
        assert!(buckets_of(&json!({"models": ["m"]}), AT).is_empty());
    }

    #[test]
    fn request_and_token_limits_are_buckets_of_their_own_when_set() {
        let buckets = buckets_of(
            &json!({"spend": 1.0, "rpm_limit": 60, "tpm_limit": null}),
            AT,
        );
        let ids: Vec<&str> = buckets.iter().map(|bucket| bucket.id.as_str()).collect();
        assert_eq!(ids, [BUDGET, RPM]);
        let rpm = &buckets[1];
        assert_eq!(
            (rpm.unit.clone(), rpm.limit, rpm.used),
            (Unit::Requests, Some(60), None)
        );
        assert_eq!(
            rpm.status,
            BucketStatus::Unknown,
            "the proxy says the limit, not what is left"
        );
        let tpm = buckets_of(&json!({"tpm_limit": 100_000}), AT);
        assert_eq!(tpm[0].id, TPM);
        assert_eq!(tpm[0].unit, Unit::Tokens);
        assert!(buckets_of(&json!({"rpm_limit": 0}), AT).is_empty());
    }

    #[test]
    fn a_budget_duration_is_the_proxys_own_words() {
        for (text, seconds) in [
            ("30s", 30),
            ("90m", 5400),
            ("12h", 43_200),
            ("1d", 86_400),
            ("1mo", 2_592_000),
        ] {
            assert_eq!(
                duration_of(text),
                Some(Duration::from_secs(seconds)),
                "{text}"
            );
        }
        assert_eq!(duration_of("soon"), None);
        assert_eq!(duration_of(""), None);
    }
}
