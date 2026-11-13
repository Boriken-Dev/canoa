//! Windows shares (`\\server\share\folder`) are ordinary absolute paths. These keep one written form of a path to compare
//! and to hand to a program, and tell a share from a device namespace.

use alloc::borrow::{Cow, ToOwned as _};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};

/// `path` in its plain form: on Windows `\\?\C:\work` is `C:\work` and `\\?\UNC\server\share\work` is
/// `\\server\share\work`, and a path that opens with two separators (`//server/share/work`) is written with backslashes, so
/// that however a share is written it compares with a root. Any other path, and every path elsewhere, is as it was
/// written.
pub(crate) fn plain(path: &Path) -> Cow<'_, Path> {
    if !cfg!(windows) {
        return Cow::Borrowed(path);
    }
    let Some(text) = path.to_str() else {
        return Cow::Borrowed(path);
    };
    let slashes =
        text.starts_with(['/', '\\']) && text[1..].starts_with(['/', '\\']) && text.contains('/');
    let written = if slashes {
        Cow::Owned(text.replace('/', "\\"))
    } else {
        Cow::Borrowed(text)
    };
    let kept = |written: Cow<'_, str>| match written {
        Cow::Owned(text) => Cow::Owned(PathBuf::from(text)),
        Cow::Borrowed(_) => Cow::Borrowed(path),
    };
    let Some(rest) = written.strip_prefix(r"\\?\") else {
        return kept(written);
    };
    if let Some((word, share)) = rest.split_at_checked(4)
        && word.eq_ignore_ascii_case(r"UNC\")
    {
        return Cow::Owned(PathBuf::from(format!(r"\\{share}")));
    }
    let bytes = rest.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Cow::Owned(PathBuf::from(rest));
    }
    kept(written)
}

/// Whether `path` is in a device namespace (`\\.\pipe\x`, `\\?\Volume{...}\`), which is no folder of any machine's.
pub(crate) fn is_device(path: &Path) -> bool {
    matches!(
        plain(path).components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::DeviceNS(_) | Prefix::Verbatim(_))
    )
}

/// Whether `path` is a folder on a share, written `\\server\share\...`, in either form.
pub(crate) fn is_share(path: &Path) -> bool {
    matches!(
        plain(path).components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::UNC(..))
    )
}

/// What to start for `program args` to run in `cwd`. cmd.exe, which runs a `.cmd` or `.bat` file, takes no share as its
/// working directory and falls back to the Windows folder, so for a file npm wrote it is the file's own program that is
/// started, in `cwd`. Anything else is as asked.
///
/// # Errors
///
/// `program` is a batch file, `cwd` is on a share and the file is not one that can be read this way.
pub(crate) fn launch_in(
    program: &str,
    args: &[String],
    cwd: &Path,
) -> io::Result<(OsString, Vec<OsString>)> {
    let asked = || {
        (
            OsString::from(program),
            args.iter().map(OsString::from).collect(),
        )
    };
    let batch = Path::new(program)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"));
    if !(cfg!(windows) && batch && is_share(cwd)) {
        return Ok(asked());
    }
    let (start, first) = std::fs::read_to_string(program)
        .ok()
        .and_then(|text| npm_shim(&text, Path::new(program)))
        .ok_or_else(|| {
            io::Error::other(format!(
                "{program} is a batch file, which cmd.exe cannot start in a folder on a share"
            ))
        })?;
    let mut all = first;
    all.extend(args.iter().map(OsString::from));
    Ok((start, all))
}

