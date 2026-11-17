/// What the host reads of a batch of catalog entries: its own fetch of each one's files, and what they provide.
library;

import 'overlay_contents.dart';

/// One entry to read: an overlay or a skill, from the git source it names.
final class ContentsRequest {
  const ContentsRequest({required this.id, required this.kind, required this.name, required this.repo, required this.ref, this.path});

  /// What the answer is keyed by.
  final String id;

  /// `overlay` or `skill`.
  final String kind;

  /// The name an overlay installs under, or a skill's own.
  final String name;
  final String repo;
  final String? ref, path;

  Map<String, Object?> toJson() => {'id': id, 'kind': kind, 'name': name, 'repo': repo, 'ref': ?ref, if (path != null && path!.isNotEmpty) 'path': path};
}

/// What reading one entry gave: what it provides, or why it could not be read.
final class ContentsAnswer {
  const ContentsAnswer.read(OverlayContents this.contents) : problem = null;
  const ContentsAnswer.failed(String this.problem) : contents = null;

  final OverlayContents? contents;
  final String? problem;
}

/// Reads entries ahead for the Store. The host does it (`store/read_ahead`), for a served page and the desktop app alike.
abstract interface class ContentsReader {
  /// Reads [requests], and says what each gave; one that cannot be read is a failed answer and the rest go on. Throws only when
  /// the whole batch could not be asked.
  Future<Map<String, ContentsAnswer>> read(List<ContentsRequest> requests);
}
