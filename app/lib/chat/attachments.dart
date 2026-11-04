import 'dart:convert';

import '../acp/client.dart' show AcpAttachment;

/// What a person adds to a message: an image, text pasted or read with no file behind it, a file on this computer, or a
/// folder named by its path.
enum AttachmentKind { image, text, file, folder }

/// The most that goes with one message. An agent's context is what these fill, so they stay what a person would pick by hand.
const int maxAttachments = 8;
const int maxAttachedText = 1024 * 1024;
const int maxAttachedImage = 5 * 1024 * 1024;

/// An image's type, from what it was chosen as or its name, or null for anything else.
String? imageTypeOf(String name, String? mimeType) {
  if (const {'image/png', 'image/jpeg', 'image/gif', 'image/webp'}.contains(mimeType)) return mimeType;
  final dot = name.lastIndexOf('.');
  return switch (dot < 0 ? '' : name.substring(dot + 1).toLowerCase()) {
    'png' => 'image/png',
    'jpg' || 'jpeg' => 'image/jpeg',
    'gif' => 'image/gif',
    'webp' => 'image/webp',
    _ => null,
  };
}

/// One thing that goes with a message, as Canoa keeps it until it is sent: how it reaches the agent is decided then, by
/// what the agent takes.
final class Attachment {
  Attachment._(this.kind, this.name, {this.mimeType, this.bytes, this.text, this.path, required this.size});

  /// An image, sent whole where the agent takes images; with a [path], linked where it does not.
  factory Attachment.image(String name, String mimeType, List<int> bytes, {String? path}) =>
      Attachment._(AttachmentKind.image, name, mimeType: mimeType, bytes: List.unmodifiable(bytes), path: path, size: bytes.length);

  /// Text with no file behind it, such as a long paste, sent as the agent's embedded context.
  factory Attachment.text(String name, String text, {String? mimeType}) =>
      Attachment._(AttachmentKind.text, name, mimeType: mimeType, text: text, size: utf8.encode(text).length);

  /// A file on this computer, embedded when the agent takes files and it is small enough text, else linked by its path.
  factory Attachment.file(String name, String path, int size, {String? mimeType}) =>
      Attachment._(AttachmentKind.file, name, mimeType: mimeType, path: path, size: size);

  /// A folder on this computer, linked by its path for the agent to read itself.
  factory Attachment.folder(String name, String path) => Attachment._(AttachmentKind.folder, name, path: path, size: 0);

  final AttachmentKind kind;
  final String name;
  final String? mimeType;
  final List<int>? bytes;
  final String? text;

  /// Where it is on this computer, when it is a file or folder there.
  final String? path;

  /// In bytes; 0 for a folder.
  final int size;

  bool get isImage => kind == AttachmentKind.image;

  /// Its `file:` URI, for a link.
  String? get uri => path == null ? null : (kind == AttachmentKind.folder ? Uri.directory(path!) : Uri.file(path!)).toString();

  Map<String, Object?> toJson({String? blob}) => {
    'kind': kind.name,
    'name': name,
    'mimeType': ?mimeType,
    'path': ?path,
    'size': size,
    'text': ?text,
    'blob': ?blob,
  };

  /// Null for anything that is not a whole record; [blob] answers the bytes a record's `blob` names.
  static Attachment? fromJson(Object? json, {List<int>? Function(String blob)? blob}) {
    if (json is! Map || json['name'] is! String || json['size'] is! int) return null;
    final name = json['name'] as String, size = json['size'] as int;
    final mimeType = json['mimeType'] is String ? json['mimeType'] as String : null;
    final path = json['path'] is String ? json['path'] as String : null;
    switch (json['kind']) {
      case 'image':
        final bytes = json['blob'] is String ? blob?.call(json['blob'] as String) : null;
        return bytes == null || mimeType == null ? null : Attachment.image(name, mimeType, bytes, path: path);
      case 'text':
        return json['text'] is String ? Attachment.text(name, json['text'] as String, mimeType: mimeType) : null;
      case 'file':
        return path == null ? null : Attachment.file(name, path, size, mimeType: mimeType);
      case 'folder':
        return path == null ? null : Attachment.folder(name, path);
    }
    return null;
  }
}

