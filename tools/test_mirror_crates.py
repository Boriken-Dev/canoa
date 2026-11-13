import io
import json
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import mirror_crates as mc

GIT = "git+https://example.test/org/lib.git?rev=abc123#abc123"
LOCK = f"""version = 4

[[package]]
name = "app"
version = "0.1.0"

[[package]]
name = "dep"
version = "1.2.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "{'a' * 64}"

[[package]]
name = "lib"
version = "0.0.1"
source = "{GIT}"
"""


def crate_archive(prefix: str, files: dict[str, bytes]) -> bytes:
    out = io.BytesIO()
    with tarfile.open(fileobj=out, mode="w:gz") as tar:
        for path, body in files.items():
            info = tarfile.TarInfo(f"{prefix}/{path}")
            info.size = len(body)
            tar.addfile(info, io.BytesIO(body))
    return out.getvalue()


def write_lock(directory: Path, text: str = LOCK) -> mc.Lock:
    path = directory / "Cargo.lock"
    path.write_text(text, encoding="utf-8")
    return mc.read_lock(path)


def vendored(crates: Path, name: str, checksum: str | None, head: str = "[package]", extra: dict | None = None) -> None:
    folder = crates / name
    folder.mkdir(parents=True)
    (folder / "Cargo.toml").write_bytes((head + "\n").encode())
    files = {"Cargo.toml": mc.sha256((head + "\n").encode())}
    for rel, body in (extra or {}).items():
        (folder / rel).parent.mkdir(parents=True, exist_ok=True)
        (folder / rel).write_bytes(body)
        files[rel] = mc.sha256(body)
    (folder / mc.CHECKSUM).write_text(json.dumps({"files": files, "package": checksum}), encoding="utf-8")


class CoverageTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.dir = Path(self._tmp.name)
        self.lock = write_lock(self.dir)
        self.crates = self.dir / "deps" / "crates"
        vendored(self.crates, "dep-1.2.3", "a" * 64)
        vendored(self.crates, "lib-0.0.1", None, head="# lib at abc123, with what it inherits written out.")

    def check(self, deep=True):
        return mc.problems(self.lock, mc.read_dir(self.crates, deep), deep)

    def test_a_vendor_directory_that_matches_the_lock_has_no_problems(self):
        self.assertEqual(self.check(), [])
        self.assertEqual(self.check(deep=False), [])

    def test_a_missing_package_is_named(self):
        import shutil
        shutil.rmtree(self.crates / "dep-1.2.3")
        self.assertEqual(self.check(), ["crates/dep-1.2.3: missing, Cargo.lock names it"])

    def test_a_changed_package_is_named(self):
        sums = self.crates / "dep-1.2.3" / mc.CHECKSUM
        sums.write_text(json.dumps({"files": {}, "package": "b" * 64}))
        self.assertTrue(any("package checksum" in line for line in self.check(deep=False)))
        self.assertTrue(any("package checksum" in line for line in self.check()))

    def test_an_edited_file_is_found_by_the_deep_check_only(self):
        (self.crates / "dep-1.2.3" / "Cargo.toml").write_bytes(b"[package] # edited\n")
        self.assertEqual(self.check(deep=False), [])
        self.assertTrue(any("SHA-256" in line for line in self.check()))

    def test_a_git_package_at_another_commit_is_named(self):
        (self.crates / "lib-0.0.1" / "Cargo.toml").write_bytes(b"# lib at def456, with what it inherits written out.\n")
        self.assertTrue(any("not lib at abc123" in line for line in self.check(deep=False)))

    def test_a_stale_package_and_a_loose_file_are_named(self):
        vendored(self.crates, "gone-9.9.9", "c" * 64)
        (self.crates / "README.md").write_text("x")
        found = self.check()
        self.assertIn("crates/gone-9.9.9: stale, Cargo.lock does not name it", found)
        self.assertTrue(any("README.md: a file" in line for line in found))

    def test_a_binary_left_in_a_package_is_named(self):
        vendored(self.crates, "dep-1.2.4", "d" * 64, extra={"data.bin": bytes(range(256)) * 8})
        lock = write_lock(self.dir, LOCK.replace("1.2.3", "1.2.4").replace("a" * 64, "d" * 64))
        import shutil
        shutil.rmtree(self.crates / "dep-1.2.3")
        found = mc.problems(lock, mc.read_dir(self.crates))
        self.assertTrue(any("data.bin" in line for line in found), found)


