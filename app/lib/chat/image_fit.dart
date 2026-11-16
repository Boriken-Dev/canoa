import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'attachments.dart' show maxAttachedImage;

/// The long edges an image over the cap is tried at, largest first.
const List<int> fitEdges = [2048, 1536];

/// An image made small enough to attach: PNG, the only form `dart:ui` writes.
typedef FittedImage = ({Uint8List bytes, int width, int height});

/// [source] as a PNG of at most [limit] bytes, scaled so its long edge is the first of [edges] that makes it fit and is
/// shorter than its own; null when none does, or when [source] is not an image the platform can decode.
///
/// Only the size is read to decide, and each try decodes straight to its target size, so the full-size picture is never held
/// as pixels and only one scaled copy exists at a time. Call it for an image that is over the cap; one that fits is sent as it
/// is. An animation keeps its first frame.
Future<FittedImage?> fitImage(List<int> source, {int limit = maxAttachedImage, List<int> edges = fitEdges}) async {
  final bytes = source is Uint8List ? source : Uint8List.fromList(source);
  ui.ImmutableBuffer? buffer;
  ui.ImageDescriptor? descriptor;
  try {
    buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
    descriptor = await ui.ImageDescriptor.encoded(buffer);
    final long = math.max(descriptor.width, descriptor.height);
    for (final edge in edges) {
      if (edge >= long) continue;
      final scale = edge / long;
      final width = math.max(1, (descriptor.width * scale).round());
      final height = math.max(1, (descriptor.height * scale).round());
      final codec = await descriptor.instantiateCodec(targetWidth: width, targetHeight: height);
      try {
        final frame = await codec.getNextFrame();
        try {
          final data = await frame.image.toByteData(format: ui.ImageByteFormat.png);
          if (data != null && data.lengthInBytes <= limit) {
            return (
              bytes: data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes),
              width: frame.image.width,
              height: frame.image.height,
            );
          }
        } finally {
          frame.image.dispose();
        }
      } finally {
        codec.dispose();
      }
    }
    return null;
  } on Object {
    return null;
  } finally {
    descriptor?.dispose();
    buffer?.dispose();
  }
}