/// How an attachment reaches an agent.
enum AttachmentRoute { image, embedded, link, refused }

/// What an agent takes with a prompt, as it said in `initialize`, and whether it can read a file of this computer by a link,
/// which only an agent running here can.
final class PromptTakes {
  const PromptTakes({required this.images, required this.files, this.links = true});
  final bool images, files, links;
}

/// How [attachment] reaches an agent that takes [takes], and in a person's words why, when it is not sent as it is:
/// a file goes as a link when it is too large to embed ([embedLimit] bytes) or the agent takes no files, and nothing is
/// ever left out without saying so.
(AttachmentRoute, String?) routeOf(Attachment attachment, PromptTakes takes, {required int embedLimit, String agent = 'The agent'}) =>
    switch (attachment.kind) {
      AttachmentKind.image when takes.images => (AttachmentRoute.image, null),
      _ when !takes.links && attachment.kind != AttachmentKind.text &&
          (attachment.kind != AttachmentKind.file || !takes.files || attachment.size > embedLimit) =>
        (AttachmentRoute.refused, '$agent runs elsewhere, and cannot read this computer\'s files by a link'),
      AttachmentKind.image when attachment.path != null => (AttachmentRoute.link, '$agent does not take images: it goes as a link to the file'),
      AttachmentKind.image => (AttachmentRoute.refused, '$agent does not take images'),
      AttachmentKind.text when takes.files => (AttachmentRoute.embedded, null),
      AttachmentKind.text => (AttachmentRoute.refused, '$agent does not take files, and this has no file to link to'),
      AttachmentKind.file when !takes.files => (AttachmentRoute.link, '$agent does not take files: it goes as a link'),
      AttachmentKind.file when attachment.size > embedLimit => (AttachmentRoute.link, 'Larger than ${sizeLabel(embedLimit)}: it goes as a link'),
      AttachmentKind.file => (AttachmentRoute.embedded, null),
      AttachmentKind.folder => (AttachmentRoute.link, null),
    };

/// [attachment] as ACP carries it to an agent that takes [takes]. [read] gives a file's bytes; a file that is not UTF-8
/// text goes as a link. Throws [StateError] with a person's reason for one the agent cannot be given.
Future<AcpAttachment> toAcp(Attachment attachment, PromptTakes takes,
    {required int embedLimit, required Future<List<int>> Function(String path) read, String agent = 'The agent'}) async {
  final (route, why) = routeOf(attachment, takes, embedLimit: embedLimit, agent: agent);
  AcpAttachment link() => AcpAttachment.link(attachment.name, attachment.uri!, mimeType: attachment.mimeType,
      size: attachment.kind == AttachmentKind.folder ? null : attachment.size);
  switch (route) {
    case AttachmentRoute.refused:
      throw StateError('${attachment.name}: $why');
    case AttachmentRoute.image:
      return AcpAttachment.image(attachment.name, attachment.mimeType!, attachment.bytes!);
    case AttachmentRoute.link:
      return link();
    case AttachmentRoute.embedded:
      if (attachment.text case final text?) return AcpAttachment.text(attachment.name, text, mimeType: attachment.mimeType);
      final bytes = await read(attachment.path!);
      // It can have grown since it was added.
      if (bytes.length > embedLimit) {
        if (!takes.links) throw StateError('${attachment.name} is larger than ${sizeLabel(embedLimit)}, and $agent cannot read this computer\'s files by a link');
        return link();
      }
      try {
        return AcpAttachment.text(attachment.name, utf8.decode(bytes), mimeType: attachment.mimeType);
      } on FormatException {
        if (!takes.links) throw StateError('${attachment.name} is not text, and $agent cannot read this computer\'s files by a link');
        return link();
      }
  }
}

/// A size as a person reads it.
String sizeLabel(int bytes) => switch (bytes) {
  < 1024 => '$bytes B',
  < 1024 * 1024 => '${(bytes / 1024).toStringAsFixed(bytes < 10 * 1024 ? 1 : 0)} KB',
  _ => '${(bytes / (1024 * 1024)).toStringAsFixed(bytes < 10 * 1024 * 1024 ? 1 : 0)} MB',
};
