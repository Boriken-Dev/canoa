import 'dart:typed_data';

import 'package:material_ui/material_ui.dart';

import '../../l10n/app_text.dart';
import 'file_sample.dart';
import 'file_viewer.dart';

/// The most pixels an image may declare for the preview to decode it: a small file can claim dimensions that take gigabytes to draw.
const int imagePixels = 16 * 1000 * 1000;

/// The longest side an image is decoded at; a larger one is scaled down as it is read.
const int imageSide = 1600;

/// The width and height a PNG, GIF, JPEG or WebP declares in its header, read from the bytes with nothing decoded, or null for bytes
/// that are none of them or are cut before the size. Done here, not by the engine, so it is the same in every build and costs nothing.
({int width, int height})? declaredImageSize(Uint8List b) {
  int be16(int at) => (b[at] << 8) | b[at + 1];
  int le16(int at) => b[at] | (b[at + 1] << 8);
  int be32(int at) => (b[at] << 24) | (b[at + 1] << 16) | (b[at + 2] << 8) | b[at + 3];
  bool starts(List<int> text, [int at = 0]) {
    if (b.length < at + text.length) return false;
    for (var i = 0; i < text.length; i++) {
      if (b[at + i] != text[i]) return false;
    }
    return true;
  }

  // PNG: the header chunk follows the signature.
  if (starts(const [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) {
    if (b.length < 24 || !starts('IHDR'.codeUnits, 12)) return null;
    return (width: be32(16), height: be32(20));
  }
  if (starts('GIF87a'.codeUnits) || starts('GIF89a'.codeUnits)) {
    return b.length < 10 ? null : (width: le16(6), height: le16(8));
  }
  if (starts(const [0xff, 0xd8])) {
    var at = 2;
    while (at + 4 <= b.length) {
      if (b[at] != 0xff) return null;
      final marker = b[at + 1];
      if (marker == 0xff) {
        at++;
        continue;
      }
      if (marker == 0xd9 || marker == 0xda) return null;
      if (marker == 0x01 || (marker >= 0xd0 && marker <= 0xd8)) {
        at += 2;
        continue;
      }
      final isFrame = marker >= 0xc0 && marker <= 0xcf && marker != 0xc4 && marker != 0xc8 && marker != 0xcc;
      if (isFrame) {
        return at + 9 > b.length ? null : (width: be16(at + 7), height: be16(at + 5));
      }
      at += 2 + be16(at + 2);
    }
    return null;
  }
  if (starts('RIFF'.codeUnits) && starts('WEBP'.codeUnits, 8)) {
    if (starts('VP8 '.codeUnits, 12)) {
      if (b.length < 30 || !starts(const [0x9d, 0x01, 0x2a], 23)) return null;
      return (width: le16(26) & 0x3fff, height: le16(28) & 0x3fff);
    }
    if (starts('VP8L'.codeUnits, 12)) {
      if (b.length < 25 || b[20] != 0x2f) return null;
      final bits = b[21] | (b[22] << 8) | (b[23] << 16) | (b[24] << 24);
      return (width: (bits & 0x3fff) + 1, height: ((bits >> 14) & 0x3fff) + 1);
    }
    if (starts('VP8X'.codeUnits, 12)) {
      if (b.length < 30) return null;
      return (width: (b[24] | (b[25] << 8) | (b[26] << 16)) + 1, height: (b[27] | (b[28] << 8) | (b[29] << 16)) + 1);
    }
  }
  return null;
}

/// An image drawn only once its header says it is small enough: a file of a few bytes can declare dimensions that take gigabytes to
/// decode. It reads the declared size, refuses past [imagePixels], and decodes no larger than [imageSide] on its long side.
class ImagePreview extends StatelessWidget {
  const ImagePreview({super.key, required this.bytes});

  final Uint8List bytes;

  @override
  Widget build(BuildContext context) {
    final size = declaredImageSize(bytes);
    if (size == null || size.width < 1 || size.height < 1) {
      return Text(context.t('files.image.no_size'), key: const ValueKey('files image failed'));
    }
    if (size.width * size.height > imagePixels) {
      return Text(context.t('files.image.huge', {'width': size.width, 'height': size.height}), key: const ValueKey('files too large'));
    }
    final wide = size.width >= size.height;
    return Image.memory(
      bytes,
      key: const ValueKey('files image'),
      fit: BoxFit.contain,
      cacheWidth: wide && size.width > imageSide ? imageSide : null,
      cacheHeight: !wide && size.height > imageSide ? imageSide : null,
      errorBuilder: (context, _, _) => Text(context.t('files.image.failed')),
    );
  }
}

/// A PNG, JPEG, GIF or WebP, drawn within the room it is given.
class ImageViewer extends FileViewer {
  const ImageViewer();

  static const String viewerId = 'image';

  @override
  String get id => viewerId;
  @override
  String get labelKey => 'files.viewer.image';
  @override
  Set<String> get mimeTypes => const {'image/*'};
  @override
  int get priority => 10;

  @override
  bool canShow(FileSample sample) => sample.kind == 'image' && sample.bytes != null;

  @override
  Widget build(BuildContext context, FileViewRequest request) =>
      SingleChildScrollView(padding: const EdgeInsets.all(16), child: ImagePreview(key: ValueKey('image ${request.sample.path}'), bytes: request.sample.bytes!));
}