class PackageTests(unittest.TestCase):
    def test_tests_are_dropped_and_a_binary_a_source_includes_becomes_text(self):
        files = {
            "Cargo.toml": b'[package]\nname = "dep"\n\n[[test]]\nname = "t"\n\n[lib]\npath = "src/lib.rs"\n',
            "src/lib.rs": b'pub static T: &[u8] = include_bytes!("table.bin");\n',
            "src/table.bin": b"\x00\x01\x02\xff\xfe" * 100,
            "tests/t.rs": b"fn main() {}\n",
            "benches/b.rs": b"",
            ".github/ci.yml": b"x",
        }
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "dep-1.0.0"
            mc.write_package(target, files, "f" * 64)
            names = {p.relative_to(target).as_posix() for p in target.rglob("*") if p.is_file()}
            self.assertEqual(names, {"Cargo.toml", "src/lib.rs", mc.CHECKSUM})
            self.assertNotIn("[[test]]", (target / "Cargo.toml").read_text())
            self.assertIn("0x00u8,0x01,0x02,0xff,0xfe", (target / "src" / "lib.rs").read_text())
            sums = json.loads((target / mc.CHECKSUM).read_text())
            self.assertEqual(sums["package"], "f" * 64)
            self.assertEqual(set(sums["files"]), {"Cargo.toml", "src/lib.rs"})
            for rel, digest in sums["files"].items():
                self.assertEqual(mc.sha256((target / rel).read_bytes()), digest)
            self.assertEqual(mc.read_dir(target.parent).binaries, {})

    def test_a_windows_import_library_becomes_hex_that_build_rs_writes_out_and_links(self):
        library = b"!<arch>\n" + bytes(range(256)) * 4
        files = {
            "Cargo.toml": b"[package]\nname = \"windows_x86_64_msvc\"\n",
            "build.rs": mc.WINDOWS_BUILD.encode(),
            "src/lib.rs": b"",
            "lib/windows.0.52.0.lib": library,
        }
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "windows_x86_64_msvc-0.52.6"
            mc.write_package(target, files, "f" * 64)
            self.assertFalse((target / "lib" / "windows.0.52.0.lib").exists())
            digits = "".join((target / "lib" / "windows.0.52.0.lib.hex").read_text().split())
            self.assertEqual(bytes.fromhex(digits), library)
            self.assertIn("OUT_DIR", (target / "build.rs").read_text())
            self.assertEqual(mc.read_dir(target.parent).binaries, {})
            changed = dict(files, **{"build.rs": b"fn main() {}\n"})
            with self.assertRaisesRegex(mc.Problem, "no longer links"):
                mc.write_package(Path(tmp) / "windows_x86_64_msvc-0.52.7", changed, "f" * 64)

    def test_a_crate_is_unpacked_from_its_archive_and_checked_against_the_lock(self):
        body = crate_archive("dep-1.2.3", {"Cargo.toml": b"[package]\n", "src/lib.rs": b""})
        crate = mc.Crate("dep", "1.2.3", mc.sha256(body))
        with tempfile.TemporaryDirectory() as tmp, patch("mirror_crates.download", return_value=body):
            mc.materialize_crate(crate, Path(tmp))
            self.assertTrue((Path(tmp) / "dep-1.2.3" / "src" / "lib.rs").is_file())
            with self.assertRaisesRegex(mc.Problem, "refused"):
                mc.fetch_crate(mc.Crate("dep", "1.2.3", "0" * 64))

    def test_workspace_inheritance_is_written_out(self):
        root = {"workspace": {"package": {"version": "1.0.0", "edition": "2021"}, "dependencies": {"x": "2"}}}
        member = {
            "package": {"name": "m", "version": {"workspace": True}, "edition": {"workspace": True}},
            "dependencies": {"x": {"workspace": True, "optional": True}},
        }
        out = mc.normalized_manifest(member, root, Path("r/m"), Path("r"), "repo/m")
        self.assertEqual(out["package"]["version"], "1.0.0")
        self.assertEqual(out["dependencies"]["x"], {"version": "2", "optional": True})


class ConfigTests(unittest.TestCase):
    def test_the_cargo_config_replaces_every_source_and_stays_offline(self):
        with tempfile.TemporaryDirectory() as tmp:
            lock = write_lock(Path(tmp))
            text = mc.config_text(lock, Path(tmp) / "crates")
            config = __import__("tomllib").loads(text)
            self.assertTrue(config["net"]["offline"])
            self.assertEqual(config["source"]["crates-io"]["replace-with"], mc.SOURCE)
            self.assertEqual(config["source"][mc.SOURCE]["directory"], (Path(tmp) / "crates").as_posix())
            git = config["source"]["git+https://example.test/org/lib.git?rev=abc123"]
            self.assertEqual((git["git"], git["rev"], git["replace-with"]), ("https://example.test/org/lib.git", "abc123", mc.SOURCE))

    def test_use_writes_a_cargo_home_outside_the_repository_only_when_covered(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            lock = write_lock(tmp)
            deps = tmp / "deps"
            with self.assertRaisesRegex(mc.Problem, "missing"):
                mc.use(deps, lock, b"x", tmp / "home")
            vendored(deps / "crates", "dep-1.2.3", "a" * 64)
            vendored(deps / "crates", "lib-0.0.1", None, head="# lib at abc123, with what it inherits written out.")
            home = mc.use(deps, lock, b"x", tmp / "home")
            self.assertTrue((home / "config.toml").read_text().startswith(mc.CONFIG_MARK))
            with self.assertRaisesRegex(mc.Problem, "inside the repository"):
                mc.use(deps, lock, b"x", mc.ROOT / "target" / "home")
            (home / "config.toml").write_text("# mine\n")
            with self.assertRaisesRegex(mc.Problem, "left alone"):
                mc.use(deps, lock, b"x", tmp / "home")


if __name__ == "__main__":
    unittest.main()
