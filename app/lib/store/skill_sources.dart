/// Where skills are looked for, written the way `npx skills add` takes them: a GitHub `owner/repo`, optionally with a folder
/// (`owner/repo/skills/pdf`), a GitHub or GitLab tree URL, a git URL, or a folder on this computer; `#ref` or `@v1.2.3` pins a
/// branch, tag or commit. The source says only where to look: which skills it holds is read from the `SKILL.md` files in it.
library;

import '../profile/aos_support.dart' show holdsPassword;

/// The source Canoa offers first, until the person removes it: the skills Anthropic publishes.
const defaultSkillSourceLabel = 'anthropics';
const defaultSkillSourceSpec = 'anthropics/skills';

/// A place that holds skills, as the person wrote it.
final class SkillSource {
  const SkillSource._({required this.spec, required this.repo, required this.ref, required this.path, required this.only, required this.local});

  /// What the person wrote, trimmed.
  final String spec;

  /// What git fetches: a URL, or a folder on this computer.
  final String repo;

  /// The branch, tag or commit to read; null reads the repository's head.
  final String? ref;

  /// The folder of the repository to look in; empty looks in all of it.
  final String path;

  /// The one skill the person asked for by name (`owner/repo@name`), or null for every skill the source holds.
  final String? only;

  /// Whether [repo] is a folder on this computer.
  final bool local;

  static final _control = RegExp(r'[\u0000-\u001f\u007f]');
  static final _owner = RegExp(r'^[A-Za-z0-9][A-Za-z0-9_.-]*$');
  static final _version = RegExp(r'^v?\d+(\.\d+)*([-+][0-9A-Za-z.+-]*)?$');
  static final _commit = RegExp(r'^([0-9a-fA-F]{40}|[0-9a-fA-F]{64})$');
  static final _archive = RegExp(r'\.(zip|tar|tgz|tar\.gz|tar\.bz2|tar\.xz)$', caseSensitive: false);

  /// Reads [text]. Throws [FormatException] saying what is wrong, never quoting more than the form asked for.
  static SkillSource parse(String text) {
    final spec = text.trim();
    if (spec.isEmpty) throw const FormatException('Name a source: owner/repo, a git URL or a folder');
    if (_control.hasMatch(spec)) throw const FormatException('A source is one line of text');
    if (holdsPassword(spec)) throw const FormatException('A source with a password in it is not kept: use a credential helper');
    final lower = spec.toLowerCase();
    if (lower.contains('/.well-known/')) {
      throw const FormatException('A site that lists its skills at /.well-known is not supported yet: name its git repository');
    }
    if ((lower.startsWith('http://') || lower.startsWith('https://')) && (lower.endsWith('/skill.md') || _archive.hasMatch(lower))) {
      throw const FormatException('A link to one SKILL.md or an archive is not supported: name the git repository that holds it');
    }
    if (_isFolder(spec)) {
      return SkillSource._(spec: spec, repo: spec, ref: null, path: '', only: null, local: true);
    }
    var rest = spec;
    String? ref;
    String? only;
    var prefix = '';
    for (final known in const ['github:', 'gitlab:']) {
      if (rest.toLowerCase().startsWith(known)) {
        prefix = known;
        rest = rest.substring(known.length);
      }
    }
    final isUrl = prefix.isEmpty && (rest.contains('://') || rest.startsWith('git+') || RegExp(r'^[\w.-]+@[\w.-]+:').hasMatch(rest));
    // `#ref` ends the text; then `@x` after the last `/` or `:` is a pin when it looks like a version or a commit, else a skill.
    final hash = rest.lastIndexOf('#');
    if (hash >= 0) {
      ref = rest.substring(hash + 1);
      rest = rest.substring(0, hash);
      if (ref.isEmpty) throw const FormatException('Nothing follows the #: name a branch, a tag or a commit');
      final at = _suffixAt(ref);
      if (at >= 0) {
        only = ref.substring(at + 1);
        ref = ref.substring(0, at);
      }
    } else {
      final at = _suffixAt(rest);
      if (at >= 0) {
        final tail = rest.substring(at + 1);
        rest = rest.substring(0, at);
        if (_version.hasMatch(tail) || _commit.hasMatch(tail)) {
          ref = tail;
        } else {
          only = tail;
        }
      }
    }
    if (only != null && (only.isEmpty || !RegExp(r'^[A-Za-z0-9][A-Za-z0-9_.-]*$').hasMatch(only))) {
      throw const FormatException('A skill named after @ is a skill name: letters, digits, - _ and .');
    }
    if (ref != null && (ref.isEmpty || ref.startsWith('-') || ref.contains('..') || ref.contains(' '))) {
      throw const FormatException('That is not a branch, tag or commit');
    }
    String repo;
    var path = '';
    if (isUrl) {
      repo = rest.startsWith('git+') ? rest.substring(4) : rest;
      final tree = RegExp(r'^(https://(?:github\.com|gitlab\.com)/[^/]+/[^/]+?)(?:\.git)?/(?:-/)?tree/([^/]+)(?:/(.+))?$').firstMatch(repo);
      if (tree != null) {
        repo = tree.group(1)!;
        ref ??= tree.group(2);
        path = tree.group(3) ?? '';
      }
      if (!RegExp(r'^(https?://|ssh://|git://|file://|[\w.-]+@[\w.-]+:)').hasMatch(repo)) {
        throw const FormatException('A source git may fetch starts with https://, ssh://, git://, file:// or user@host:');
      }
    } else {
      final parts = rest.split('/');
      if (parts.length < 2 || !_owner.hasMatch(parts[0]) || !_owner.hasMatch(parts[1].replaceAll(RegExp(r'\.git$'), ''))) {
        throw const FormatException('Write a source as owner/repo, a git URL, or a folder on this computer');
      }
      final host = prefix == 'gitlab:' ? 'gitlab.com' : 'github.com';
      repo = 'https://$host/${parts[0]}/${parts[1].replaceAll(RegExp(r'\.git$'), '')}.git';
      path = parts.skip(2).join('/');
    }
    path = path.replaceAll(RegExp(r'/+$'), '');
    if (path.isNotEmpty && !_goodPath(path)) throw const FormatException('A folder in a source is a relative path with no .. in it');
    if (repo.length > 2048) throw const FormatException('That source is too long');
    return SkillSource._(spec: spec, repo: repo, ref: ref, path: path, only: only, local: false);
  }

