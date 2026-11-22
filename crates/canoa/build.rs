//! Resolves the prefix of the environment variables the host reads: `BRAND_ENV_PREFIX` at build time, else the default.

const DEFAULT_PREFIX: &str = "CANOA";

fn main() {
    println!("cargo:rerun-if-env-changed=BRAND_ENV_PREFIX");
    let prefix = std::env::var("BRAND_ENV_PREFIX")
        .ok()
        .filter(|prefix| !prefix.is_empty())
        .unwrap_or_else(|| DEFAULT_PREFIX.to_owned());
    let mut letters = prefix.chars();
    let valid = letters
        .next()
        .is_some_and(|first| first.is_ascii_uppercase())
        && letters.all(|letter| letter.is_ascii_uppercase() || letter.is_ascii_digit());
    assert!(
        valid,
        "BRAND_ENV_PREFIX is upper-case letters and digits, starting with a letter (got {prefix:?})"
    );
    println!("cargo:rustc-env=CANOA_RESOLVED_ENV_PREFIX={prefix}");
}
