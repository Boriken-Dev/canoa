//! A windowed application (`SUBSYSTEM:WINDOWS` on Windows) started by a browser with `chrome-extension://...` ends promptly
//! when the browser closes its port (both with the host's door down and with a link open) or when the host's link closes,
//! even when launched through `cmd.exe /d /s /c` with a hidden console.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use canoa_host::Home;
use serde_json::{Value, json};

const ORIGIN: &str = "chrome-extension://lpppofcphpjkedfnpbpabhmiamaicaed/";
const WAIT: Duration = Duration::from_secs(5);

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "canoa-ffi-native-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("creates scratch directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn windowed_child() -> PathBuf {
    let tests = std::env::current_exe().expect("test binary path");
    tests
        .parent()
        .and_then(Path::parent)
        .expect("target/<profile> directory")
        .join("examples")
        .join(format!("windowed_child{}", std::env::consts::EXE_SUFFIX))
}

enum Launch {
    Direct,
    #[cfg(windows)]
    CmdNoWindow,
}

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn as_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("child still present")
    }

    fn wait_exit(mut self, timeout: Duration) -> Result<ExitStatus, String> {
        let mut child = self.0.take().expect("child still present");
        wait_exit(&mut child, timeout)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = wait_exit(&mut child, Duration::from_millis(100));
        }
    }
}

fn spawn_child(launch: &Launch, home: &Path, args: &[&str]) -> ChildGuard {
    let program = windowed_child();
    assert!(
        program.exists(),
        "build examples first: {}",
        program.display()
    );
    let child = match launch {
        Launch::Direct => Command::new(&program)
            .args(args)
            .env("COQUI_AGENTIC_PEER_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawns windowed_child"),
        #[cfg(windows)]
        Launch::CmdNoWindow => {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let quoted_args: Vec<String> = args
                .iter()
                .map(|arg| {
                    if arg.contains(' ') {
                        format!("\"{arg}\"")
                    } else {
                        (*arg).to_owned()
                    }
                })
                .collect();
            let line = format!("\"{}\" {}", program.display(), quoted_args.join(" "));
            Command::new("cmd.exe")
                .args(["/d", "/s", "/c"])
                .raw_arg(format!("\"{line}\""))
                .env("COQUI_AGENTIC_PEER_HOME", home)
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawns cmd.exe native host")
        }
    };
    ChildGuard(Some(child))
}

fn write_native(stdin: &mut impl std::io::Write, value: &Value) {
    let body = value.to_string();
    let len = u32::try_from(body.len()).expect("short native message");
    stdin
        .write_all(&len.to_le_bytes())
        .and_then(|()| stdin.write_all(body.as_bytes()))
        .and_then(|()| stdin.flush())
        .expect("writes native message");
}

fn start_reader(mut stdout: std::process::ChildStdout) -> Receiver<Value> {
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        let mut len_buf = [0_u8; 4];
        while stdout.read_exact(&mut len_buf).is_ok() {
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut body = vec![0_u8; len];
            if stdout.read_exact(&mut body).is_err() {
                break;
            }
            let Ok(value) = serde_json::from_slice::<Value>(&body) else {
                break;
            };
            if send.send(value).is_err() {
                break;
            }
        }
    });
    recv
}

