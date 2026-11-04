import 'package:flutter/services.dart';
import 'package:pasteboard/pasteboard.dart';

import 'composer.dart';

/// The system's clipboard: an image as PNG bytes, files by their paths, and text.
final class SystemClipboard implements ClipboardReader {
  const SystemClipboard();

  @override
  Future<Uint8List?> image() async {
    try {
      return await Pasteboard.image;
    } on Object {
      // A platform that cannot say has no image to give.
      return null;
    }
  }

  @override
  Future<List<String>> files() async {
    try {
      return await Pasteboard.files();
    } on Object {
      return const [];
    }
  }

  @override
  Future<String?> text() async => (await Clipboard.getData(Clipboard.kTextPlain))?.text;
}
