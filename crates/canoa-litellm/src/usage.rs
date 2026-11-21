//! What a LiteLLM key has used, read from the proxy's `/key/info`: its spend against its budget, and its request and token limits.
//!
//! The proxy's document is made into buckets by `coqui_acp_api::key_usage`: one covers every model (the budget, `spend` against
//! `max_budget`, with its reset), and a bucket each for `rpm_limit` and `tpm_limit` when the key has them. The request is made
//! here, through the host's HTTP client, so the proxy is reached the way every other address is.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a clock is std's"
)]

use std::time::{SystemTime, UNIX_EPOCH};

use canoa::api_gateway::http;
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::key_usage::{SOURCE_GATEWAY, gateway_buckets};
use coqui_acp_api::{AccountUsage, Bucket, Source};
use serde_json::Value;

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

/// The buckets a key's `info` object says, as of `observed` (Unix seconds).
#[must_use]
pub fn buckets_of(info: &Value, observed: i64) -> Vec<Bucket> {
    gateway_buckets(info, observed)
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
        .map_err(|why| ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &why.to_string()))?;
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
            usage.sources = vec![Source::Other(SOURCE_GATEWAY.to_owned())];
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
    use core::time::Duration;

    use coqui_acp_api::{Coverage, Refill, Status as BucketStatus, Unit};
    use serde_json::json;

    use super::*;

    const AT: i64 = 1_800_000_000;

    #[test]
    fn spend_against_a_budget_is_one_bucket_with_its_refill_and_its_status() {
        let info = json!({
            "spend": 12.5, "max_budget": 50.0, "budget_duration": "30d",
            "budget_reset_at": "2027-02-01T00:00:00Z",
        });
        let buckets = buckets_of(&json!({ "info": info }), AT);
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
        assert!(buckets_of(&json!({"models": ["m"]}), AT).is_empty());
    }

    #[test]
    fn a_key_with_no_budget_reports_its_spend_and_no_limit() {
        let buckets = buckets_of(&json!({"spend": 0.0001065, "max_budget": null}), AT);
        assert_eq!(buckets.len(), 1);
        assert_eq!((buckets[0].used, buckets[0].limit), (Some(107), None));
    }
}
