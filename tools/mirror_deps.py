#!/usr/bin/env python3
"""Populate pub and CDN caches on Canoa's deps branch, with compile files only.

Each hosted package keeps what a build needs: its tests, examples, benchmarks, docs and
.github directories are dropped, then any remaining file that a malware or archive
scanner would refuse (archives, executables, key files, pack files, opaque
high-entropy data) is deleted. Git dependencies are not mirrored (their bare clones are
pack files); manifest.json records each one's URL and pinned revision from
pubspec.lock, and resolving them needs the network. Hosted packages resolve offline
from the result with PUB_CACHE pointed at the branch's pub directory.
The one exception is cdn/office-js/, which holds a whole repository tree unmodified, by the owner's word.
"""
import argparse
import collections
import json
import math
import re
import shutil
import sys
from pathlib import Path

sys.path.insert(0,str(Path(__file__).resolve().parent))
import cdn_files
import fallback_fonts
import office_js

# leanflutter's desktop packages: the tray and the window. A desktop build needs all of these, whichever of them pub resolves.
DESKTOP_PACKAGES = ("tray_manager","window_manager","screen_retriever","screen_retriever_linux","screen_retriever_macos",
                    "screen_retriever_platform_interface","screen_retriever_windows","menu_base")
# What those packages link on Linux, which no pub cache can carry: the manifest says so, for someone building offline.
DESKTOP_LINUX_SYSTEM = ("libayatana-appindicator3-dev","libgtk-3-dev","libx11-dev","libxi-dev")
# Directories at the top of a package that compilation never reads.
DROP_DIRECTORIES = ("test","tests","example","examples","benchmark","benchmarks","doc","docs",".github")

FLAG_MAGIC = [
    (b"PK\x03\x04", "zip"), (b"PK\x05\x06", "zip"), (b"PK\x07\x08", "zip"),
    (b"\x1f\x8b", "gzip"), (b"BZh", "bzip2"), (b"\xfd7zXZ\x00", "xz"),
    (b"7z\xbc\xaf\x27\x1c", "7z"), (b"Rar!\x1a\x07", "rar"), (b"\x28\xb5\x2f\xfd", "zstd"),
    (b"\x04\x22\x4d\x18", "lz4"), (b"MSCF", "cab"), (b"Cr24", "crx"), (b"!<arch>", "ar/deb"),
    (b"\xed\xab\xee\xdb", "rpm"), (b"Salted__", "openssl-encrypted"),
    (b"age-encryption.org", "age-encrypted"), (b"-----BEGIN PGP MESSAGE", "pgp-encrypted"),
    (b"MZ", "windows executable"), (b"\x7fELF", "elf executable"),
    (b"\xcf\xfa\xed\xfe", "mach-o executable"), (b"\xfe\xed\xfa\xcf", "mach-o executable"),
    (b"\xca\xfe\xba\xbe", "mach-o/java class"), (b"PACK", "git pack"), (b"\xfftOc", "git pack index"),
    (b"RIDX", "git reverse index"), (b"\x30\x82", "der (certificate or key)"),
]
MEDIA = [b"\x89PNG", b"GIF8", b"\xff\xd8\xff", b"RIFF", b"\x00\x00\x01\x00", b"wOFF", b"wOF2",
         b"\x00\x01\x00\x00", b"OTTO"]
KEY_EXT = re.compile(r"\.(pfx|p12|jks|keystore|der|key)$", re.I)
# A whole PEM block with a body, not a label quoted in code or docs.
PRIVATE_KEY = re.compile(
    rb"-----BEGIN ([A-Z0-9-]+ )?PRIVATE KEY-----\r?\n(?:[A-Za-z0-9+/=]{16,}\r?\n){2,}"
    rb"[A-Za-z0-9+/=]*\r?\n?-----END ([A-Z0-9-]+ )?PRIVATE KEY-----")


def entropy(b):
    c = collections.Counter(b)
    return -sum(v / len(b) * math.log2(v / len(b)) for v in c.values()) if b else 0.0


def flagged_kind(body, name):
    """The kind of scanner-refused data in a file's bytes, or None."""
    head = body[:600]
    if len(head) >= 262 and head[257:262] == b"ustar":
        return "tar"
    for magic, kind in FLAG_MAGIC:
        if head.startswith(magic):
            return kind
    if KEY_EXT.search(name):
        return "key or keystore file"
    if head.startswith((b"\x00asm", b"%PDF", b"SQLite format 3")):
        return None
    if any(head.startswith(m) for m in MEDIA) or head[4:8] == b"ftyp":
        return None
    try:
        body.decode("utf-8")
        return "private key in text" if PRIVATE_KEY.search(body) else None
    except UnicodeDecodeError:
        pass
    return "opaque high-entropy data" if entropy(body[:65536]) > 7.5 else None