/// The program and first arguments of the `.cmd` file `text` that npm wrote at `file`: its last command, which names its
/// program (`%_prog%`: a `node.exe` beside the file, else `node`) or a path under the file's folder (`%dp0%`), and its
/// arguments, ended by `%*`. `None` for any other file.
fn npm_shim(text: &str, file: &Path) -> Option<(OsString, Vec<OsString>)> {
    let folder = file.parent()?.to_string_lossy().into_owned();
    let line = text.lines().rev().find(|line| line.contains("%*"))?;
    let command = line.rsplit_once("& ").map_or(line, |(_, command)| command);
    let mut words = Vec::new();
    let mut rest = command.trim();
    while !rest.is_empty() {
        let (word, after) = if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"')?;
            (&quoted[..end], &quoted[end + 1..])
        } else {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            (&rest[..end], &rest[end..])
        };
        words.push(word);
        rest = after.trim_start();
    }
    if words.pop()? != "%*" {
        return None;
    }
    let first = words.first()?;
    if !(*first == "%_prog%" || first.starts_with("%dp0%") || first.starts_with("%~dp0")) {
        return None;
    }
    let mut made = Vec::new();
    for word in words {
        let word = if word == "%_prog%" {
            let own = Path::new(&folder).join("node.exe");
            if own.is_file() {
                own.to_string_lossy().into_owned()
            } else {
                "node".to_owned()
            }
        } else {
            word.replace("%dp0%", &folder).replace("%~dp0", &folder)
        };
        if word.contains('%') {
            return None;
        }
        made.push(OsString::from(word));
    }
    let mut made = made.into_iter();
    let program = made.next()?;
    Some((program, made.collect()))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    const SHIM: &str = concat!(
        "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\n",
        "IF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n",
        "  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\n",
        "endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@scope\\tool\\bin\\tool.js\" %*\r\n"
    );

    #[test]
    fn an_npm_command_file_names_its_program_and_script() {
        let (program, args) = npm_shim(SHIM, Path::new(r"C:\npm\tool.cmd")).expect("a shim");
        assert_eq!(program, "node");
        assert_eq!(args, [r"C:\npm\node_modules\@scope\tool\bin\tool.js"]);
        let direct = "@SETLOCAL\r\n\"%~dp0\\tool.exe\"   %*\r\n";
        let (program, args) = npm_shim(direct, Path::new(r"C:\npm\tool.cmd")).expect("a shim");
        assert_eq!(program, r"C:\npm\tool.exe");
        assert!(args.is_empty());
    }

    #[test]
    fn any_other_batch_file_is_not_read_as_one() {
        for text in [
            "@echo off\r\nset X=%TEMP%\r\ncall other.cmd %*\r\n",
            "@echo hi\r\n",
            "\"%_prog%\" \"%SOMETHING%\\x.js\" %*\r\n",
        ] {
            assert_eq!(
                npm_shim(text, Path::new(r"C:\npm\tool.cmd")),
                None,
                "{text}"
            );
        }
    }

    #[test]
    fn only_a_batch_file_in_a_share_folder_is_started_another_way() {
        let args = vec!["--mode".to_owned()];
        for (program, cwd) in [
            (r"C:\bin\tool.exe", r"\\fileserver\projects\app"),
            (r"C:\bin\tool.cmd", r"C:\work"),
        ] {
            let (start, all) = launch_in(program, &args, Path::new(cwd)).expect("as asked");
            assert_eq!(
                (start, all),
                (OsString::from(program), vec![OsString::from("--mode")])
            );
        }
        let error = launch_in(
            r"C:\bin\absent.cmd",
            &args,
            Path::new(r"\\fileserver\projects"),
        )
        .expect_err("no file to read");
        assert!(error.to_string().contains("batch file"), "{error}");
    }

    #[test]
    fn a_verbatim_path_is_written_plain() {
        for (written, plain_form) in [
            (
                r"\\?\UNC\fileserver\projects\app",
                r"\\fileserver\projects\app",
            ),
            (r"\\?\unc\fileserver\projects", r"\\fileserver\projects"),
            (r"\\?\C:\work\app", r"C:\work\app"),
            (r"\\fileserver\projects\app", r"\\fileserver\projects\app"),
            (r"C:\work", r"C:\work"),
            (r"\\?\Volume{1234}\work", r"\\?\Volume{1234}\work"),
            ("//fileserver/projects/app", r"\\fileserver\projects\app"),
            (r"\\fileserver/projects/app", r"\\fileserver\projects\app"),
            ("//?/UNC/fileserver/projects", r"\\fileserver\projects"),
            ("//./pipe/x", r"\\.\pipe\x"),
            ("//?/C:/work", r"C:\work"),
        ] {
            assert_eq!(
                plain(Path::new(written)),
                Path::new(plain_form),
                "{written}"
            );
        }
    }

    #[test]
    fn a_share_is_told_from_a_device() {
        assert!(is_share(Path::new(r"\\fileserver\projects\app")));
        assert!(is_share(Path::new(r"\\?\UNC\fileserver\projects\app")));
        assert!(!is_share(Path::new(r"C:\work")));
        assert!(!is_share(Path::new(r"\\.\pipe\x")));
        assert!(is_device(Path::new(r"\\.\pipe\x")));
        assert!(is_device(Path::new("//./pipe/x")));
        assert!(is_share(Path::new("//fileserver/projects/app")));
        assert!(is_device(Path::new(r"\\?\Volume{1234}\work")));
        assert!(!is_device(Path::new(r"\\fileserver\projects\app")));
        assert!(!is_device(Path::new(r"\\?\UNC\fileserver\projects\app")));
        assert!(!is_device(Path::new(r"\\?\C:\work")));
    }
}
