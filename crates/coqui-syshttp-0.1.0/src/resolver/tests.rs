use super::*;
use crate::bypass::Dialect;
use crate::system::{SettingsSource, SettingsState};

fn proxy(text: &str) -> Proxy {
    Proxy::parse(text).unwrap()
}

fn hosts(routes: &[Route]) -> Vec<String> {
    routes
        .iter()
        .map(|route| match route {
            Route::Proxy(proxy) => proxy.host().to_string(),
            Route::Direct => "direct".to_string(),
        })
        .collect()
}

fn isolated() -> ResolverBuilder {
    Resolver::builder()
        .environment(Environment::from_pairs([("PATH", "x")]))
        .settings(SystemSettings::default())
}

fn manual(proxies: &str) -> SystemSettings {
    SystemSettings {
        source: SettingsSource::Windows,
        state: SettingsState::Configured,
        proxies: crate::system::parse_proxy_list(proxies, ProxyScheme::Socks4),
        ..SystemSettings::default()
    }
}

#[test]
fn with_no_environment_and_no_settings_everything_goes_direct() {
    let resolver = isolated().build();
    assert_eq!(
        resolver.lookup("https://example.com/").unwrap(),
        [Route::Direct]
    );
}

#[test]
fn an_explicit_proxy_replaces_the_environment_and_the_system_but_not_no_proxy() {
    let environment = Environment::from_pairs([
        ("HTTPS_PROXY", "http://env:1"),
        ("NO_PROXY", "internal.example"),
    ]);
    let resolver = Resolver::builder()
        .environment(environment)
        .settings(manual("sys:2"))
        .proxy(ProxySetting::Proxy(proxy("http://chosen:3")))
        .build();
    assert_eq!(
        hosts(&resolver.lookup("https://example.com/").unwrap()),
        ["chosen"]
    );
    assert_eq!(
        resolver.lookup("https://a.internal.example/").unwrap(),
        [Route::Direct]
    );
}

#[test]
fn a_bypass_list_given_to_the_builder_replaces_no_proxy_from_the_environment() {
    let environment = Environment::from_pairs([
        ("HTTPS_PROXY", "http://env:1"),
        ("NO_PROXY", "env-bypass.example"),
    ]);
    let resolver = Resolver::builder()
        .environment(environment)
        .no_proxy(BypassRules::parse("given.example", Dialect::NoProxy))
        .build();
    assert_eq!(
        resolver.lookup("https://given.example/").unwrap(),
        [Route::Direct]
    );
    assert_eq!(
        hosts(&resolver.lookup("https://env-bypass.example/").unwrap()),
        ["env"]
    );
}

#[test]
fn proxy_variables_decide_alone_and_a_scheme_without_one_goes_direct() {
    let environment = Environment::from_pairs([("https_proxy", "http://secure:1")]);
    let resolver = Resolver::builder()
        .environment(environment)
        .settings(manual("sys:2"))
        .build();
    assert_eq!(
        hosts(&resolver.lookup("https://example.com/").unwrap()),
        ["secure"]
    );
    assert_eq!(
        resolver.lookup("http://example.com/").unwrap(),
        [Route::Direct]
    );
}

#[test]
fn a_variable_may_hold_a_pac_result_with_failover() {
    let environment = Environment::from_pairs([("HTTP_PROXY", "PROXY a:1; PROXY b:2; DIRECT")]);
    let resolver = Resolver::builder().environment(environment).build();
    assert_eq!(
        hosts(&resolver.lookup("http://example.com/").unwrap()),
        ["a", "b", "direct"]
    );
}

#[test]
fn the_systems_manual_proxies_answer_by_scheme_with_its_bypass_list() {
    let mut settings = manual("http=plain:80;https=secure:443");
    settings.bypass = BypassRules::parse("corp.example", Dialect::WinInet);
    let resolver = Resolver::builder()
        .environment(Environment::default())
        .settings(settings)
        .build();
    assert_eq!(
        hosts(&resolver.lookup("https://example.com/").unwrap()),
        ["secure"]
    );
    assert_eq!(
        hosts(&resolver.lookup("http://example.com/").unwrap()),
        ["plain"]
    );
    assert_eq!(
        resolver.lookup("https://corp.example/").unwrap(),
        [Route::Direct]
    );
    assert_eq!(
        resolver.lookup("https://localhost/").unwrap(),
        [Route::Direct]
    );
}

