// The clipboard's image as the chat gets it: a Windows screenshot arrives as an uncompressed bitmap, which is turned into PNG
// before the size check and before it is labelled as PNG.
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:canoa/chat/attachments.dart';
import 'package:canoa/chat/clipboard.dart';
import 'package:flutter_test/flutter_test.dart';

/// A 32-bit bitmap file of [width] by [height] pixels, a gradient so that it is not trivially empty.
Uint8List _bitmap(int width, int height) {
  final pixels = width * height * 4;
  final file = ByteData(54 + pixels);
  file.setUint8(0, 0x42);
  file.setUint8(1, 0x4d);
  file.setUint32(2, 54 + pixels, Endian.little);
  file.setUint32(10, 54, Endian.little);
  file.setUint32(14, 40, Endian.little);
  file.setInt32(18, width, Endian.little);
  file.setInt32(22, height, Endian.little);
  file.setUint16(26, 1, Endian.little);
  file.setUint16(28, 32, Endian.little);
  file.setUint32(34, pixels, Endian.little);
  for (var y = 0; y < height; y++) {
    for (var x = 0; x < width; x++) {
      final at = 54 + (y * width + x) * 4;
      file.setUint8(at, x % 256);
      file.setUint8(at + 1, y % 256);
      file.setUint8(at + 2, (x + y) % 256);
      file.setUint8(at + 3, 255);
    }
  }
  return file.buffer.asUint8List();
}

const _signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

void main() {
  testWidgets('a screenshot bitmap larger than an image can be here arrives as a PNG that fits', (tester) async {
    final bitmap = _bitmap(1920, 1080);
    expect(bitmap.length, greaterThan(maxAttachedImage), reason: 'as the clipboard hands it over');
    final got = await tester.runAsync(() => SystemClipboard(readImage: () async => bitmap).image());
    expect(got!.sublist(0, 8), _signature);
    expect(got.length, lessThanOrEqualTo(maxAttachedImage));
    final image = await tester.runAsync(() async => (await (await ui.instantiateImageCodec(got)).getNextFrame()).image);
    expect((image!.width, image.height), (1920, 1080));
  });

  testWidgets('a PNG is passed as it is, and what is not an image of any kind is nothing', (tester) async {
    final png = Uint8List.fromList([..._signature, 1, 2, 3]);
    expect(await tester.runAsync(() => SystemClipboard(readImage: () async => png).image()), png);
    expect(await tester.runAsync(() => SystemClipboard(readImage: () async => null).image()), isNull);
    expect(await tester.runAsync(() => SystemClipboard(readImage: () async => throw StateError('no')).image()), isNull);
    final broken = Uint8List.fromList([0x42, 0x4d, 0, 0, 0, 0]);
    expect(await tester.runAsync(() => SystemClipboard(readImage: () async => broken).image()), isNull, reason: 'a bitmap that cannot be read gives nothing');
  });
}
