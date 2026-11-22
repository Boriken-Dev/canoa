import 'dart:convert';
import 'dart:typed_data';

import '../../served/host_files.dart';

/// What a size in bytes reads as.
String sizeText(int bytes) {
  if (bytes < 1024) return '$bytes bytes';
  if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(bytes < 10 * 1024 ? 1 : 0)} KiB';
  return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MiB';
}

/// UTF-8 [bytes] as text, a character cut off at the end left out and anything that is not UTF-8 shown as U+FFFD.
String decodeFileText(List<int> bytes) {
  var end = bytes.length;
  // A cut can leave the lead of a character and part of its tail: drop at most three bytes.
  for (var back = 1; back <= 3 && back <= end; back++) {
    final byte = bytes[end - back];
    if (byte & 0xC0 == 0x80) continue;
    final need = byte >= 0xF0
        ? 4
        : byte >= 0xE0
        ? 3
        : byte >= 0xC0
        ? 2
        : 1;
    if (need > back) end -= back;
    break;
  }
  return utf8.decode(bytes.sublist(0, end), allowMalformed: true);
}

/// The file a viewer is asked to show: what the host read of it, and the few things every viewer wants of that.
final class FileSample {
  FileSample(this.read);

  static final Expando<FileSample> _samples = Expando<FileSample>('FileSample');

  /// The sample of [read]; the same object for the same read, so what a viewer worked out of it is worked out once.
  factory FileSample.of(HostFileRead read) => _samples[read] ??= FileSample(read);

  final HostFileRead read;
  final Map<String, Object?> _memo = {};

  String get path => read.path;
  String get name => read.name;
  int get size => read.size;

  /// `text`, `image` or `binary`, as the host sorted it.
  String get kind => read.kind;
  String? get mime => read.mime;
  bool get truncated => read.truncated;
  Uint8List? get bytes => read.bytes;

  /// The name's last extension in lower case, without the dot; empty for none (a leading dot is part of the name).
  late final String extension = () {
    final cut = name.lastIndexOf('.');
    return cut <= 0 ? '' : name.substring(cut + 1).toLowerCase();
  }();

  /// Whether the host read some text of it.
  bool get isText => kind == 'text' && bytes != null;

  /// The text read, or null for a file that is not text.
  late final String? text = isText ? decodeFileText(bytes!) : null;

  /// A value a viewer worked out of this file, computed once per [key].
  T memo<T>(String key, T Function() compute) => (_memo.putIfAbsent(key, compute)) as T;
}

/// Works out which type a file is, as a MIME type: the host's own word when it is specific, then the extension, then a look at the
/// first bytes. Extensions and sniffers are added to, so an extension that brings a file type needs no change here.
final class FileTypes {
  FileTypes({Map<String, String>? extensions, List<String? Function(FileSample)>? sniffers})
    : _extensions = {...standardExtensions, ...?extensions},
      _sniffers = [...?sniffers, ...standardSniffers];

  /// The type a text file of no known kind is.
  static const String plain = 'text/plain';

  /// The type of a binary file nothing recognised.
  static const String unknown = 'application/octet-stream';

  /// The type of an image the host could draw but whose format is not one of the sniffed ones.
  static const String anImage = 'image/x-unknown';

  static const Map<String, String> standardExtensions = {
    'md': 'text/markdown',
    'markdown': 'text/markdown',
    'json': 'application/json',
    'yaml': 'application/yaml',
    'yml': 'application/yaml',
    'toml': 'application/toml',
    'diff': 'text/x-diff',
    'patch': 'text/x-diff',
    'png': 'image/png',
    'jpg': 'image/jpeg',
    'jpeg': 'image/jpeg',
    'gif': 'image/gif',
    'webp': 'image/webp',
    'txt': plain,
    'log': plain,
  };

  static final List<String? Function(FileSample)> standardSniffers = [_imageMagic, _diffLooking, _jsonLooking];

  final Map<String, String> _extensions;
  final List<String? Function(FileSample)> _sniffers;

  /// Makes files named `*.[extension]` of type [mime].
  void addExtension(String extension, String mime) => _extensions[extension.toLowerCase().replaceFirst('.', '')] = mime.toLowerCase();

  /// Reads the first bytes of a file with [sniffer]: it answers a MIME type, or null when it does not know. A sniffer added here
  /// is asked before the standard ones.
  void addSniffer(String? Function(FileSample) sniffer) => _sniffers.insert(0, sniffer);

  /// [sample]'s MIME type.
  String typeOf(FileSample sample) {
    final said = _generic(sample.mime);
    if (said != null) return said;
    final byName = _extensions[sample.extension];
    if (byName != null) return byName;
    for (final sniff in _sniffers) {
      final found = sniff(sample);
      if (found != null) return found;
    }
    return switch (sample.kind) {
      'text' => plain,
      'image' => anImage,
      _ => unknown,
    };
  }

  /// A host's MIME type without its parameters, or null when it says nothing a name or the bytes would not say better.
  static String? _generic(String? mime) {
    if (mime == null) return null;
    final bare = mime.split(';').first.trim().toLowerCase();
    return bare.isEmpty || bare == unknown || bare == plain ? null : bare;
  }

  static String? _imageMagic(FileSample sample) {
    final b = sample.bytes;
    if (b == null || b.length < 12) return null;
    bool starts(List<int> text, [int at = 0]) {
      for (var i = 0; i < text.length; i++) {
        if (b[at + i] != text[i]) return false;
      }
      return true;
    }

    if (starts(const [0x89, 0x50, 0x4e, 0x47])) return 'image/png';
    if (starts(const [0xff, 0xd8, 0xff])) return 'image/jpeg';
    if (starts('GIF8'.codeUnits)) return 'image/gif';
    if (starts('RIFF'.codeUnits) && starts('WEBP'.codeUnits, 8)) return 'image/webp';
    return null;
  }

  static final RegExp _hunk = RegExp(r'^@@ -\d+(,\d+)? \+\d+(,\d+)? @@', multiLine: true);

  static String? _diffLooking(FileSample sample) {
    final text = sample.text;
    if (text == null) return null;
    final head = text.length > 4000 ? text.substring(0, 4000) : text;
    final git = head.startsWith('diff --git ') || head.startsWith('Index: ');
    final headers = RegExp(r'^--- .*\n\+\+\+ ', multiLine: true).hasMatch(head);
    return (git || headers) && _hunk.hasMatch(head) ? 'text/x-diff' : null;
  }

  static String? _jsonLooking(FileSample sample) {
    final text = sample.text;
    if (text == null || sample.truncated) return null;
    final first = text.trimLeft();
    if (first.isEmpty || (first[0] != '{' && first[0] != '[')) return null;
    try {
      jsonDecode(text);
      return 'application/json';
    } on FormatException {
      return null;
    }
  }
}
