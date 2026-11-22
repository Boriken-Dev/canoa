//! Changes to files made of bytes: a text replaced where it occurs, and a patch in the form agents write (`*** Begin Patch`, `***
//! Add File:`, `*** Update File:`, `@@` hunks of context, removed and added lines). Lines are compared and written as bytes, so a
//! file that is not UTF-8 keeps every byte a change does not touch, and each line keeps its own line ending.

/// Where `old` next is in `bytes`, from `from`, without overlapping what was found before.
fn next(bytes: &[u8], old: &[u8], from: usize) -> Option<usize> {
    if old.is_empty() || from + old.len() > bytes.len() {
        return None;
    }
    bytes[from..]
        .windows(old.len())
        .position(|window| window == old)
        .map(|found| from + found)
}

/// How many times `old` is in `bytes`, not overlapping.
pub(crate) fn count(bytes: &[u8], old: &[u8]) -> usize {
    let (mut found, mut from) = (0, 0);
    while let Some(at) = next(bytes, old, from) {
        found += 1;
        from = at + old.len();
    }
    found
}

/// `bytes` with `old` replaced by `new`: its first occurrence, or every one with `all`.
///
/// # Errors
///
/// The result would be longer than `limit` bytes: it is not built.
pub(crate) fn replace(
    bytes: &[u8],
    old: &[u8],
    new: &[u8],
    all: bool,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let times = if all {
        count(bytes, old)
    } else {
        usize::from(next(bytes, old, 0).is_some())
    };
    let length = (bytes.len() - times * old.len()).checked_add(times.saturating_mul(new.len()));
    let length = length.filter(|length| *length <= limit).ok_or_else(|| {
        format!("the file would be larger than [host] max_message_bytes ({limit} bytes), so it is not changed")
    })?;
    let mut out = Vec::with_capacity(length);
    let (mut last, mut done) = (0, 0);
    while done < times
        && let Some(start) = next(bytes, old, last)
    {
        out.extend_from_slice(&bytes[last..start]);
        out.extend_from_slice(new);
        last = start + old.len();
        done += 1;
    }
    out.extend_from_slice(&bytes[last..]);
    Ok(out)
}

