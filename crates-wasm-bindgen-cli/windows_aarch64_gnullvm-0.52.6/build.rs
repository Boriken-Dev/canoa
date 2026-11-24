use std::fs;
use std::path::Path;

// The import libraries this crate carries are `lib/<name>.hex`; link against them as bytes written below OUT_DIR.
fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out = std::env::var("OUT_DIR").unwrap();
    for entry in fs::read_dir(Path::new(&dir).join("lib")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(library) = name.strip_suffix(".hex") {
            let text = fs::read_to_string(&path).unwrap();
            let digits: Vec<u8> = text.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
            let bytes: Vec<u8> = digits
                .chunks(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            fs::write(Path::new(&out).join(library), bytes).unwrap();
        }
    }
    println!("cargo:rustc-link-search=native={}", out);
}
