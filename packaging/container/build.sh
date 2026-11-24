#!/bin/sh
# Build Canoa's image from packaging/container/Containerfile, and copy the host program and the site out of it.
#
#   sh packaging/container/build.sh [--tag NAME] [--out DIR] [--no-out] [--print] [-- ENGINE BUILD ARGS...]
#
#   --tag NAME   the image's name (default canoa)
#   --out DIR    where `canoa` and `site/` are copied (default ./out; must not exist or be empty)
#   --no-out     build the image only
#   --print      print the build commands and run nothing
#
# Environment: CONTAINER_ENGINE (podman, or docker), FLUTTER_COMMIT and DEPS_COMMIT (override the Containerfile's pins),
# FLUTTER_REPOSITORY and DEPS_REPOSITORY (another place holding the same commits), FLUTTER_STORAGE_BASE_URL (a mirror of
# Flutter's engine storage: passed to the build as a build argument and read by the flutter stage's `flutter precache`).
# Arguments after `--` go to every build command (for example `--platform linux/amd64`).
set -eu

engine=${CONTAINER_ENGINE:-podman}
tag=canoa
out=out
copy=1
print=0
while [ $# -gt 0 ]; do
    case "$1" in
        --tag) tag=$2; shift 2 ;;
        --out) out=$2; shift 2 ;;
        --no-out) copy=0; shift ;;
        --print) print=1; shift ;;
        --) shift; break ;;
        *) echo "build.sh: unknown argument $1" >&2; exit 2 ;;
    esac
done

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

args=
for name in FLUTTER_COMMIT DEPS_COMMIT FLUTTER_REPOSITORY DEPS_REPOSITORY FLUTTER_STORAGE_BASE_URL; do
    eval "value=\${$name:-}"
    [ -z "$value" ] || args="$args --build-arg $name=$value"
done

# Every command, so --print and the run are the same text.
build() {
    # shellcheck disable=SC2086
    set -- "$engine" build -f packaging/container/Containerfile $args "$@"
    if [ "$print" = 1 ]; then echo "$*"; else "$@"; fi
}

build --target runtime -t "$tag" "$@" .
[ "$copy" = 1 ] || exit 0
build --target out -t "$tag-out" "$@" .
[ "$print" = 0 ] || { echo "$engine create/cp $tag-out into $out"; exit 0; }

if [ -e "$out" ] && [ -n "$(ls -A "$out")" ]; then
    echo "build.sh: $out is not empty" >&2
    exit 1
fi
mkdir -p "$out"
id=$("$engine" create "$tag-out" /canoa)
"$engine" cp "$id:/canoa" "$out/canoa"
"$engine" cp "$id:/site" "$out/site"
"$engine" rm "$id" >/dev/null
echo "build.sh: image $tag; canoa and site/ in $out"