/// One hunk of an update: what it is found by, the lines it replaces and the lines that take their place.
#[derive(Debug, Default, PartialEq, Eq)]
struct Hunk {
    /// The `@@` line's text, which the change follows.
    anchor: Option<String>,
    old: Vec<String>,
    new: Vec<String>,
    /// `*** End of File`: the change is at the end.
    at_end: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Change {
    Add { path: String, lines: Vec<String> },
    Update { path: String, hunks: Vec<Hunk> },
}

/// A patch, read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Patch(Vec<Change>);

/// Reads `text`, a patch from `*** Begin Patch` to `*** End Patch`.
///
/// # Errors
///
/// What is not in the form, naming the line; a deletion or a move, which an environment cannot do with reads and writes.
pub(crate) fn parse(text: &str) -> Result<Patch, String> {
    let lines: Vec<&str> = text
        .lines()
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    let start = lines
        .iter()
        .position(|line| line.trim() == "*** Begin Patch")
        .ok_or("the patch does not start with *** Begin Patch")?;
    let end = lines
        .iter()
        .rposition(|line| line.trim() == "*** End Patch")
        .filter(|end| *end > start)
        .ok_or("the patch does not end with *** End Patch")?;
    let mut changes = Vec::new();
    let mut at = start + 1;
    while at < end {
        let line = lines[at];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            at += 1;
            let mut added = Vec::new();
            while at < end && !lines[at].starts_with("*** ") {
                let Some(text) = lines[at].strip_prefix('+') else {
                    return Err(format!(
                        "line {}: a line of an added file starts with +",
                        at + 1
                    ));
                };
                added.push(text.to_owned());
                at += 1;
            }
            changes.push(Change::Add {
                path: path.trim().to_owned(),
                lines: added,
            });
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            at += 1;
            if lines
                .get(at)
                .is_some_and(|next| next.starts_with("*** Move to: "))
            {
                return Err(format!(
                    "line {}: a move needs a file deleted, which an environment's tools cannot do",
                    at + 1
                ));
            }
            let mut hunks: Vec<Hunk> = Vec::new();
            while at < end && (!lines[at].starts_with("*** ") || lines[at] == "*** End of File") {
                let line = lines[at];
                if line == "*** End of File" {
                    hunks
                        .last_mut()
                        .ok_or("*** End of File before any hunk")?
                        .at_end = true;
                } else if let Some(anchor) = line.strip_prefix("@@") {
                    let anchor = anchor.trim();
                    hunks.push(Hunk {
                        anchor: (!anchor.is_empty()).then(|| anchor.to_owned()),
                        ..Hunk::default()
                    });
                } else {
                    if hunks.is_empty() {
                        hunks.push(Hunk::default());
                    }
                    let hunk = hunks.last_mut().ok_or("no hunk")?;
                    match line.as_bytes().first() {
                        Some(b' ') => {
                            hunk.old.push(line[1..].to_owned());
                            hunk.new.push(line[1..].to_owned());
                        }
                        Some(b'-') => hunk.old.push(line[1..].to_owned()),
                        Some(b'+') => hunk.new.push(line[1..].to_owned()),
                        None => {
                            hunk.old.push(String::new());
                            hunk.new.push(String::new());
                        }
                        _ => {
                            return Err(format!(
                                "line {}: a hunk's line starts with a space, - or +",
                                at + 1
                            ));
                        }
                    }
                }
                at += 1;
            }
            if hunks.is_empty() {
                return Err(format!("{path}: an update with no change"));
            }
            changes.push(Change::Update {
                path: path.trim().to_owned(),
                hunks,
            });
        } else if line.starts_with("*** Delete File: ") {
            return Err(format!(
                "line {}: an environment's tools write files and cannot delete one",
                at + 1
            ));
        } else if line.trim().is_empty() {
            at += 1;
        } else {
            return Err(format!("line {}: not part of a patch: {line}", at + 1));
        }
    }
    if changes.is_empty() {
        return Err("the patch changes nothing".to_owned());
    }
    Ok(Patch(changes))
}

/// A line of a file: its bytes without the line ending, and the ending.
struct Line {
    text: Vec<u8>,
    ending: &'static [u8],
}

fn lines_of(bytes: &[u8]) -> Vec<Line> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        match rest.iter().position(|byte| *byte == b'\n') {
            Some(end) => {
                let (text, ending): (&[u8], &'static [u8]) = if end > 0 && rest[end - 1] == b'\r' {
                    (&rest[..end - 1], b"\r\n")
                } else {
                    (&rest[..end], b"\n")
                };
                out.push(Line {
                    text: text.to_vec(),
                    ending,
                });
                rest = &rest[end + 1..];
            }
            None => {
                out.push(Line {
                    text: rest.to_vec(),
                    ending: b"",
                });
                rest = &[];
            }
        }
    }
    out
}

/// A way to compare two lines: what of each is compared.
type Fold = fn(&[u8]) -> &[u8];

