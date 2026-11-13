//! Windows shares (`\\server\share\folder`) are ordinary absolute paths. These keep one written form of a path to compare
//! and to hand to a program, and tell a share from a device namespace.

use alloc::borrow::Cow;
use alloc::format;
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

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
