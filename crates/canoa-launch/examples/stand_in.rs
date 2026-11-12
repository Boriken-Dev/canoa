//! The program a launcher test packs in place of the app: reports where it runs from and what it was given.
//!
//! `--canoa-child ...` ends with status 2, as the app's host child does with no window; `--exit N` ends with N.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = std::env::current_exe().expect("current_exe");
    let dir = exe.parent().expect("parent");
    println!("stand-in dir={}", dir.display());
    println!("stand-in library={}", dir.join("canoa_ffi.dll").is_file());
    println!("stand-in args={args:?}");
    let cwd = std::env::current_dir().expect("current_dir");
    println!("stand-in cwd={}", cwd.display());
    println!(
        "stand-in config={}",
        std::env::var("CANOA_CONFIG").unwrap_or_default()
    );
    let code = match args.as_slice() {
        [flag, ..] if flag == "--canoa-child" => 2,
        [flag, n, ..] if flag == "--exit" => n.parse().unwrap_or(1),
        _ => 0,
    };
    std::process::exit(code);
}
