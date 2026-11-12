//! The tools a store fetches and unpacks with: every refusal the host makes before a byte reaches a program.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;

use canoa::store::{Check, FetchRequest, FindRequest, InstallTools, UnpackRequest};
use canoa_host::store::tools::{HttpTransport, LocalTools, Lookup};
use coqui_acp_api::guatiao::Status;
use sha2::{Digest, Sha256};

/// A directory removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let mut nonce = [0u8; 4];
        getrandom::fill(&mut nonce).expect("random");
        let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let path = std::env::temp_dir().join(format!("canoa-tools-{name}-{hex}"));
        fs::create_dir_all(&path).expect("a directory");
        Self(path)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A loopback server answering by path: `(status, location, body)`.
fn serve(routes: Vec<(&'static str, u16, Option<String>, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let origin = format!(
        "http://127.0.0.1:{}",
        listener.local_addr().expect("addr").port()
    );
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let routes = routes.clone();
            thread::spawn(move || answer(stream, &routes));
        }
    });
    origin
}

fn answer(mut stream: TcpStream, routes: &[(&'static str, u16, Option<String>, Vec<u8>)]) {
    let mut buffer = [0u8; 2048];
    let Ok(count) = stream.read(&mut buffer) else {
        return;
    };
    let request = String::from_utf8_lossy(&buffer[..count]).into_owned();
    let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let (status, location, body) = routes
        .iter()
        .find(|(p, ..)| *p == path)
        .map_or((404, None, Vec::new()), |(_, s, l, b)| {
            (*s, l.clone(), b.clone())
        });
    let mut head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(location) = location {
        head.push_str(&format!("Location: {location}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&body);
}

fn tools(origin: &str, granted: &Path) -> LocalTools {
    LocalTools::new(Arc::new(HttpTransport::default()), vec![origin.to_owned()])
        .granted(granted)
        .loopback_http(true)
        .allow_unverified(true)
}

fn fetch(
    tools: &LocalTools,
    origin: &str,
    path: &str,
    check: Check,
    max: u64,
    exec: bool,
) -> Result<canoa::store::Fetched, coqui_acp_api::guatiao::library::ProviderError> {
    let mut request = FetchRequest::new(format!("{origin}{path}"), "out.bin", check, max);
    request.executable = exec;
    tools.fetch(request)
}

#[test]
fn a_matching_download_is_kept_and_a_mismatch_is_deleted_and_refused() {
    let body = b"program bytes".to_vec();
    let origin = serve(vec![("/f", 200, None, body.clone())]);
    let dir = Dir::new("fetch");
    let t = tools(&origin, &dir.0);
    let ok = fetch(
        &t,
        &origin,
        "/f",
        Check::Sha256 { value: sha(&body) },
        1000,
        false,
    )
    .expect("verified");
    assert_eq!(ok.sha256, sha(&body));
    assert_eq!(fs::read(dir.0.join("out.bin")).expect("kept"), body);
    fs::remove_file(dir.0.join("out.bin")).expect("clear");
    let error = fetch(
        &t,
        &origin,
        "/f",
        Check::Sha256 {
            value: "00".repeat(32),
        },
        1000,
        false,
    )
    .expect_err("mismatch");
    assert_eq!(error.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(
        error.message().contains("checksum mismatch"),
        "{}",
        error.message()
    );
    assert!(
        !dir.0.join("out.bin").exists(),
        "a mismatched file is deleted"
    );
}

#[test]
fn a_download_over_its_cap_is_stopped_and_deleted() {
    let origin = serve(vec![("/big", 200, None, vec![7u8; 5000])]);
    let dir = Dir::new("cap");
    let error = fetch(
        &tools(&origin, &dir.0),
        &origin,
        "/big",
        Check::Unverified { why: "test".into() },
        1000,
        false,
    )
    .expect_err("capped");
    assert_eq!(error.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(!dir.0.join("out.bin").exists());
}

#[test]
fn a_redirect_to_an_origin_the_store_did_not_declare_is_refused() {
    let other = serve(vec![("/f", 200, None, b"x".to_vec())]);
    let origin = serve(vec![("/f", 302, Some(format!("{other}/f")), Vec::new())]);
    let dir = Dir::new("redirect");
    let error = fetch(
        &tools(&origin, &dir.0),
        &origin,
        "/f",
        Check::Unverified { why: "test".into() },
        1000,
        false,
    )
    .expect_err("refused");
    assert!(
        error.message().contains("not one of the store's endpoints"),
        "{}",
        error.message()
    );
}

#[test]
fn a_redirect_inside_the_declared_origin_is_followed() {
    let body = b"abc".to_vec();
    let origin = serve(vec![
        ("/a", 302, Some("/b".to_owned()), Vec::new()),
        ("/b", 200, None, body.clone()),
    ]);
    let dir = Dir::new("redirect-ok");
    fetch(
        &tools(&origin, &dir.0),
        &origin,
        "/a",
        Check::Sha256 { value: sha(&body) },
        1000,
        false,
    )
    .expect("followed");
}

#[test]
fn plain_http_is_refused_unless_loopback_is_allowed() {
    let origin = serve(vec![("/f", 200, None, b"x".to_vec())]);
    let dir = Dir::new("http");
    let strict = LocalTools::new(Arc::new(HttpTransport::default()), vec![origin.clone()])
        .granted(&dir.0)
        .allow_unverified(true);
    let error = fetch(
        &strict,
        &origin,
        "/f",
        Check::Unverified { why: "test".into() },
        10,
        false,
    )
    .expect_err("http");
    assert!(
        error.message().contains("only https"),
        "{}",
        error.message()
    );
}

#[cfg(unix)]
#[test]
fn the_executable_bit_is_set_only_after_the_check_passes() {
    use std::os::unix::fs::PermissionsExt;
    let body = b"#!/bin/sh\n".to_vec();
    let origin = serve(vec![("/f", 200, None, body.clone())]);
    let dir = Dir::new("exec");
    let t = tools(&origin, &dir.0);
    fetch(
        &t,
        &origin,
        "/f",
        Check::Sha256 { value: sha(&body) },
        100,
        true,
    )
    .expect("ok");
    assert_eq!(
        fs::metadata(dir.0.join("out.bin"))
            .expect("meta")
            .permissions()
            .mode()
            & 0o111,
        0o111
    );
    fs::remove_file(dir.0.join("out.bin")).expect("clear");
    let _ = fetch(
        &t,
        &origin,
        "/f",
        Check::Sha256 {
            value: "11".repeat(32),
        },
        100,
        true,
    );
    assert!(!dir.0.join("out.bin").exists());
}

#[test]
fn a_path_that_leaves_the_granted_folder_is_refused() {
    let origin = serve(vec![("/f", 200, None, b"x".to_vec())]);
    let dir = Dir::new("path");
    let t = tools(&origin, &dir.0);
    for path in ["../x", "a/../../x", "/abs", "C:/x", "con", "a:b"] {
        let request = FetchRequest::new(
            format!("{origin}/f"),
            path,
            Check::Unverified { why: "t".into() },
            10,
        );
        assert!(t.fetch(request).is_err(), "{path} was accepted");
    }
}

fn tar_gz(entries: &[(&str, &[u8], tar::EntryType, Option<&str>)]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(gz);
    for (path, data, kind, link) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(*kind);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        if let Some(link) = link {
            header.set_link_name(link).expect("link");
        }
        // `append_data` refuses a `..` path; the raw name is what a hostile archive carries.
        let name = header.as_old_mut();
        let bytes = path.as_bytes();
        name.name[..bytes.len()].copy_from_slice(bytes);
        header.set_cksum();
        builder.append(&header, *data).expect("entry");
    }
    builder.into_inner().expect("tar").finish().expect("gz")
}

fn unpack(
    dir: &Dir,
    bytes: &[u8],
    max_files: u64,
    max_bytes: u64,
    skip_links: bool,
) -> Result<canoa::store::Unpacked, coqui_acp_api::guatiao::library::ProviderError> {
    fs::write(dir.0.join("a.tar.gz"), bytes).expect("archive");
    let tools = LocalTools::new(Arc::new(HttpTransport::default()), vec![]).granted(&dir.0);
    let mut request = UnpackRequest::new("a.tar.gz", "out", "tar.gz", max_bytes, max_files);
    request.skip_links = skip_links;
    tools.unpack(request)
}

use tar::EntryType as E;

#[test]
fn a_good_archive_unpacks() {
    let dir = Dir::new("good");
    let archive = tar_gz(&[
        ("bin/x", b"hello", E::Regular, None),
        ("data/y", b"y", E::Regular, None),
    ]);
    let unpacked = unpack(&dir, &archive, 10, 1000, false).expect("unpacked");
    assert_eq!(unpacked.files, 2);
    assert_eq!(fs::read(dir.0.join("out/bin/x")).expect("file"), b"hello");
}

#[test]
fn an_entry_that_leaves_the_folder_is_refused() {
    for name in [
        "../evil",
        "a/../../evil",
        "/abs/evil",
        "C:evil",
        "con",
        "aux.txt",
        "a/nul",
    ] {
        let dir = Dir::new("slip");
        let archive = tar_gz(&[(name, b"x", E::Regular, None)]);
        let error = unpack(&dir, &archive, 10, 1000, false).expect_err(name);
        assert_eq!(error.status, Status::GUATIAO_ERR_BAD_VALUE, "{name}");
        assert!(!dir.0.parent().expect("p").join("evil").exists());
    }
}

#[test]
fn a_link_is_refused_and_left_out_only_when_asked() {
    let dir = Dir::new("link");
    let archive = tar_gz(&[
        ("a", b"", E::Symlink, Some("/etc/passwd")),
        ("b", b"ok", E::Regular, None),
    ]);
    assert!(unpack(&dir, &archive, 10, 1000, false).is_err());
    let dir = Dir::new("link-skip");
    let unpacked = unpack(&dir, &archive, 10, 1000, true).expect("skipped");
    assert_eq!(unpacked.files, 1);
    assert!(!dir.0.join("out/a").exists());
}

#[test]
fn an_archive_over_its_file_or_byte_limit_is_refused() {
    let dir = Dir::new("bomb");
    let many: Vec<(String, Vec<u8>)> = (0..20).map(|i| (format!("f{i}"), vec![1u8; 10])).collect();
    let entries: Vec<(&str, &[u8], E, Option<&str>)> = many
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice(), E::Regular, None))
        .collect();
    let archive = tar_gz(&entries);
    assert!(
        unpack(&dir, &archive, 5, 1_000_000, false).is_err(),
        "too many files"
    );
    let dir = Dir::new("bomb-bytes");
    let big = tar_gz(&[("x", &vec![0u8; 100_000], E::Regular, None)]);
    assert!(
        unpack(&dir, &big, 5, 1000, false).is_err(),
        "too many bytes"
    );
}

#[test]
fn a_zip_with_a_slipping_entry_is_refused() {
    let dir = Dir::new("zip");
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("ok/a.txt", options).expect("start");
        zip.write_all(b"fine").expect("write");
        zip.start_file("../evil.txt", options).expect("start");
        zip.write_all(b"x").expect("write");
        zip.finish().expect("finish");
    }
    fs::write(dir.0.join("a.zip"), buffer.into_inner()).expect("zip");
    let tools = LocalTools::new(Arc::new(HttpTransport::default()), vec![]).granted(&dir.0);
    let request = UnpackRequest::new("a.zip", "out", "zip", 1000, 10);
    assert!(tools.unpack(request).is_err());
}

#[test]
fn a_zip_unpacks_with_stripping() {
    let dir = Dir::new("zip-ok");
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        zip.start_file("root/a.txt", zip::write::SimpleFileOptions::default())
            .expect("start");
        zip.write_all(b"fine").expect("write");
        zip.finish().expect("finish");
    }
    fs::write(dir.0.join("a.zip"), buffer.into_inner()).expect("zip");
    let tools = LocalTools::new(Arc::new(HttpTransport::default()), vec![]).granted(&dir.0);
    let mut request = UnpackRequest::new("a.zip", "out", "zip", 1000, 10);
    request.strip = 1;
    tools.unpack(request).expect("unpacked");
    assert_eq!(fs::read(dir.0.join("out/a.txt")).expect("file"), b"fine");
}

#[test]
fn find_reads_only_absolute_path_entries() {
    let dir = Dir::new("find");
    let name = if cfg!(windows) { "tool.exe" } else { "tool" };
    fs::write(dir.0.join(name), b"x").expect("tool");
    let sep = if cfg!(windows) { ";" } else { ":" };
    let path = format!(".{sep}relative{sep}{}", dir.0.display());
    let lookup = Lookup {
        path: path.into(),
        home: None,
        variables: std::collections::BTreeMap::new(),
    };
    let tools = LocalTools::new(Arc::new(HttpTransport::default()), vec![]).lookup(lookup);
    let found = tools
        .find(FindRequest::new(vec!["tool".to_owned()], vec![]))
        .expect("find");
    assert_eq!(found.len(), 1);
    assert!(Path::new(&found[0].path).starts_with(&dir.0));
    // A copy in the current directory is never found through `.`.
    let cwd_tool = std::env::current_dir().expect("cwd").join(name);
    let _ = fs::remove_file(&cwd_tool);
}

#[test]
fn a_download_nothing_checks_is_refused_unless_it_was_allowed() {
    let origin = serve(vec![("/f", 200, None, b"x".to_vec())]);
    let dir = Dir::new("unverified");
    let strict = LocalTools::new(Arc::new(HttpTransport::default()), vec![origin.clone()])
        .granted(&dir.0)
        .loopback_http(true);
    let error = fetch(
        &strict,
        &origin,
        "/f",
        Check::Unverified { why: "test".into() },
        10,
        false,
    )
    .expect_err("refused");
    assert!(
        error.message().contains("nothing checks"),
        "{}",
        error.message()
    );
    assert!(!dir.0.join("out.bin").exists());
}

#[test]
fn an_endpoint_with_a_path_reaches_that_path_and_no_other_part_of_the_host() {
    let origin = serve(vec![
        ("/bucket/a", 200, None, b"in".to_vec()),
        ("/other/a", 200, None, b"out".to_vec()),
        ("/bucket/x", 302, Some("/other/a".to_owned()), Vec::new()),
    ]);
    let dir = Dir::new("prefix");
    let prefix = format!("{origin}/bucket/");
    let tools = LocalTools::new(Arc::new(HttpTransport::default()), vec![prefix])
        .granted(&dir.0)
        .loopback_http(true)
        .allow_unverified(true);
    let unchecked = || Check::Unverified { why: "test".into() };
    fetch(&tools, &origin, "/bucket/a", unchecked(), 10, false).expect("inside the prefix");
    fs::remove_file(dir.0.join("out.bin")).expect("clear");
    for path in ["/other/a", "/bucket/x"] {
        let error = fetch(&tools, &origin, path, unchecked(), 10, false).expect_err(path);
        assert!(
            error.message().contains("not one of the store's endpoints"),
            "{path}: {}",
            error.message()
        );
    }
}

#[test]
fn a_path_with_an_encoded_dot_segment_or_slash_is_not_inside_the_prefix() {
    let origin = serve(vec![
        ("/bucket/a", 200, None, b"in".to_vec()),
        ("/other/a", 200, None, b"out".to_vec()),
    ]);
    let dir = Dir::new("encoded");
    let tools = LocalTools::new(
        Arc::new(HttpTransport::default()),
        vec![format!("{origin}/bucket/")],
    )
    .granted(&dir.0)
    .loopback_http(true)
    .allow_unverified(true);
    for path in [
        "/bucket/%2e%2e/other/a",
        "/bucket/%2E%2E/other/a",
        "/bucket/..%2fother/a",
        "/bucket/%2e/a",
        "/bucket/x%2Fy",
    ] {
        let error = fetch(
            &tools,
            &origin,
            path,
            Check::Unverified { why: "t".into() },
            10,
            false,
        )
        .expect_err(path);
        assert!(
            error.message().contains("not one of the store's endpoints"),
            "{path}: {}",
            error.message()
        );
    }
}