fn wait_exit(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/T", "/F", "/PID", &child.id().to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("process did not exit within {timeout:?}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn assert_door_down_exits(launch: &Launch) {
    let scratch = Scratch::new("door-down");
    let home = scratch.0.join("home");
    std::fs::create_dir_all(&home).expect("creates home");
    let mut child = spawn_child(launch, &home, &[ORIGIN, "--parent-window=0"]);
    let mut stdin = child.as_mut().stdin.take().expect("stdin");
    let messages = start_reader(child.as_mut().stdout.take().expect("stdout"));
    write_native(&mut stdin, &json!({ "op": "hello" }));
    let reply = messages
        .recv_timeout(WAIT)
        .unwrap_or_else(|e| panic!("expected hello reply within {WAIT:?}: {e}"));
    assert_eq!(reply["reply"], "hello", "{reply}");
    assert_eq!(reply["running"], false, "{reply}");
    assert_eq!(reply["door"], false, "{reply}");
    drop(stdin);
    let status = child
        .wait_exit(WAIT)
        .expect("relay should exit after stdin closes");
    assert!(status.success(), "{status:?}");
}

#[test]
fn windowed_relay_exits_when_input_closes_with_door_down() {
    assert_door_down_exits(&Launch::Direct);
    #[cfg(windows)]
    assert_door_down_exits(&Launch::CmdNoWindow);
}

#[test]
fn windowed_relay_exits_when_input_closes_and_when_door_closes() {
    let launch = {
        #[cfg(windows)]
        {
            Launch::CmdNoWindow
        }
        #[cfg(not(windows))]
        {
            Launch::Direct
        }
    };
    // 1) Browser closes port while link is open.
    {
        let scratch = Scratch::new("door-open-stdin-close");
        let home = Home::at(scratch.0.join("home"));
        std::fs::create_dir_all(home.dir()).expect("creates home");
        let door = canoa_host::native::door_name(home.dir()).expect("door name");
        let listener = canoa_host::pipe::Listener::bind(&door).expect("binds door");
        let door_thread = std::thread::spawn(move || {
            let mut stream = listener.accept().expect("accepts");
            let mut len = [0_u8; 4];
            let _ = stream.read_exact(&mut len);
            let mut body = vec![0_u8; u32::from_le_bytes(len) as usize];
            let _ = stream.read_exact(&mut body);
            let ok = br#"{"opened":true}"#;
            let _ = stream.write_all(&u32::try_from(ok.len()).expect("len").to_le_bytes());
            let _ = stream.write_all(ok);
            let _ = stream.flush();
            // Keep link open until peer shuts down or closes.
            let mut drain = [0_u8; 64];
            while stream.read(&mut drain).is_ok_and(|n| n > 0) {}
        });

        let mut child = spawn_child(&launch, home.dir(), &[ORIGIN, "--parent-window=0"]);
        let mut stdin = child.as_mut().stdin.take().expect("stdin");
        let messages = start_reader(child.as_mut().stdout.take().expect("stdout"));
        write_native(&mut stdin, &json!({ "op": "open", "entry": "@control" }));
        let opened = messages.recv_timeout(WAIT).expect("open reply");
        assert_eq!(opened["reply"], "open", "{opened}");
        assert_eq!(opened["opened"], true, "{opened}");
        drop(stdin);
        let status = child
            .wait_exit(WAIT)
            .expect("relay exits when stdin closes");
        assert!(status.success(), "{status:?}");
        let _ = door_thread.join();
    }

    // 2) Door closes link while stdin is still open.
    {
        let scratch = Scratch::new("door-closes-first");
        let home = Home::at(scratch.0.join("home"));
        std::fs::create_dir_all(home.dir()).expect("creates home");
        let door = canoa_host::native::door_name(home.dir()).expect("door name");
        let listener = canoa_host::pipe::Listener::bind(&door).expect("binds door");
        let door_thread = std::thread::spawn(move || {
            let mut stream = listener.accept().expect("accepts");
            let mut len = [0_u8; 4];
            let _ = stream.read_exact(&mut len);
            let mut body = vec![0_u8; u32::from_le_bytes(len) as usize];
            let _ = stream.read_exact(&mut body);
            let ok = br#"{"opened":true}"#;
            let _ = stream.write_all(&u32::try_from(ok.len()).expect("len").to_le_bytes());
            let _ = stream.write_all(ok);
            let _ = stream.flush();
            drop(stream);
        });

        let mut child = spawn_child(&launch, home.dir(), &[ORIGIN, "--parent-window=0"]);
        let mut stdin = child.as_mut().stdin.take().expect("stdin");
        let messages = start_reader(child.as_mut().stdout.take().expect("stdout"));
        write_native(&mut stdin, &json!({ "op": "open", "entry": "@control" }));
        let opened = messages.recv_timeout(WAIT).expect("open reply");
        assert_eq!(opened["opened"], true, "{opened}");
        let closed = messages.recv_timeout(WAIT).expect("closed notice");
        assert_eq!(closed["closed"], true, "{closed}");
        let status = child.wait_exit(WAIT).expect("relay exits when door closes");
        assert!(status.success(), "{status:?}");
        drop(stdin);
        let _ = door_thread.join();
    }
}

#[cfg(windows)]
#[test]
fn windowed_child_command_keeps_piped_stdio_when_parent_has_a_console() {
    let scratch = Scratch::new("piped-child");
    let home = scratch.0.join("home");
    std::fs::create_dir_all(&home).expect("creates home");
    let child_entry =
        windowed_child().with_file_name(format!("child_entry{}", std::env::consts::EXE_SUFFIX));
    let target = child_entry.to_string_lossy().into_owned();
    let mut child = spawn_child(
        &Launch::CmdNoWindow,
        &home,
        &["--canoa-child", "launch", "--", &target],
    );
    let mut stdin = child.as_mut().stdin.take().expect("stdin");
    stdin
        .write_all(b"{\"canoa_host_launch\":1,\"env\":[[\"CANOA_FFI_TEST_VALUE\",\"given\"]]}\n")
        .expect("writes launch request");
    drop(stdin);
    let mut out = String::new();
    child
        .as_mut()
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .expect("reads stdout");
    let status = child.wait_exit(WAIT).expect("launch exits");
    assert_eq!(status.code(), Some(7), "{status:?}");
    assert_eq!(out.trim(), "given", "stdout was: {out:?}");
}
