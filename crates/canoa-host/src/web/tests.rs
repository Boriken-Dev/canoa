#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::BTreeMap;
use std::net::Shutdown;
use std::time::Duration;

use super::*;

const PEER: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn site() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("site")
}

fn owned(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

#[test]
fn a_plain_path_splits_into_its_segments() {
    assert_eq!(segments("/"), Some(Vec::new()));
    assert_eq!(
        segments("/assets/app.js"),
        Some(owned(&["assets", "app.js"]))
    );
    assert_eq!(segments("/a%20b/c.js?v=1#x"), Some(owned(&["a b", "c.js"])));
    assert_eq!(segments("/assets/"), Some(owned(&["assets"])));
    assert_eq!(segments("/caf%C3%A9.txt"), Some(owned(&["caf\u{e9}.txt"])));
}

#[test]
fn a_path_that_could_step_outside_the_site_is_refused() {
    for target in [
        "relative.js",
        "http://example.com/x",
        "/../x",
        "/a/../../x",
        "/./x",
        "/%2e%2e/x",
        "/a/%2E%2E/x",
        "/..%2fx",
        "/..%2Fx",
        "/..%5cx",
        "/a%5c..%5c..%5cx",
        "/a\\..\\x",
        "//x",
        "/a//b",
        "/C:/Windows/win.ini",
        "/c%3a/x",
        "/x.js:stream",
        "/.git/config",
        "/.well-known/x",
        "/x.",
        "/x%20",
        "/x%00.js",
        "/x%0a",
        "/%ff",
        "/a*b",
        "/CON",
        "/con.txt",
        "/nul",
        "/COM1.js",
        "/lpt9",
        "/aux.html",
        "/CONIN$",
        "/conout$.txt",
        "/CLOCK$",
        "/COM%C2%B9",
        "/lpt%C2%B2.js",
        "/com%C2%B3",
    ] {
        assert_eq!(segments(target), None, "{target}");
    }
}

#[test]
fn an_app_route_has_no_extension() {
    assert!(is_app_route(&[]));
    assert!(is_app_route(&owned(&["sessions", "abc"])));
    assert!(is_app_route(&owned(&["assets"])));
    assert!(!is_app_route(&owned(&["missing.js"])));
    assert!(!is_app_route(&owned(&["a", "favicon.ico"])));
}

#[test]
fn a_type_is_chosen_by_extension() {
    for (name, expected) in [
        ("index.html", "text/html; charset=utf-8"),
        ("main.dart.js", "text/javascript; charset=utf-8"),
        ("bridge.MJS", "text/javascript; charset=utf-8"),
        ("style.css", "text/css; charset=utf-8"),
        ("manifest.json", "application/json"),
        ("main.dart.js.map", "application/json"),
        ("site.webmanifest", "application/manifest+json"),
        ("canvaskit.wasm", "application/wasm"),
        ("icon.png", "image/png"),
        ("logo.svg", "image/svg+xml"),
        ("favicon.ico", "image/x-icon"),
        ("photo.jpg", "image/jpeg"),
        ("photo.jpeg", "image/jpeg"),
        ("pic.gif", "image/gif"),
        ("pic.webp", "image/webp"),
        ("Roboto.woff2", "font/woff2"),
        ("Roboto.woff", "font/woff"),
        ("MaterialIcons-Regular.otf", "font/otf"),
        ("Noto.ttf", "font/ttf"),
        ("LICENSE.txt", "text/plain; charset=utf-8"),
        ("NOTICES", "application/octet-stream"),
        ("AssetManifest.bin", "application/octet-stream"),
        ("ink_sparkle.frag", "application/octet-stream"),
    ] {
        assert_eq!(content_type(name), expected, "{name}");
    }
}

#[test]
fn a_hashed_name_is_one_with_a_hash_after_its_first_part() {
    for name in [
        "main.3f9a1b2c.js",
        "index-3f9a1b2c7d.css",
        "chunk_0123456789abcdef.js",
        "style.3f9a1b2c7d.css",
        "a.b.DEADBEEF.png",
    ] {
        assert!(is_hashed(name), "{name}");
    }
    for name in [
        "main.dart.js",
        "deadbeef.js",
        "app.js",
        "chunk.3f9a1b2.js",
        "index.html",
        "canvaskit.wasm",
        "flutter_bootstrap.js",
        "NOTICES",
        "3f9a1b2c",
        "main.3f9a1b2g.js",
    ] {
        assert!(!is_hashed(name), "{name}");
    }
}

#[test]
fn a_host_header_must_name_this_host() {
    let named = vec!["canoa.example".to_owned()];
    for host in [
        "127.0.0.1",
        "127.0.0.1:7800",
        "192.0.2.10:443",
        "localhost",
        "LocalHost:7800",
        "[::1]",
        "[::1]:7800",
        "[fe80::1]:80",
        "canoa.example",
        "Canoa.Example:443",
    ] {
        assert!(host_allowed(host, &named), "{host}");
    }
    for host in [
        "",
        "evil.example",
        "evil.example:7800",
        "127.0.0.1.evil.example",
        "localhost.evil.example",
        "canoa.example.evil",
        "127.1",
        "127.0.0.1:",
        "127.0.0.1:x",
        "127.0.0.1:123456",
        "[::1",
        "[::1]x",
        "[evil]:80",
        "::1",
        "canoa.example:80:80",
        "local host",
    ] {
        assert!(!host_allowed(host, &named), "{host}");
    }
    assert!(!host_allowed("canoa.example", &[]));
}

#[test]
fn a_date_is_written_as_http_writes_one() {
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(784_111_777);
    assert_eq!(http_date(time), "Sun, 06 Nov 1994 08:49:37 GMT");
    let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(951_782_400);
    assert_eq!(http_date(leap), "Tue, 29 Feb 2000 00:00:00 GMT");
    assert_eq!(
        http_date(SystemTime::UNIX_EPOCH),
        "Thu, 01 Jan 1970 00:00:00 GMT"
    );
}

#[test]
fn the_policy_lets_the_page_connect_to_this_hosts_entries_alone() {
    let policy = policy(&["ws://127.0.0.1:7801", "ws://127.0.0.1:7802"]);
    let directives: BTreeMap<&str, &str> = policy
        .split(';')
        .filter_map(|directive| directive.trim().split_once(' '))
        .collect();
    assert_eq!(directives["default-src"], "'none'");
    assert_eq!(
        directives["connect-src"],
        "'self' ws://127.0.0.1:7801 ws://127.0.0.1:7802"
    );
    assert_eq!(directives["script-src"], "'self' 'wasm-unsafe-eval'");
    assert_eq!(directives["frame-ancestors"], "'none'");
    assert_eq!(directives["object-src"], "'none'");
    assert_eq!(directives["base-uri"], "'self'");
    assert_eq!(directives["form-action"], "'none'");
    assert!(!policy.contains('*'), "{policy}");
    assert!(!policy.contains("'unsafe-eval'"), "{policy}");
    assert!(!policy.contains("https:"), "{policy}");
}

#[test]
fn a_source_the_settings_add_is_allowed_besides_the_entries() {
    let entries = [Listening {
        name: "echo".to_owned(),
        kind: "agent",
        url: "ws://127.0.0.1:7801".to_owned(),
    }];
    let mut config = loopback(&site());
    config.connect = vec!["https://fonts.gstatic.com".to_owned()];
    let address = running(&config, &entries);
    let policy = get(address, "/").headers["content-security-policy"].clone();
    assert!(
        policy.contains("connect-src 'self' ws://127.0.0.1:7801 https://fonts.gstatic.com;"),
        "{policy}"
    );
}

/// A server on `config`'s site, on a loopback port of its own, taking connections on a thread.
fn running(config: &WebConfig, entries: &[Listening]) -> SocketAddr {
    let server = WebServer::bind_listing(config, PEER, "noise", entries).expect("it binds");
    let address = server.local_addr();
    std::thread::spawn(move || server.run());
    address
}

fn loopback(dir: &Path) -> WebConfig {
    let mut config = WebConfig::new();
    config.dir = Some(dir.to_owned());
    config.listen = "127.0.0.1:0".parse().expect("an address");
    config
}

/// An answer as read off the wire: its status, its headers by lower-case name, and its body.
#[derive(Debug)]
struct Reply {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// One answer at the start of `bytes`, and where it ends; `bodiless` for the answer to a `HEAD`.
fn parse_reply(bytes: &[u8], bodiless: bool) -> Option<(Reply, usize)> {
    let end = bytes.windows(4).position(|window| window == b"\r\n\r\n")? + 4;
    let head = core::str::from_utf8(&bytes[..end]).expect("a head in ASCII");
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split(' ').nth(1)?.parse().ok()?;
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = match headers.get("content-length") {
        _ if bodiless || status == 304 => 0,
        Some(length) => length.parse().ok()?,
        None => 0,
    };
    Some((
        Reply {
            status,
            headers,
            body: Vec::new(),
        },
        end + length,
    ))
}

/// Sends `request` as written and reads every byte until the server closes.
fn exchange_raw(address: SocketAddr, request: &[u8]) -> Vec<u8> {
    let mut tcp = TcpStream::connect(address).expect("it connects");
    tcp.set_read_timeout(Some(Duration::from_secs(20)))
        .expect("a timeout");
    tcp.write_all(request).expect("the request is sent");
    let mut received = Vec::new();
    let _ = tcp.read_to_end(&mut received);
    received
}

/// The replies to `request`, read until the server closes.
fn replies(address: SocketAddr, request: &[u8]) -> Vec<Reply> {
    let received = exchange_raw(address, request);
    // The requests here carry no body, so each one's method starts a line after an empty one.
    let heads: Vec<bool> = request
        .split(|byte| *byte == b'\n')
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|pair| pair[0] == b"\r" || pair[0].is_empty())
        .map(|pair| pair[1].starts_with(b"HEAD "))
        .collect();
    let mut heads = core::iter::once(request.starts_with(b"HEAD ")).chain(heads);
    let mut at = 0;
    let mut replies = Vec::new();
    while let Some((mut reply, end)) = parse_reply(&received[at..], heads.next().unwrap_or(false)) {
        let head_end = received[at..]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("a head")
            + 4;
        let end = end.min(received.len() - at);
        reply.body = received[at + head_end..at + end].to_vec();
        replies.push(reply);
        at += end;
    }
    replies
}

fn get(address: SocketAddr, path: &str) -> Reply {
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    replies(address, request.as_bytes())
        .into_iter()
        .next()
        .expect("an answer")
}

#[test]
fn files_are_served_from_the_directory_with_their_types_and_headers() {
    let address = running(
        &loopback(&site()),
        &[Listening {
            name: "echo".to_owned(),
            kind: "agent",
            url: "ws://127.0.0.1:7801".to_owned(),
        }],
    );
    let index = get(address, "/");
    assert_eq!(index.status, 200);
    assert_eq!(
        index.body,
        std::fs::read(site().join("index.html")).expect("the fixture")
    );
    assert_eq!(index.headers["content-type"], "text/html; charset=utf-8");
    assert_eq!(index.headers["cache-control"], "no-cache");
    assert!(
        index.headers["content-security-policy"]
            .contains("connect-src 'self' ws://127.0.0.1:7801;"),
        "{:?}",
        index.headers
    );
    assert_eq!(index.headers["x-content-type-options"], "nosniff");
    assert_eq!(index.headers["referrer-policy"], "no-referrer");
    assert_eq!(index.headers["x-frame-options"], "DENY");
    assert_eq!(index.headers["cross-origin-opener-policy"], "same-origin");
    assert!(index.headers["date"].ends_with(" GMT"));

    let script = get(address, "/app.js");
    assert_eq!(script.status, 200);
    assert_eq!(
        script.headers["content-type"],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        script.body,
        std::fs::read(site().join("app.js")).expect("the fixture")
    );

    let hashed = get(address, "/assets/style.3f9a1b2c7d.css");
    assert_eq!(hashed.status, 200);
    assert_eq!(hashed.headers["content-type"], "text/css; charset=utf-8");
    assert_eq!(
        hashed.headers["cache-control"],
        "public, max-age=31536000, immutable"
    );

    let wasm = get(address, "/data/module.wasm");
    assert_eq!(wasm.headers["content-type"], "application/wasm");
    assert_eq!(wasm.body, [0, 97, 115, 109, 1, 0, 0, 0]);
}

#[test]
fn head_answers_without_a_body_and_a_kept_connection_serves_several() {
    let address = running(&loopback(&site()), &[]);
    let replies = replies(
        address,
        b"HEAD /app.js HTTP/1.1\r\nHost: localhost\r\n\r\nGET /app.js HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(replies.len(), 2, "{replies:?}");
    let length = std::fs::read(site().join("app.js"))
        .expect("the fixture")
        .len();
    assert_eq!(replies[0].status, 200);
    assert_eq!(replies[0].headers["content-length"], length.to_string());
    assert!(replies[0].body.is_empty());
    assert_eq!(replies[1].body.len(), length);
}

#[test]
fn a_file_whose_tag_the_browser_holds_is_not_sent_again() {
    let address = running(&loopback(&site()), &[]);
    let first = get(address, "/app.js");
    let tag = &first.headers["etag"];
    let request = format!(
        "GET /app.js HTTP/1.1\r\nHost: 127.0.0.1\r\nIf-None-Match: \"other\", {tag}\r\nConnection: close\r\n\r\n"
    );
    let again = replies(address, request.as_bytes())
        .into_iter()
        .next()
        .expect("an answer");
    assert_eq!(again.status, 304);
    assert!(again.body.is_empty());
    assert_eq!(&again.headers["etag"], tag);
}

#[test]
fn an_app_route_falls_back_to_the_index_and_a_missing_file_does_not() {
    let address = running(&loopback(&site()), &[]);
    let index = std::fs::read(site().join("index.html")).expect("the fixture");
    for route in ["/sessions/abc", "/settings", "/assets/", "/data"] {
        let reply = get(address, route);
        assert_eq!(reply.status, 200, "{route}");
        assert_eq!(reply.body, index, "{route}: no listing, the app");
        assert_eq!(reply.headers["cache-control"], "no-cache", "{route}");
        assert_eq!(
            reply.headers["content-type"], "text/html; charset=utf-8",
            "{route}"
        );
    }
    for missing in ["/missing.js", "/assets/gone.css", "/data/info.json.bak"] {
        assert_eq!(get(address, missing).status, 404, "{missing}");
    }
}

#[test]
fn the_host_index_lists_the_entries_it_was_given() {
    let entries = [
        Listening {
            name: "echo".to_owned(),
            kind: "agent",
            url: "ws://127.0.0.1:7801".to_owned(),
        },
        Listening {
            name: "py".to_owned(),
            kind: "provider",
            url: "ws://127.0.0.1:7802".to_owned(),
        },
    ];
    let address = running(&loopback(&site()), &entries);
    let reply = get(address, "/canoa.json?x=1");
    assert_eq!(reply.status, 200);
    assert_eq!(reply.headers["content-type"], "application/json");
    assert_eq!(reply.headers["cache-control"], "no-store");
    let listed: serde_json::Value = serde_json::from_slice(&reply.body).expect("JSON");
    assert_eq!(
        listed,
        serde_json::json!({
            "peer": PEER,
            "method": "noise",
            "entries": [
                {"name": "echo", "kind": "agent", "url": "ws://127.0.0.1:7801"},
                {"name": "py", "kind": "provider", "url": "ws://127.0.0.1:7802"},
            ],
        })
    );
}

/// A site in a directory of the test's own, beside a file and a directory outside it.
struct Outside {
    dir: PathBuf,
}

impl Outside {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("canoa-web-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("site")).expect("a site");
        std::fs::create_dir_all(dir.join("outside")).expect("a directory outside it");
        std::fs::write(dir.join("site").join("index.html"), "INDEX").expect("an index");
        std::fs::write(dir.join("secret.txt"), "SECRET-OUTSIDE").expect("a file outside");
        std::fs::write(dir.join("outside").join("inner.txt"), "SECRET-INNER")
            .expect("a file in the directory outside");
        Self { dir }
    }

    fn site(&self) -> PathBuf {
        self.dir.join("site")
    }
}

impl Drop for Outside {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn an_escape_is_refused_however_it_is_spelled() {
    let outside = Outside::new("escape");
    let address = running(&loopback(&outside.site()), &[]);
    for target in [
        "/../secret.txt",
        "/%2e%2e/secret.txt",
        "/..%2fsecret.txt",
        "/..%5csecret.txt",
        "/x/..%5c..%5csecret.txt",
        "/../outside/inner.txt",
    ] {
        let reply = get(address, target);
        assert_eq!(reply.status, 404, "{target}");
        let body = String::from_utf8_lossy(&reply.body);
        assert!(!body.contains("SECRET"), "{target}: {body}");
    }
}

/// Makes `link` lead to the directory `target`: a symbolic link, or on Windows a junction, which needs no privilege.
fn link_dir(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|done| done.status.success())
    }
}

/// Makes `link` lead to the file `target`, where this user may: Windows asks for Developer Mode or an administrator.
fn link_file(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
}

#[test]
fn a_link_out_of_the_site_is_refused_and_one_inside_is_followed() {
    let outside = Outside::new("links");
    let site = outside.site();
    std::fs::create_dir_all(site.join("inside")).expect("a directory inside");
    std::fs::write(site.join("inside").join("ok.txt"), "INSIDE").expect("a file inside");
    assert!(
        link_dir(&outside.dir.join("outside"), &site.join("out")),
        "a directory link can be made"
    );
    assert!(
        link_dir(&site.join("inside"), &site.join("in")),
        "a directory link can be made"
    );
    let file_link = link_file(&outside.dir.join("secret.txt"), &site.join("secret.txt"));
    let address = running(&loopback(&site), &[]);

    let refused = get(address, "/out/inner.txt");
    assert_eq!(refused.status, 404);
    assert!(!String::from_utf8_lossy(&refused.body).contains("SECRET"));
    let route = get(address, "/out");
    assert_ne!(
        String::from_utf8_lossy(&route.body),
        "SECRET-INNER",
        "a link out answers nothing of what it leads to"
    );
    let followed = get(address, "/in/ok.txt");
    assert_eq!(followed.status, 200);
    assert_eq!(followed.body, b"INSIDE");
    if file_link {
        let refused = get(address, "/secret.txt");
        assert_eq!(refused.status, 404);
        assert!(!String::from_utf8_lossy(&refused.body).contains("SECRET"));
    } else {
        println!("skipped: a link to a file, which this user may not make here");
    }
}

#[test]
fn a_request_for_another_host_is_refused() {
    let address = running(&loopback(&site()), &[]);
    let reply = replies(
        address,
        b"GET / HTTP/1.1\r\nHost: evil.example\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(reply[0].status, 421);
    let mut named = loopback(&site());
    named.hosts = vec!["canoa.example".to_owned()];
    let address = running(&named, &[]);
    let reply = replies(
        address,
        b"GET / HTTP/1.1\r\nHost: canoa.example:7800\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(reply[0].status, 200);
    let missing = replies(address, b"GET / HTTP/1.1\r\nConnection: close\r\n\r\n");
    assert_eq!(missing[0].status, 400, "HTTP/1.1 needs a Host");
    let old = replies(address, b"GET / HTTP/1.0\r\n\r\n");
    assert_eq!(
        old[0].status, 400,
        "so does HTTP/1.0, or the check is skipped"
    );
    let old = replies(address, b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n");
    assert_eq!(old[0].status, 200);
    let twice = replies(
        address,
        b"GET / HTTP/1.1\r\nHost: localhost\r\nHost: evil.example\r\n\r\n",
    );
    assert_eq!(twice[0].status, 400);
}

#[test]
fn only_get_and_head_without_a_body_are_answered() {
    let address = running(&loopback(&site()), &[]);
    for method in ["POST", "PUT", "DELETE", "OPTIONS", "PATCH", "TRACE"] {
        let request =
            format!("{method} / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n");
        let reply = replies(address, request.as_bytes());
        assert_eq!(reply[0].status, 405, "{method}");
        assert_eq!(reply[0].headers["allow"], "GET, HEAD", "{method}");
    }
    let bodied = replies(
        address,
        b"GET / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nhello",
    );
    assert_eq!(bodied[0].status, 400);
    let chunked = replies(
        address,
        b"GET / HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
    );
    assert_eq!(chunked[0].status, 400);
    let twice = replies(
        address,
        b"GET / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n",
    );
    assert_eq!(twice[0].status, 400, "two lengths are one too many");
    let absolute = replies(
        address,
        b"GET http://localhost/ HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(absolute[0].status, 400);
    let garbage = replies(
        address,
        b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03\r\n\r\n",
    );
    assert_eq!(garbage[0].status, 400);
}

#[test]
fn a_request_past_the_limits_is_refused() {
    let mut config = loopback(&site());
    config.limits = Limits::default().with_max_request_bytes(1024);
    let address = running(&config, &[]);
    let long = format!(
        "GET / HTTP/1.1\r\nHost: localhost\r\nX-Long: {}\r\n\r\n",
        "a".repeat(2000)
    );
    assert_eq!(replies(address, long.as_bytes())[0].status, 431);
    let many: String = (0..MAX_HEADERS + 1)
        .map(|index| format!("X-{index}: 1\r\n"))
        .collect();
    let many = format!("GET / HTTP/1.1\r\nHost: localhost\r\n{many}\r\n");
    let mut config = loopback(&site());
    config.limits = Limits::default();
    let address = running(&config, &[]);
    assert_eq!(replies(address, many.as_bytes())[0].status, 431);
}

#[test]
fn a_connection_ends_after_its_last_request() {
    let mut config = loopback(&site());
    config.limits = Limits::default().with_max_requests(2);
    let address = running(&config, &[]);
    let three = b"GET /app.js HTTP/1.1\r\nHost: localhost\r\n\r\n".repeat(3);
    let replies = replies(address, &three);
    assert_eq!(replies.len(), 2, "{replies:?}");
    assert_eq!(replies[1].headers["connection"], "close");
}

#[test]
fn a_slow_or_idle_peer_is_dropped_at_its_limit() {
    let mut config = loopback(&site());
    config.limits = Limits::default()
        .with_request_time(Duration::from_millis(500))
        .with_idle_time(Duration::from_millis(500));
    let address = running(&config, &[]);

    let mut slow = TcpStream::connect(address).expect("it connects");
    slow.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    slow.write_all(b"GET / HTTP/1.1\r\nHost: local")
        .expect("part of a request");
    let started = Instant::now();
    let mut received = Vec::new();
    let _ = slow.read_to_end(&mut received);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        String::from_utf8_lossy(&received).starts_with("HTTP/1.1 408 "),
        "{}",
        String::from_utf8_lossy(&received)
    );

    let mut dripping = TcpStream::connect(address).expect("it connects");
    dripping
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    let started = Instant::now();
    for byte in b"GET / HTTP/1.1\r\nHost: localhost\r\n" {
        if dripping.write_all(&[*byte]).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        if started.elapsed() > Duration::from_secs(3) {
            break;
        }
    }
    let mut received = Vec::new();
    let _ = dripping.read_to_end(&mut received);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a request dripped a byte at a time ends at request_time"
    );

    let mut idle = TcpStream::connect(address).expect("it connects");
    idle.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    idle.write_all(b"GET /app.js HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("a request");
    let started = Instant::now();
    let mut received = Vec::new();
    let _ = idle.read_to_end(&mut received);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(String::from_utf8_lossy(&received).starts_with("HTTP/1.1 200 "));
}

#[test]
fn connections_past_the_limit_are_closed_unanswered() {
    let mut config = loopback(&site());
    config.limits = Limits::default().with_max_connections(1);
    let address = running(&config, &[]);
    let held = TcpStream::connect(address).expect("it connects");
    std::thread::sleep(Duration::from_millis(200));
    let mut turned = TcpStream::connect(address).expect("it connects");
    turned
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    let _ = turned.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let mut received = Vec::new();
    let _ = turned.read_to_end(&mut received);
    assert!(
        received.is_empty(),
        "{}",
        String::from_utf8_lossy(&received)
    );
    let _ = held.shutdown(Shutdown::Both);
    drop(held);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(get(address, "/").status, 200);
}

#[test]
fn plain_http_beyond_loopback_is_refused() {
    let mut config = loopback(&site());
    config.listen = "0.0.0.0:0".parse().expect("an address");
    let refused = WebServer::bind_listing(&config, PEER, "noise", &[]).expect_err("refused");
    assert!(
        matches!(refused, WebError::PlainBeyondLoopback(_)),
        "{refused}"
    );
}

#[test]
fn a_site_without_an_index_or_a_limit_of_zero_is_refused() {
    let empty = std::env::temp_dir().join(format!("canoa-web-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty).expect("a directory");
    let refused =
        WebServer::bind_listing(&loopback(&empty), PEER, "noise", &[]).expect_err("refused");
    assert!(matches!(refused, WebError::Site { .. }), "{refused}");
    let _ = std::fs::remove_dir_all(&empty);
    let refused =
        WebServer::bind_listing(&loopback(&empty), PEER, "noise", &[]).expect_err("refused");
    assert!(matches!(refused, WebError::Site { .. }), "{refused}");
    let mut config = loopback(&site());
    config.limits = Limits::default().with_send_time(Duration::ZERO);
    let refused = WebServer::bind_listing(&config, PEER, "noise", &[]).expect_err("refused");
    assert!(
        matches!(refused, WebError::Limit("send_seconds")),
        "{refused}"
    );
}

#[test]
fn a_dot_name_is_refused_however_it_is_reached() {
    let outside = Outside::new("dots");
    let site = outside.site();
    std::fs::create_dir_all(site.join(".git")).expect("a dot directory");
    std::fs::write(site.join(".git").join("config"), "SECRET-GIT").expect("a file in it");
    std::fs::write(site.join(".env.local"), "SECRET-ENV").expect("a dot file");
    assert!(
        link_dir(&site.join(".git"), &site.join("visible")),
        "a directory link can be made"
    );
    let address = running(&loopback(&site), &[]);
    let mut targets = vec!["/visible/config".to_owned()];
    // NTFS gives a dot-name a short name such as GIT~1, which resolves to the dot-name itself.
    for (long, short) in [(".git", "/GIT~1/config"), (".env.local", "/ENV~1.LOC")] {
        let named = site.join(
            short
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or(""),
        );
        if std::fs::metadata(&named).is_ok() {
            targets.push(short.to_owned());
        } else {
            println!("skipped: {long} has no short name here");
        }
    }
    for target in targets {
        let reply = get(address, &target);
        assert_eq!(reply.status, 404, "{target}");
        assert!(
            !String::from_utf8_lossy(&reply.body).contains("SECRET"),
            "{target}"
        );
    }
}

#[test]
fn a_site_holding_a_directory_the_host_keeps_private_is_refused() {
    let outside = Outside::new("private");
    let site = outside.site();
    let config_dir = site.join("conf");
    std::fs::create_dir_all(&config_dir).expect("a directory inside");
    for private in [config_dir.clone(), site.clone()] {
        let mut config = loopback(&site);
        config.keep_out = vec![private.clone()];
        let refused = WebServer::bind_listing(&config, PEER, "noise", &[]).expect_err("refused");
        assert!(
            matches!(refused, WebError::Site { .. }),
            "{}: {refused}",
            private.display()
        );
    }
    let mut config = loopback(&site);
    config.keep_out = vec![outside.dir.clone(), outside.dir.join("missing")];
    assert!(
        WebServer::bind_listing(&config, PEER, "noise", &[]).is_ok(),
        "a site inside one, or beside one, holds none"
    );
}

#[test]
fn a_time_limit_too_long_to_add_is_no_limit_rather_than_a_crash() {
    let mut config = loopback(&site());
    config.limits = Limits::default()
        .with_request_time(Duration::MAX)
        .with_idle_time(Duration::MAX)
        .with_send_time(Duration::MAX)
        .with_connection_time(Duration::MAX);
    let address = running(&config, &[]);
    assert_eq!(get(address, "/").status, 200);
    assert_eq!(get(address, "/app.js").status, 200);
}

#[test]
fn a_connection_ends_at_its_lifetime_however_busy() {
    let mut config = loopback(&site());
    config.limits = Limits::default().with_connection_time(Duration::from_millis(800));
    let address = running(&config, &[]);
    let mut busy = TcpStream::connect(address).expect("it connects");
    busy.set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    let started = Instant::now();
    let mut answered = 0;
    while started.elapsed() < Duration::from_secs(5) {
        if busy
            .write_all(b"GET /app.js HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .is_err()
        {
            break;
        }
        let mut chunk = [0_u8; 4096];
        match busy.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(_) => answered += 1,
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(answered >= 1, "it answered before its lifetime ended");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "kept busy, it still ended at its lifetime"
    );
}

#[test]
fn one_address_cannot_hold_every_connection() {
    let mut config = loopback(&site());
    config.limits = Limits::default().with_max_connections_per_address(1);
    let address = running(&config, &[]);
    let held = TcpStream::connect(address).expect("it connects");
    std::thread::sleep(Duration::from_millis(200));
    let mut turned = TcpStream::connect(address).expect("it connects");
    turned
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("a timeout");
    let _ = turned.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    let mut received = Vec::new();
    let _ = turned.read_to_end(&mut received);
    assert!(
        received.is_empty(),
        "{}",
        String::from_utf8_lossy(&received)
    );
    drop(held);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(get(address, "/").status, 200);
}

#[cfg(unix)]
#[test]
fn a_key_others_may_read_is_one_to_warn_of() {
    use std::os::unix::fs::PermissionsExt as _;
    let outside = Outside::new("key");
    let key = outside.dir.join("web.key");
    std::fs::write(&key, "KEY").expect("a key file");
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).expect("its mode");
    assert!(readable_by_others(&key));
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).expect("its mode");
    assert!(!readable_by_others(&key));
}

#[cfg(not(feature = "web"))]
#[test]
fn without_the_feature_no_directory_is_no_site() {
    const { assert!(!BUILT_IN) };
    let mut config = loopback(&site());
    config.dir = None;
    let refused = WebServer::bind_listing(&config, PEER, "noise", &[]).expect_err("refused");
    assert!(matches!(refused, WebError::NoSite), "{refused}");
}

#[cfg(feature = "web")]
#[test]
fn the_built_in_site_is_every_file_of_the_directory_it_was_built_from() {
    const { assert!(BUILT_IN) };
    let dist = PathBuf::from(env!("CANOA_WEB_DIST_BUILT"));
    let mut config = loopback(&site());
    config.dir = None;
    let address = running(&config, &[]);
    assert!(!BUILT.is_empty());
    assert!(
        BUILT.windows(2).all(|pair| pair[0].0 < pair[1].0),
        "in order"
    );
    for (path, bytes, _) in BUILT {
        let on_disk = std::fs::read(dist.join(path)).expect("each file is in the directory");
        assert_eq!(*bytes, on_disk.as_slice(), "{path}");
        let reply = get(address, &format!("/{path}"));
        assert_eq!(reply.status, 200, "{path}");
        assert_eq!(reply.body, on_disk, "{path}");
    }
    assert_eq!(
        get(address, "/").body,
        std::fs::read(dist.join("index.html")).expect("an index")
    );
}
