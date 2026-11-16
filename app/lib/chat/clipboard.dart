import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:pasteboard/pasteboard.dart';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show ClipboardReader;

import 'attachments.dart' show maxAttachedImage;
import 'image_fit.dart';

/// Whether [bytes] is a Windows bitmap file, which is how the clipboard hands over a screenshot there.
bool isBitmap(Uint8List bytes) => bytes.length > 2 && bytes[0] == 0x42 && bytes[1] == 0x4d;

/// [bytes] as PNG when they are a bitmap, which is uncompressed and so often larger than an image can be here; anything else
/// as it is. An image still over the cap is made smaller ([fitImage]). Null when a bitmap cannot be decoded, or an image
/// cannot be made to fit.
Future<Uint8List?> pngFromClipboard(Uint8List bytes) async {
  final png = await _png(bytes);
  if (png == null || png.length <= maxAttachedImage) return png;
  return (await fitImage(png))?.bytes;
}

Future<Uint8List?> _png(Uint8List bytes) async {
  if (!isBitmap(bytes)) return bytes;
  try {
    final codec = await ui.instantiateImageCodec(bytes);
    try {
      final frame = await codec.getNextFrame();
      final data = await frame.image.toByteData(format: ui.ImageByteFormat.png);
      frame.image.dispose();
      return data?.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes);
    } finally {
      codec.dispose();
    }
  } on Object {
    return null;
  }
}

/// The system's clipboard: an image as PNG bytes, files by their paths, and text.
final class SystemClipboard implements ClipboardReader {
  const SystemClipboard({this.readImage = _pasteboardImage});

  /// Where the clipboard's image comes from, as its platform gives it.
  final Future<Uint8List?> Function() readImage;

  static Future<Uint8List?> _pasteboardImage() => Pasteboard.image;

  @override
  Future<Uint8List?> image() async {
    try {
      final raw = await readImage();
      return raw == null ? null : await pngFromClipboard(raw);
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