/// Where `old` is in `lines` from `from`: as written, then ignoring trailing white space, then ignoring white space at both ends.
fn find(lines: &[Line], old: &[String], from: usize) -> Option<usize> {
    fn as_written(bytes: &[u8]) -> &[u8] {
        bytes
    }
    let ways: [Fold; 3] = [as_written, <[u8]>::trim_ascii_end, <[u8]>::trim_ascii];
    // A hunk longer than what is left of the file is not in it.
    let last = lines
        .len()
        .checked_sub(old.len())
        .filter(|last| *last >= from)?;
    for way in ways {
        let found = (from..=last).find(|start| {
            old.iter()
                .enumerate()
                .all(|(n, want)| way(&lines[start + n].text) == way(want.as_bytes()))
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

fn update(bytes: &[u8], hunks: &[Hunk], path: &str) -> Result<Vec<u8>, String> {
    let mut lines = lines_of(bytes);
    let crlf = lines.iter().filter(|line| line.ending == b"\r\n").count();
    let lf = lines.iter().filter(|line| line.ending == b"\n").count();
    let ending: &'static [u8] = if crlf > lf { b"\r\n" } else { b"\n" };
    let mut cursor = 0;
    for hunk in hunks {
        if let Some(anchor) = &hunk.anchor {
            let at = find(&lines, core::slice::from_ref(anchor), cursor)
                .ok_or_else(|| format!("{path}: the line @@ {anchor} is not in the file"))?;
            cursor = at + 1;
        }
        let start = if hunk.at_end {
            let start = lines
                .len()
                .checked_sub(hunk.old.len())
                .ok_or_else(|| format!("{path}: the file is shorter than the end of a hunk"))?;
            find(&lines, &hunk.old, start).filter(|found| *found == start)
        } else if hunk.old.is_empty() {
            Some(cursor)
        } else {
            find(&lines, &hunk.old, cursor)
        }
        .ok_or_else(|| {
            format!(
                "{path}: the lines a hunk changes are not in the file: {}",
                hunk.old.first().map_or("", String::as_str)
            )
        })?;
        if start + hunk.old.len() == lines.len()
            && !hunk.new.is_empty()
            && let Some(last) = lines.last_mut()
            && last.ending.is_empty()
            && hunk.old.is_empty()
        {
            last.ending = ending;
        }
        let kept_end = lines
            .get(start + hunk.old.len().saturating_sub(1))
            .filter(|_| !hunk.old.is_empty())
            .map(|line| line.ending);
        let mut fresh: Vec<Line> = hunk
            .new
            .iter()
            .map(|text| Line {
                text: text.as_bytes().to_vec(),
                ending,
            })
            .collect();
        // The last line replaced keeps its ending, so a file without a final line ending stays without one.
        if let (Some(kept), Some(last)) = (kept_end, fresh.last_mut()) {
            last.ending = kept;
        }
        let added = fresh.len();
        lines.splice(start..start + hunk.old.len(), fresh);
        cursor = start + added;
    }
    let mut out = Vec::with_capacity(bytes.len());
    for line in lines {
        out.extend_from_slice(&line.text);
        out.extend_from_slice(line.ending);
    }
    Ok(out)
}

impl Patch {
    /// The files it writes, in its order.
    pub(crate) fn paths(&self) -> Vec<&str> {
        self.0
            .iter()
            .map(|change| match change {
                Change::Add { path, .. } | Change::Update { path, .. } => path.as_str(),
            })
            .collect()
    }

    /// Reads every file it updates through `read`, works out each new content, and only then writes them through `write`, in its
    /// order; the paths written.
    ///
    /// # Errors
    ///
    /// A file that cannot be read or a hunk not found in it (nothing is written then), or a write that failed, naming what was
    /// written before it.
    pub(crate) fn apply(
        &self,
        read: impl Fn(&str) -> Result<Option<Vec<u8>>, String>,
        write: impl Fn(&str, Vec<u8>) -> Result<(), String>,
    ) -> Result<Vec<String>, String> {
        let mut planned = Vec::new();
        for change in &self.0 {
            match change {
                Change::Add { path, lines } => {
                    let mut bytes = Vec::new();
                    for line in lines {
                        bytes.extend_from_slice(line.as_bytes());
                        bytes.push(b'\n');
                    }
                    planned.push((path.clone(), bytes));
                }
                Change::Update { path, hunks } => {
                    let held = read(path)?.ok_or_else(|| format!("{path} is not there"))?;
                    planned.push((path.clone(), update(&held, hunks, path)?));
                }
            }
        }
        let mut written = Vec::new();
        for (path, bytes) in planned {
            write(&path, bytes).map_err(|why| {
                if written.is_empty() {
                    why
                } else {
                    format!("{why} (already written: {})", written.join(", "))
                }
            })?;
            written.push(path);
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn a_replacement_counts_every_occurrence_and_keeps_every_other_byte() {
        let bytes = b"\xff\xfeA-x-\x80x\x00";
        assert_eq!(count(bytes, b"x"), 2);
        let once = replace(bytes, b"x", b"yy", false, 100).expect("small");
        assert_eq!(once.as_slice(), &b"\xff\xfeA-yy-\x80x\x00"[..]);
        let every = replace(bytes, b"x", b"", true, 100).expect("small");
        assert_eq!(every.as_slice(), &b"\xff\xfeA--\x80\x00"[..]);
        assert_eq!(count(b"abc", b"z"), 0);
    }

    #[test]
    fn a_replacement_that_would_pass_the_limit_is_refused_before_it_is_built() {
        // A mebibyte of `a`, each made 64 KiB: 64 GiB if it were built.
        let file = vec![b'a'; 1 << 20];
        let new = vec![b'b'; 64 * 1024];
        let why = replace(&file, b"a", &new, true, 16 << 20).expect_err("refused");
        assert!(why.contains("max_message_bytes"), "{why}");
        assert_eq!(
            replace(b"aa", b"a", b"bb", true, 4).expect("fits"),
            b"bbbb".to_vec()
        );
        assert!(
            replace(b"aa", b"a", b"bb", true, 3).is_err(),
            "one byte over"
        );
    }

    #[test]
    fn a_hunk_longer_than_the_file_is_not_found_and_nothing_panics() {
        let held = files(&[("a.txt", b"x\n")]);
        let patch = "*** Begin Patch\n*** Update File: a.txt\n-x\n-y\n-z\n+w\n*** End Patch\n";
        let why = run(patch, &held).expect_err("not found");
        assert!(why.contains("not in the file"), "{why}");
        assert_eq!(held.borrow()["a.txt"], b"x\n".to_vec());
    }

    fn files(start: &[(&str, &[u8])]) -> RefCell<BTreeMap<String, Vec<u8>>> {
        RefCell::new(
            start
                .iter()
                .map(|(path, bytes)| ((*path).to_owned(), bytes.to_vec()))
                .collect(),
        )
    }

    fn run(patch: &str, held: &RefCell<BTreeMap<String, Vec<u8>>>) -> Result<Vec<String>, String> {
        parse(patch)?.apply(
            |path| Ok(held.borrow().get(path).cloned()),
            |path, bytes| {
                held.borrow_mut().insert(path.to_owned(), bytes);
                Ok(())
            },
        )
    }

    #[test]
    fn an_update_changes_the_lines_it_names_and_leaves_the_rest_byte_for_byte() {
        let held = files(&[("a.txt", b"one\r\n\xfftwo\r\nthree\r\nfour")]);
        let patch =
            "*** Begin Patch\n*** Update File: a.txt\n@@ one\n-three\n+THREE\n+3\n*** End Patch\n";
        assert_eq!(run(patch, &held).expect("applied"), ["a.txt"]);
        assert_eq!(
            held.borrow()["a.txt"],
            b"one\r\n\xfftwo\r\nTHREE\r\n3\r\nfour".to_vec()
        );
    }

    #[test]
    fn a_file_is_added_and_a_hunk_at_the_end_keeps_a_file_without_a_last_line_ending() {
        let held = files(&[("b.txt", b"x\ny")]);
        let patch = "*** Begin Patch\n*** Add File: new.txt\n+hello\n+world\n*** Update File: b.txt\n@@\n x\n-y\n+z\n*** End of File\n*** End Patch";
        run(patch, &held).expect("applied");
        assert_eq!(held.borrow()["new.txt"], b"hello\nworld\n".to_vec());
        assert_eq!(held.borrow()["b.txt"], b"x\nz".to_vec());
    }

    #[test]
    fn nothing_is_written_when_one_hunk_is_not_found_and_a_delete_or_move_is_refused() {
        let held = files(&[("a.txt", b"one\n"), ("b.txt", b"two\n")]);
        let patch = "*** Begin Patch\n*** Update File: a.txt\n-one\n+uno\n*** Update File: b.txt\n-zwei\n+dos\n*** End Patch";
        let why = run(patch, &held).expect_err("refused");
        assert!(why.contains("b.txt"), "{why}");
        assert_eq!(
            held.borrow()["a.txt"],
            b"one\n".to_vec(),
            "nothing was written"
        );
        for refused in [
            "*** Begin Patch\n*** Delete File: a.txt\n*** End Patch",
            "*** Begin Patch\n*** Update File: a.txt\n*** Move to: c.txt\n-one\n+uno\n*** End Patch",
            "*** Update File: a.txt\n-one\n+uno\n",
        ] {
            assert!(parse(refused).is_err(), "{refused}");
        }
    }
}
