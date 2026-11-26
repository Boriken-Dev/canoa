/// What a person types as a catalog's source, written as the full URI canoa.cfg holds. A path becomes a `file:` URI, `owner/repo` and
/// a GitHub address become the `git+https` address of the repository (a tree address keeps its folder and ref), and anything with a
/// scheme is left as typed.
///
/// Only a Store type whose catalogs are read through the content sources takes this: a type whose own reader still takes its own
/// grammar (`owner/repo`, `<repository>@<ref>#<file>`) keeps what was typed.
library;

final _scheme = RegExp(r'^[A-Za-z][A-Za-z0-9+.-]+:');
final _drive = RegExp(r'^[A-Za-z]:[\\/]');
final _ownerRepo = RegExp(r'^[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*$');
final _github = RegExp(r'^https?://(?:www\.)?github\.com/([^/\s]+)/([^/\s?#]+?)(?:\.git)?(?:/(?:tree|blob)/([^/\s?#]+)(?:/([^\s?#]*))?)?/?$');

String _fileUri(String path) {
  final slashed = path.replaceAll('\\', '/');
  final encoded = slashed.replaceAll('%', '%25').replaceAll(' ', '%20').replaceAll('#', '%23').replaceAll('?', '%3F');
  if (encoded.startsWith('//')) return 'file:$encoded';
  return encoded.startsWith('/') ? 'file://$encoded' : 'file:///$encoded';
}

/// [typed] as the URI to write. [uris] is whether the type reads its catalogs through the content sources.
String sourceUri(String typed, {required bool uris}) {
  final text = typed.trim();
  if (!uris || text.isEmpty) return text;
  final github = _github.firstMatch(text);
  if (github != null) {
    final owner = github.group(1);
    final repo = github.group(2);
    final ref = github.group(3);
    final path = github.group(4);
    final folder = path == null || path.isEmpty ? '' : '!/${path.replaceAll(RegExp(r'/+$'), '')}';
    return 'git+https://github.com/$owner/$repo.git$folder${ref == null ? '' : '?ref=$ref'}';
  }
  if (_drive.hasMatch(text) || text.startsWith('/') || text.startsWith(r'\\')) return _fileUri(text);
  if (_scheme.hasMatch(text)) return text;
  if (_ownerRepo.hasMatch(text)) return 'git+https://github.com/$text.git';
  return text;
}
