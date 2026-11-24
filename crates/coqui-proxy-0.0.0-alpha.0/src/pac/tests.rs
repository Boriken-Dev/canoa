use super::*;

/// Wednesday 2026-11-18 12:30:45 UTC.
const NOW: i64 = 1_795_005_045_000;

fn script(source: &str) -> PacScript {
    PacScript::builder(source).clock(NOW).build().unwrap()
}

/// What a script made of `condition` answers: `YES` or `NO`.
fn check(condition: &str) -> bool {
    let pac = script(&alloc::format!(
        "function FindProxyForURL(url, host) {{ return ({condition}) ? 'YES' : 'NO'; }}"
    ));
    pac.find_proxy_for_url("http://example.com/", "example.com")
        .unwrap()
        == "YES"
}

#[test]
fn a_script_chooses_a_proxy_by_host_and_falls_back_to_direct() {
    let pac = script(
        "function FindProxyForURL(url, host) {
           if (dnsDomainIs(host, '.corp.example')) return 'PROXY gateway.corp:3128; DIRECT';
           return 'DIRECT';
         }",
    );
    let routes = pac.routes("https://wiki.corp.example/page").unwrap();
    assert!(
        matches!(&routes[0], Route::Proxy(p) if p.host() == "gateway.corp" && p.port() == 3128)
    );
    assert_eq!(routes[1], Route::Direct);
    assert_eq!(pac.routes("https://example.org/").unwrap(), [Route::Direct]);
}

#[test]
fn a_result_that_is_not_a_string_is_direct() {
    let pac = script("function FindProxyForURL(url, host) { return 42; }");
    assert_eq!(pac.find_proxy_for_url("http://a/", "a").unwrap(), "DIRECT");
}

#[test]
fn a_script_that_raises_is_an_error_naming_no_input() {
    let pac = script("function FindProxyForURL(url, host) { throw new Error('boom'); }");
    assert!(matches!(
        pac.find_proxy_for_url("http://a/", "a"),
        Err(Error::Pac(_))
    ));
}

#[test]
fn a_script_without_the_function_or_with_a_syntax_error_does_not_load() {
    assert!(PacScript::builder("var x = 1;").build().is_err());
    assert!(PacScript::builder("function (").build().is_err());
}

#[test]
fn a_runaway_script_is_abandoned_and_refused_for_a_while() {
    let pac = PacScript::builder("function FindProxyForURL(url, host) { while (true) {} }")
        .timeout(Duration::from_millis(300))
        .build()
        .unwrap();
    let started = Instant::now();
    assert!(pac.find_proxy_for_url("http://a/", "a").is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
    let again = Instant::now();
    assert!(pac.find_proxy_for_url("http://a/", "a").is_err());
    assert!(
        again.elapsed() < Duration::from_millis(100),
        "a refused script answers at once"
    );
}

#[test]
fn string_and_network_builtins_answer_as_the_pac_reference_does() {
    assert!(check("shExpMatch('www.example.com', '*.example.com')"));
    assert!(!check("shExpMatch('WWW.example.com', 'www.*')"));
    assert!(check("isPlainHostName('intranet')"));
    assert!(!check("isPlainHostName('a.b')"));
    assert!(check("dnsDomainIs('www.example.com', '.example.com')"));
    assert!(check("localHostOrDomainIs('www', 'www.example.com')"));
    assert!(!check("localHostOrDomainIs('ww', 'www.example.com')"));
    assert!(check("dnsDomainLevels('a.b.c') === 2"));
    assert!(check("isInNet('10.1.2.3', '10.0.0.0', '255.0.0.0')"));
    assert!(!check("isInNet('192.168.1.1', '10.0.0.0', '255.0.0.0')"));
    assert!(check("isInNetEx('2001:db8::1', '2001:db8::/32')"));
    assert!(check(
        "sortIpAddressList('10.0.0.9;9.0.0.1') === '9.0.0.1; 10.0.0.9'"
    ));
    assert!(check("getClientVersion() === '1.0'"));
}

#[test]
fn dns_builtins_resolve_through_the_system_resolver_and_a_literal_without_it() {
    assert!(check("dnsResolve('127.0.0.1') === '127.0.0.1'"));
    assert!(check("isResolvable('localhost')"));
    assert!(check("isResolvable('no-such-host.invalid') === false"));
    assert!(check("dnsResolve('no-such-host.invalid') === null"));
    assert!(check("myIpAddress().length > 0"));
}

#[test]
fn weekday_range_includes_both_ends_and_wraps_past_saturday() {
    assert!(check("weekdayRange('MON', 'FRI', 'GMT')"));
    assert!(check("weekdayRange('WED', 'GMT')"));
    assert!(!check("weekdayRange('THU', 'FRI', 'GMT')"));
    assert!(check("weekdayRange('FRI', 'MON', 'GMT') === false"));
    assert!(check("weekdayRange('TUE', 'SUN', 'GMT')"));
    assert!(!check("weekdayRange('FUNDAY', 'GMT')"));
}

#[test]
fn time_range_ends_at_the_last_second_of_the_final_hour_and_the_hour_form_does_not_wrap() {
    // 12:30:45
    assert!(check("timeRange(12, 'GMT')"));
    assert!(check("timeRange(9, 12, 'GMT')"));
    assert!(check("timeRange(12, 12, 'GMT')"));
    assert!(!check("timeRange(13, 17, 'GMT')"));
    assert!(!check("timeRange(22, 6, 'GMT')"));
    assert!(check("timeRange(12, 0, 12, 30, 'GMT')"));
    assert!(check("timeRange(12, 30, 12, 30, 'GMT')"));
    assert!(check("timeRange(22, 0, 13, 0, 'GMT')"));
    assert!(check("timeRange(12, 30, 0, 12, 30, 45, 'GMT')"));
}

#[test]
fn date_range_reads_days_months_and_years_and_a_day_with_one_other_value_stays_in_the_month() {
    // 2026-11-18
    assert!(check("dateRange(18, 'GMT')"));
    assert!(check("dateRange('NOV', 'GMT')"));
    assert!(check("dateRange(2026, 'GMT')"));
    assert!(check("dateRange(1, 'NOV', 'GMT')"));
    assert!(check("dateRange(10, 20, 'GMT')"));
    assert!(!check("dateRange(19, 25, 'GMT')"));
    assert!(check("dateRange('OCT', 'DEC', 'GMT')"));
    assert!(!check("dateRange('JAN', 'MAR', 'GMT')"));
    assert!(check("dateRange(15, 'NOV', 20, 'DEC', 'GMT')"));
    assert!(check("dateRange('NOV', 2026, 'MAR', 2027, 'GMT')"));
    assert!(!check("dateRange('BOGUS', 'GMT')"));
    // A wrapping range: from 20 December through 10 January does not include 18 November.
    assert!(check("dateRange(20, 'DEC', 10, 'JAN', 'GMT') === false"));
    assert!(check("dateRange('NOV', 'OCT', 'GMT')"));
}

#[test]
fn the_local_clock_forms_run_without_a_fixed_clock() {
    let pac = PacScript::builder(
        "function FindProxyForURL(url, host) {
           return (weekdayRange('SUN', 'SAT') && dateRange(1, 31) && timeRange(0, 23)) ? 'DIRECT' : 'PROXY x:1';
         }",
    )
    .build()
    .unwrap();
    assert_eq!(pac.find_proxy_for_url("http://a/", "a").unwrap(), "DIRECT");
}