def git_dependencies(lock):
    """Each git entry of pubspec.lock: package, url, path, ref and resolved revision."""
    found=[]
    current=None
    description=False
    entry={}
    for line in Path(lock).read_text(encoding="utf-8").splitlines():
        if re.match(r"^  \S.*:$",line):
            current=line.strip()[:-1]; entry={}; description=False
        elif line.startswith("    description:"):
            description=True
        elif line.startswith("    source:"):
            description=False
            if line.split(":",1)[1].strip()=="git":
                found.append(dict(package=current,url=entry.get("url"),path=entry.get("path"),
                                  ref=entry.get("ref"),revision=entry.get("resolved-ref")))
        elif description and line.startswith("      "):
            key,_,value=line.strip().partition(":")
            entry[key]=value.strip().strip('"')
    return found


def filter_hosted(root):
    """Drop non-compilation directories, then flagged files. Returns (dropped, deleted)."""
    dropped=collections.Counter()
    deleted=[]
    for package in sorted(p for p in root.glob("*/*") if p.is_dir()):
        for name in DROP_DIRECTORIES:
            directory=package/name
            if directory.is_dir():
                dropped[name]+=sum(1 for f in directory.rglob("*") if f.is_file())
                shutil.rmtree(directory)
        for file in sorted(f for f in package.rglob("*") if f.is_file()):
            kind=flagged_kind(file.read_bytes(),file.name)
            if kind:
                dropped[kind]+=1
                deleted.append(dict(path=file.relative_to(root).as_posix(),kind=kind))
                file.unlink()
    return dict(sorted(dropped.items())),deleted


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--source-pub",required=True)
    parser.add_argument("--destination",required=True)
    parser.add_argument("--revision",required=True)
    parser.add_argument("--lock",default=str(Path(__file__).resolve().parent.parent/"app"/"pubspec.lock"),
                        help="pubspec.lock whose git dependencies the manifest records")
    parser.add_argument("--skip-cdn",action="store_true")
    args=parser.parse_args()
    target=Path(args.destination)
    for kind in ("hosted","hosted-hashes"):
        src=Path(args.source_pub)/kind
        dst=target/"pub"/kind
        if dst.exists(): shutil.rmtree(dst)
        if src.exists(): shutil.copytree(src,dst,ignore=shutil.ignore_patterns("*.lock"))
    stale=target/"pub"/"git"
    if stale.exists(): shutil.rmtree(stale)
    dropped,deleted=filter_hosted(target/"pub"/"hosted")
    directory=target/"pub"/"hosted"/"pub.dev"
    packages=sorted(item.name for item in directory.glob("*") if item.is_dir())
    gits=git_dependencies(args.lock)
    missing=[name for name in DESKTOP_PACKAGES if not any(package.rsplit("-",1)[0]==name for package in packages)]
    if missing: raise SystemExit("Desktop packages missing from the mirror: "+", ".join(missing))
    files=[]
    if not args.skip_cdn:
        # The CDN files the builds read (the fonts), pinned by tools/cdn_files.json.
        files=cdn_files.mirror(target,cdn_files.read_list()["files"])
        # Office.js: the whole tree of one commit of its repository, written as published. This is the one exception to keeping
        # only what compiles (the owner's word): nothing in cdn/office-js is trimmed, renamed or rewritten, and git must not
        # convert its line endings either.
        office_pin=office_js.read_list()
        office=office_js.mirror(target,office_pin)
        attributes=target/".gitattributes"
        text=attributes.read_text(encoding="utf-8") if attributes.is_file() else ""
        if "cdn/office-js/**" not in text:
            attributes.write_text(text+("" if not text or text.endswith("\n") else "\n")+"cdn/office-js/** -text\n",encoding="utf-8",newline="\n")
        (target/"cdn"/"README.md").write_text(cdn_files.README,encoding="utf-8",newline="\n")
    else:
        office=[]
        office_pin=None
    fallback=0
    if not args.skip_cdn:
        # The fonts the web engine falls back to, pinned by fallback_fonts.sha256: the app's build copies them from here.
        pinned=fallback_fonts.read_list()
        fallback_fonts.check_engine(pinned,fallback_fonts.flutter_root())
        fallback=fallback_fonts.mirror(target,pinned)
    report=dict(format=2,source="Boriken-Dev/canoa",sourceCommit=args.revision,
                status="complete",pubPackages=packages,gitDependencies=gits,
                gitDependenciesNote="not mirrored; pub fetches them from their URL, so they need the network",
                dropped=dict(directories=list(DROP_DIRECTORIES),counts=dropped,deletedFlaggedFiles=deleted),
                cdnFiles=files,
                officeJs=None if office_pin is None else dict(repository=office_pin["repository"],commit=office_pin["commit"],
                    fileVersion=office_pin["file_version"],carriedByTheHost=office_pin["carried"],files=office),
                fallbackFonts=dict(count=0 if args.skip_cdn else len(pinned),bytes=fallback),
                desktopPackages=list(DESKTOP_PACKAGES),desktopLinuxSystemPackages=list(DESKTOP_LINUX_SYSTEM))
    (target/"manifest.json").write_text(json.dumps(report,indent=2)+"\n")
    print("Mirrored",len(packages),"pub packages,",len(gits),"git dependencies recorded and",len(files),"CDN files;",
          "dropped",sum(dropped.values()),"files")
if __name__=="__main__": main()