#[test]
fn settings_that_say_direct_send_everything_direct_even_with_a_pac_url() {
    let mut settings = manual("sys:2");
    settings.state = SettingsState::Direct;
    settings.pac_url = Some("file:///nowhere.pac".into());
    let resolver = Resolver::builder()
        .environment(Environment::default())
        .settings(settings)
        .build();
    assert_eq!(
        resolver.lookup("https://example.com/").unwrap(),
        [Route::Direct]
    );
}

#[test]
fn credentials_are_given_to_http_proxies_without_their_own_and_never_to_socks() {
    let environment =
        Environment::from_pairs([("ALL_PROXY", "PROXY a:1; PROXY u:p@b:2; SOCKS5 s:3")]);
    let resolver = Resolver::builder()
        .environment(environment)
        .credentials(Credentials::new("user", "secret"))
        .build();
    let routes = resolver.lookup("http://example.com/").unwrap();
    let login = |index: usize| match &routes[index] {
        Route::Proxy(proxy) => proxy.credentials().map(|c| c.username().to_string()),
        Route::Direct => None,
    };
    assert_eq!(login(0).as_deref(), Some("user"));
    assert_eq!(login(1).as_deref(), Some("u"));
    assert_eq!(login(2), None);
}

#[test]
fn a_proxy_that_refused_a_connection_is_tried_last() {
    let environment = Environment::from_pairs([("HTTP_PROXY", "PROXY a:1; PROXY b:2; DIRECT")]);
    let resolver = Resolver::builder().environment(environment).build();
    resolver.report_failure(&proxy("a:1"));
    assert_eq!(
        hosts(&resolver.lookup("http://example.com/").unwrap()),
        ["b", "direct", "a"]
    );
    resolver.clear();
    assert_eq!(
        hosts(&resolver.lookup("http://example.com/").unwrap()),
        ["a", "b", "direct"]
    );
}

#[test]
fn a_url_without_a_host_is_an_error() {
    assert!(isolated().build().lookup("not a url").is_err());
}

#[cfg(feature = "pac")]
mod with_pac {
    use super::*;

    const PAC: &str = "function FindProxyForURL(url, host) {
        if (shExpMatch(host, '*.corp.example')) return 'PROXY gateway.corp:3128';
        return 'DIRECT';
    }";

    #[test]
    fn a_pac_file_in_the_systems_settings_chooses_per_host() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxy.pac");
        std::fs::write(&path, PAC).unwrap();
        let mut settings = manual("fallback:9");
        settings.pac_url = Some(path.to_string_lossy().into_owned());
        let resolver = Resolver::builder()
            .environment(Environment::default())
            .settings(settings)
            .build();
        assert_eq!(
            hosts(&resolver.lookup("https://wiki.corp.example/").unwrap()),
            ["gateway.corp"]
        );
        assert_eq!(
            resolver.lookup("https://example.org/").unwrap(),
            [Route::Direct]
        );
    }

    #[test]
    fn a_pac_that_cannot_be_loaded_passes_to_the_manual_proxies() {
        let mut settings = manual("fallback:9");
        settings.pac_url = Some("file:///no/such/proxy.pac".into());
        let resolver = Resolver::builder()
            .environment(Environment::default())
            .settings(settings)
            .build();
        assert_eq!(
            hosts(&resolver.lookup("https://example.org/").unwrap()),
            ["fallback"]
        );
    }

    #[test]
    fn an_explicit_pac_script_text_in_a_variable_is_run() {
        let environment = Environment::from_pairs([("HTTPS_PROXY", PAC)]);
        let resolver = Resolver::builder().environment(environment).build();
        assert_eq!(
            hosts(&resolver.lookup("https://wiki.corp.example/").unwrap()),
            ["gateway.corp"]
        );
    }

    #[test]
    fn a_pac_location_in_a_variable_is_loaded_on_first_use() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxy.pac");
        std::fs::write(&path, PAC).unwrap();
        let location = alloc::format!("file:{}", path.to_string_lossy().replace('\\', "/"));
        let environment = Environment::from_pairs([("HTTPS_PROXY", location)]);
        let resolver = Resolver::builder().environment(environment).build();
        assert_eq!(
            hosts(&resolver.lookup("https://wiki.corp.example/").unwrap()),
            ["gateway.corp"]
        );
    }
}

#[cfg(not(feature = "pac"))]
mod without_pac {
    use super::*;

    #[test]
    fn a_pac_location_without_an_engine_is_an_error_or_direct_by_setting() {
        let environment = Environment::from_pairs([("HTTPS_PROXY", "http://wpad.corp/wpad.dat")]);
        let resolver = Resolver::builder().environment(environment.clone()).build();
        assert!(matches!(
            resolver.lookup("https://example.com/"),
            Err(Error::MissingEngine)
        ));
        let resolver = Resolver::builder()
            .environment(environment)
            .on_missing_engine(OnMissingEngine::Direct)
            .build();
        assert_eq!(
            resolver.lookup("https://example.com/").unwrap(),
            [Route::Direct]
        );
    }
}

