#!/usr/bin/env python3
"""Populate pub and CDN caches on Canoa's deps branch."""
import argparse
import hashlib
import json
import shutil
import urllib.request
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

INDEX = "https://raw.githubusercontent.com/jose-pr/ai6/main/clients/outer-office/tool/cdn_files.json"
# leanflutter's desktop packages: the tray and the window. A desktop build needs all of these, whichever of them pub resolves.
DESKTOP_PACKAGES = ("tray_manager","window_manager","screen_retriever","screen_retriever_linux","screen_retriever_macos",
                    "screen_retriever_platform_interface","screen_retriever_windows","menu_base")
# What those packages link on Linux, which no pub cache can carry: the manifest says so, for someone building offline.
DESKTOP_LINUX_SYSTEM = ("libayatana-appindicator3-dev","libgtk-3-dev","libx11-dev","libxi-dev")
def fetch(url):
    request=urllib.request.Request(url, headers={"User-Agent":"Canoa-Offline-Mirror"})
    with urllib.request.urlopen(request,timeout=90) as response:
        return response.read()
def digest(data): return hashlib.sha256(data).hexdigest()
def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--source-pub",required=True)
    parser.add_argument("--destination",required=True)
    parser.add_argument("--revision",required=True)
    parser.add_argument("--skip-cdn",action="store_true")
    args=parser.parse_args()
    target=Path(args.destination)
    # git/cache holds a bare clone of each git dependency, which pub checks out from when offline.
    for kind in ("hosted","hosted-hashes","git/cache"):
        src=Path(args.source_pub)/kind
        dst=target/"pub"/kind
        if dst.exists(): shutil.rmtree(dst)
        if src.exists(): shutil.copytree(src,dst,ignore=shutil.ignore_patterns("*.lock"))
    directory=target/"pub"/"hosted"/"pub.dev"
    packages=sorted(item.name for item in directory.glob("*") if item.is_dir())
    clones=sorted(item.name for item in (target/"pub"/"git"/"cache").glob("*") if item.is_dir())
    missing=[name for name in DESKTOP_PACKAGES if not any(package.rsplit("-",1)[0]==name for package in packages)]
    if missing: raise SystemExit("Desktop packages missing from the mirror: "+", ".join(missing))
    files=[]
    if not args.skip_cdn:
        inventory=json.loads(fetch(INDEX))
        fonts=[e for e in inventory["files"] if e["host"]=="fonts.gstatic.com"]
        def copy(e):
            name=e["path"]
            if not name.startswith("/") or ".." in Path(name).parts:
                raise ValueError("Invalid CDN path")
            destination=target/"cdn"/e["host"]/name.lstrip("/")
            if destination.exists() and digest(destination.read_bytes())==e["sha256"]:
                data=destination.read_bytes()
            else:
                data=fetch("https://"+e["host"]+name)
                if digest(data)!=e["sha256"]: raise ValueError("CDN checksum mismatch "+name)
                destination.parent.mkdir(parents=True,exist_ok=True)
                destination.write_bytes(data)
            return dict(host=e["host"],path=name,sha256=digest(data),bytes=len(data))
        with ThreadPoolExecutor(max_workers=12) as workers:
            files=list(workers.map(copy,fonts))
    report=dict(format=1,source="Boriken-Dev/canoa",sourceCommit=args.revision,
                status="complete",pubPackages=packages,gitClones=clones,cdnFiles=files,
                desktopPackages=list(DESKTOP_PACKAGES),desktopLinuxSystemPackages=list(DESKTOP_LINUX_SYSTEM))
    (target/"manifest.json").write_text(json.dumps(report,indent=2)+"\n")
    print("Mirrored",len(packages),"pub packages,",len(clones),"git clones and",len(files),"font assets")
if __name__=="__main__": main()
