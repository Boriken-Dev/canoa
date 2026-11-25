#!/usr/bin/env python3
"""Populate pub and CDN caches on Canoa's deps branch, with compile files only.

Each hosted package keeps what a build needs: its tests, examples, benchmarks, docs and
.github directories are dropped, then any remaining file that a malware or archive
scanner would refuse (archives, executables, key files, pack files, opaque
high-entropy data) is deleted. A git dependency is carried as a source tree, never a clone (a
clone is pack files): pub/git/<repo>-<revision>/ holds, at the revision pubspec.lock names, the path
each package of that repository uses, as `git archive` writes it and under the same filtering. The
folder is named as pub names its own checkout, so app/tool/offline_pub.py seeds a pub cache with it
and pub resolves hosted and git packages offline, with PUB_CACHE pointed at that cache.
The one exception is cdn/office-js/, which holds a whole repository tree unmodified, by the owner's word.
"""
import argparse
import collections
import json
import math
import io
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

sys.path.insert(0,str(Path(__file__).resolve().parent))
import catalog_documents
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


def git_checkout_name(entry):
    """The folder pub's own cache gives a git dependency's checkout: <repository>-<revision>."""
    return entry["url"].rstrip("/").rsplit("/",1)[-1].removesuffix(".git")+"-"+entry["revision"]


def run_git(*args,cwd=None):
    done=subprocess.run(["git",*args],cwd=cwd,capture_output=True)
    if done.returncode:
        raise SystemExit("git "+" ".join(args)+": "+done.stderr.decode("utf-8","replace").strip())
    return done.stdout


def filter_packages(destination,paths):
    """filter_hosted for packages that sit at `paths` inside a repository checkout."""
    dropped=collections.Counter()
    deleted=[]
    for path in paths:
        package=destination/path
        for name in DROP_DIRECTORIES:
            directory=package/name
            if directory.is_dir():
                dropped[name]+=sum(1 for f in directory.rglob("*") if f.is_file())
                shutil.rmtree(directory)
        for file in sorted(f for f in package.rglob("*") if f.is_file()):
            kind=flagged_kind(file.read_bytes(),file.name)
            if kind:
                dropped[kind]+=1
                deleted.append(dict(path=file.relative_to(destination).as_posix(),kind=kind))
                file.unlink()
    return dict(sorted(dropped.items())),deleted


def mirror_git(root,gits):
    """Writes each git dependency's package paths at its revision under root/<repo>-<revision>/; returns what was kept.

    The tree is `git archive`'s, so it holds exactly the revision's files. Each package then loses what
    compiling never reads, and any file a scanner would refuse, as a hosted package does.
    """
    if root.exists(): shutil.rmtree(root)
    kept=[]
    sources={}
    for entry in gits:
        sources.setdefault((entry["url"],entry["revision"]),[]).append(entry)
    for (url,revision),entries in sorted(sources.items()):
        name=git_checkout_name(entries[0])
        paths=sorted({(entry["path"] or ".") for entry in entries})
        with tempfile.TemporaryDirectory() as scratch:
            run_git("init","--quiet",scratch)
            run_git("fetch","--quiet","--depth","1",url,revision,cwd=scratch)
            if run_git("rev-parse","FETCH_HEAD",cwd=scratch).decode().strip()!=revision:
                raise SystemExit(f"{url} did not give {revision}")
            archive=run_git("archive","--format=tar",revision,"--",*paths,cwd=scratch)
        destination=root/name
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            for member in tar:
                if member.isfile():
                    target=destination/member.name
                    target.parent.mkdir(parents=True,exist_ok=True)
                    target.write_bytes(tar.extractfile(member).read())
        for path in paths:
            if not (destination/path/"pubspec.yaml").is_file():
                raise SystemExit(f"{url} at {revision} has no package at {path}")
        for entry in entries:
            kept.append(dict(package=entry["package"],checkout=name,path=entry["path"] or "."))
        dropped,deleted=filter_packages(destination,paths)
        if deleted: print("Deleted from",name+":",", ".join(item["path"] for item in deleted))
    return sorted(kept,key=lambda item:item["package"])


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
    dropped,deleted=filter_hosted(target/"pub"/"hosted")
    directory=target/"pub"/"hosted"/"pub.dev"
    packages=sorted(item.name for item in directory.glob("*") if item.is_dir())
    gits=git_dependencies(args.lock)
    gitkept=mirror_git(target/"pub"/"git",gits)
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
        # The documents the extensions' shipped catalog snapshots are made from, pinned by tools/catalog_documents.json: an offline
        # build regenerates each snapshot from them. They are kept as published, so git must not convert their line endings.
        catalog_documents.mirror(target,catalog_documents.read_list())
        text=attributes.read_text(encoding="utf-8")
        if "catalogs/**" not in text:
            attributes.write_text(text+("" if text.endswith("\n") else "\n")+"catalogs/** -text\n",encoding="utf-8",newline="\n")
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
                status="complete",pubPackages=packages,gitDependencies=gits,gitPackages=gitkept,
                gitDependenciesNote="source trees at the locked revision under pub/git/<repository>-<revision>/, named as pub names its checkout",
                dropped=dict(directories=list(DROP_DIRECTORIES),counts=dropped,deletedFlaggedFiles=deleted),
                cdnFiles=files,
                officeJs=None if office_pin is None else dict(repository=office_pin["repository"],commit=office_pin["commit"],
                    fileVersion=office_pin["file_version"],carriedByTheHost=office_pin["carried"],files=office),
                fallbackFonts=dict(count=0 if args.skip_cdn else len(pinned),bytes=fallback),
                desktopPackages=list(DESKTOP_PACKAGES),desktopLinuxSystemPackages=list(DESKTOP_LINUX_SYSTEM))
    (target/"manifest.json").write_text(json.dumps(report,indent=2)+"\n")
    print("Mirrored",len(packages),"pub packages,",len(gits),"git dependencies and",len(files),"CDN files;",
          "dropped",sum(dropped.values()),"files")
if __name__=="__main__": main()
