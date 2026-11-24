import subprocess
import tempfile
import unittest
from pathlib import Path

import mirror_deps


def git(repo: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.test", "-C", str(repo), *args],
        check=True, capture_output=True, text=True,
    )
    return done.stdout.strip()


class GitDependencyTests(unittest.TestCase):
    def test_a_git_package_is_carried_as_the_source_the_revision_has(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            repo = tmp / "lib.git"
            package = repo / "bindings" / "dart" / "lib"
            (package / "lib").mkdir(parents=True)
            (package / "test").mkdir()
            (package / "pubspec.yaml").write_text("name: lib\n")
            (package / "lib" / "lib.dart").write_text("// code\n")
            (package / "test" / "lib_test.dart").write_text("// test\n")
            (package / "key.p12").write_bytes(bytes(range(256)) * 4)
            (repo / "elsewhere.txt").write_text("not a package path\n")
            git(repo.parent, "init", "--quiet", str(repo))
            git(repo, "add", "-A")
            git(repo, "commit", "--quiet", "-m", "one")
            revision = git(repo, "rev-parse", "HEAD")
            entry = dict(package="lib", url=repo.as_uri(), path="bindings/dart/lib", ref=revision, revision=revision)
            root = tmp / "pub" / "git"
            kept = mirror_deps.mirror_git(root, [entry])
            checkout = root / mirror_deps.git_checkout_name(entry)
            self.assertEqual(kept, [dict(package="lib", checkout=checkout.name, path="bindings/dart/lib")])
            carried = sorted(p.relative_to(checkout).as_posix() for p in checkout.rglob("*") if p.is_file())
            self.assertEqual(carried, ["bindings/dart/lib/lib/lib.dart", "bindings/dart/lib/pubspec.yaml"])

    def test_a_path_that_is_not_a_package_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = Path(tmp)
            repo = tmp / "lib.git"
            git(repo.parent, "init", "--quiet", str(repo))
            (repo / "a.txt").write_text("x\n")
            git(repo, "add", "-A")
            git(repo, "commit", "--quiet", "-m", "one")
            revision = git(repo, "rev-parse", "HEAD")
            entry = dict(package="lib", url=repo.as_uri(), path=".", ref=revision, revision=revision)
            with self.assertRaisesRegex(SystemExit, "no package"):
                mirror_deps.mirror_git(tmp / "out", [entry])


if __name__ == "__main__":
    unittest.main()