mod child_environment {
    use super::*;

    fn value(environment: &ChildEnvironment, name: &str) -> Option<String> {
        environment
            .pairs()
            .into_iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    }

    fn system(settings: SystemSettings) -> ResolverBuilder {
        Resolver::builder()
            .environment(Environment::from_pairs([("PATH", "x")]))
            .settings(settings)
    }

    #[test]
    fn nothing_is_set_when_everything_goes_direct() {
        let environment = isolated().build().child_environment(["https://a.example/"]);
        let environment = environment.unwrap();
        assert!(environment.pairs().is_empty());
        assert!(!environment.per_endpoint());
    }

    #[test]
    fn the_systems_manual_proxies_and_bypass_list_become_variables() {
        let settings = SystemSettings {
            bypass: BypassRules::parse(
                "*.corp.example;10.0.0.0/8;<local>;intranet.example",
                Dialect::WinInet,
            ),
            ..manual("http=web:3128;https=secure:3129")
        };
        let environment = system(settings)
            .build()
            .child_environment(Vec::<&str>::new())
            .unwrap();
        assert_eq!(
            value(&environment, "HTTP_PROXY").unwrap(),
            "http://web:3128"
        );
        assert_eq!(
            value(&environment, "HTTPS_PROXY").unwrap(),
            "http://secure:3129"
        );
        assert_eq!(
            environment.no_proxy(),
            [
                "localhost",
                "127.0.0.1",
                "::1",
                "corp.example",
                "10.0.0.0/8",
                "intranet.example"
            ]
        );
        assert_eq!(environment.unexpressed(), ["<local>"]);
        assert!(!environment.per_endpoint());
        let names: Vec<_> = environment
            .pairs()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names.contains(&"http_proxy".to_string()), !cfg!(windows));
    }

    #[test]
    fn a_login_goes_into_the_url_percent_encoded() {
        let resolver = system(manual("web:3128"))
            .credentials(crate::Credentials::new("DOM\\user", "p@ss:word"))
            .build();
        let environment = resolver.child_environment(["https://a.example/"]).unwrap();
        assert_eq!(
            value(&environment, "HTTPS_PROXY").unwrap(),
            "http://DOM%5Cuser:p%40ss%3Aword@web:3128"
        );
        assert!(!format!("{environment:?}").contains("p@ss"));
    }

    #[test]
    fn an_explicit_proxy_and_its_bypass_list_are_passed_on() {
        let resolver = isolated()
            .proxy(ProxySetting::Proxy(proxy("http://chosen:3")))
            .no_proxy(BypassRules::parse("internal.example", Dialect::NoProxy))
            .build();
        let environment = resolver.child_environment(Vec::<&str>::new()).unwrap();
        assert_eq!(
            value(&environment, "HTTP_PROXY").unwrap(),
            "http://chosen:3"
        );
        assert_eq!(
            value(&environment, "NO_PROXY").unwrap(),
            "localhost,127.0.0.1,::1,internal.example"
        );
    }

    #[test]
    fn loopback_is_left_out_when_the_list_turns_it_off() {
        let resolver = isolated()
            .proxy(ProxySetting::Proxy(proxy("http://chosen:3")))
            .no_proxy(BypassRules::parse(
                "<-loopback>,a.example",
                Dialect::NoProxy,
            ))
            .build();
        let environment = resolver.child_environment(Vec::<&str>::new()).unwrap();
        assert_eq!(environment.no_proxy(), ["a.example"]);
    }

    #[test]
    fn a_variable_with_a_pac_result_becomes_the_proxy_it_names() {
        let environment = Environment::from_pairs([
            ("HTTPS_PROXY", "PROXY gateway:8080; DIRECT"),
            ("NO_PROXY", "corp.example"),
        ]);
        let resolver = Resolver::builder().environment(environment).build();
        let environment = resolver.child_environment(Vec::<&str>::new()).unwrap();
        assert_eq!(
            value(&environment, "HTTPS_PROXY").unwrap(),
            "http://gateway:8080"
        );
        assert!(value(&environment, "HTTP_PROXY").is_none());
        assert!(environment.no_proxy().contains(&"corp.example".to_string()));
    }

    #[test]
    fn an_endpoint_a_bypass_rule_without_a_variable_form_covers_is_listed_by_host() {
        let settings = SystemSettings {
            bypass: BypassRules::parse("<local>", Dialect::WinInet),
            ..manual("web:3128")
        };
        let environment = system(settings)
            .build()
            .child_environment(["https://intranet/api", "https://public.example/"])
            .unwrap();
        assert!(environment.no_proxy().contains(&"intranet".to_string()));
        assert!(
            !environment
                .no_proxy()
                .contains(&"public.example".to_string())
        );
        assert!(environment.unexpressed().contains(&"<local>".to_string()));
    }

    #[test]
    fn a_socks_proxy_for_every_scheme_is_written_as_all_proxy_with_remote_names() {
        let environment = system(manual("socks=gate:1080"))
            .build()
            .child_environment(["https://a.example/"])
            .unwrap();
        assert!(environment.unexpressed().is_empty());
        assert_eq!(environment.socks_proxy().unwrap().port(), 1080);
        let pairs = environment.pairs();
        assert!(pairs.contains(&("ALL_PROXY".to_string(), "socks4a://gate:1080".to_string())));
        assert!(pairs.iter().any(|(name, _)| name == "NO_PROXY"));
        assert!(environment.http_proxy().is_none());
    }

    #[test]
    fn socks5_is_written_as_socks5h() {
        let environment = isolated()
            .environment(Environment::from_pairs([("ALL_PROXY", "SOCKS5 gate:1080")]))
            .build()
            .child_environment(["https://a.example/"])
            .unwrap();
        let pairs = environment.pairs();
        let (_, value) = pairs.iter().find(|(name, _)| name == "ALL_PROXY").unwrap();
        assert_eq!(value, "socks5h://gate:1080");
    }

    #[test]
    fn a_socks_proxy_beside_a_direct_scheme_cannot_be_written_and_is_said_so() {
        let environment = isolated()
            .environment(Environment::from_pairs([(
                "HTTPS_PROXY",
                "SOCKS5 gate:1080",
            )]))
            .build()
            .child_environment(["https://a.example/"])
            .unwrap();
        assert!(environment.pairs().is_empty());
        assert!(!environment.unexpressed().is_empty());
    }

    #[test]
    fn a_list_naming_the_hosts_that_use_the_proxy_is_resolved_for_the_endpoints() {
        let settings = SystemSettings {
            bypass: BypassRules::parse("corp.example", Dialect::KdeReversed),
            ..manual("web:3128")
        };
        let environment = system(settings)
            .build()
            .child_environment(["https://api.corp.example/", "https://other.example/"])
            .unwrap();
        assert!(environment.per_endpoint());
        assert_eq!(
            value(&environment, "HTTPS_PROXY").unwrap(),
            "http://web:3128"
        );
        assert!(
            environment
                .no_proxy()
                .contains(&"other.example".to_string())
        );
    }

    #[test]
    fn an_endpoint_that_is_not_a_url_is_an_error() {
        assert!(isolated().build().child_environment(["not a url"]).is_err());
    }

    #[cfg(feature = "pac")]
    mod with_pac {
        use super::*;

        const PAC: &str = "function FindProxyForURL(url, host) {
            if (host == 'direct.example') return 'DIRECT';
            if (host == 'other.example') return 'PROXY second:3128';
            return 'PROXY first:3128';
        }";

        fn pac_settings() -> SystemSettings {
            SystemSettings::builder(SettingsSource::Windows, SettingsState::Configured)
                .pac_script(PAC)
                .build()
        }

        #[test]
        fn a_pac_script_is_asked_about_each_endpoint() {
            let environment = system(pac_settings())
                .build()
                .child_environment([
                    "https://api.example/",
                    "https://direct.example/",
                    "https://other.example/",
                    "http://plain.example/",
                ])
                .unwrap();
            assert!(environment.per_endpoint());
            assert_eq!(
                value(&environment, "HTTPS_PROXY").unwrap(),
                "http://first:3128"
            );
            assert_eq!(
                value(&environment, "HTTP_PROXY").unwrap(),
                "http://first:3128"
            );
            assert!(
                environment
                    .no_proxy()
                    .contains(&"direct.example".to_string())
            );
            assert_eq!(environment.unexpressed(), ["https://other.example/"]);
        }

        #[test]
        fn a_pac_script_with_no_endpoints_sets_nothing() {
            let environment = system(pac_settings())
                .build()
                .child_environment(Vec::<&str>::new())
                .unwrap();
            assert!(environment.pairs().is_empty());
            assert!(environment.per_endpoint());
        }
    }
}