  /// Where an `@` that starts a pin or a skill name may be: after the last `/`, or the last `:` of an scp-like address.
  static int _suffixAt(String text) {
    final slash = text.lastIndexOf('/');
    final colon = text.lastIndexOf(':');
    final from = (slash > colon ? slash : colon) + 1;
    final at = text.indexOf('@', from);
    return at;
  }

  static bool _isFolder(String text) {
    if (text.startsWith('/') || text.startsWith('./') || text.startsWith('../') || text.startsWith(r'.\') || text.startsWith(r'..\')) return true;
    if (RegExp(r'^[A-Za-z]:[\\/]').hasMatch(text) || text.startsWith(r'\\')) return true;
    return text.toLowerCase().startsWith('file://');
  }

  static bool _goodPath(String path) =>
      !path.contains(r'\') && path.split('/').every((part) => part.isNotEmpty && part != '.' && part != '..' && !_control.hasMatch(part));

  /// Where it is, for the person to read before anything is fetched.
  String get shown {
    if (local) return repo;
    final bare = repo.replaceAll(RegExp(r'\.git$'), '').replaceFirst(RegExp(r'^https://'), '');
    return '$bare${ref == null ? '' : ' @ $ref'}${path.isEmpty ? '' : ' in $path'}';
  }

  /// A name for this source in canoa.cfg: a letter, then letters, digits, `-` or `_`, at most 64.
  String get label {
    final base = local ? repo : repo.replaceAll(RegExp(r'\.git$'), '').replaceFirst(RegExp(r'^[a-z+]+://[^/]*/'), '').replaceFirst(RegExp(r'^[^/@]+@[^:]+:'), '');
    final parts = base.split(RegExp(r'[\\/]')).where((part) => part.isNotEmpty).toList();
    final tail = parts.length >= 2 && !local ? '${parts[parts.length - 2]}-${parts.last}' : parts.lastOrNull ?? 'skills';
    var out = tail.replaceAll(RegExp(r'[^A-Za-z0-9_-]'), '-');
    if (out.isEmpty || !RegExp(r'^[A-Za-z]').hasMatch(out)) out = 'skills-$out';
    return out.length > 64 ? out.substring(0, 64) : out;
  }
}
