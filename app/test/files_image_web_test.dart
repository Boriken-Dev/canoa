// The Files preview's image check in a real browser: the size comes from the header's bytes, and an image that declares more pixels
// than the budget is not drawn.
//
//   flutter test --platform chrome test/files_image_web_test.dart   (CHROME_EXECUTABLE may name Edge or Chromium)
@TestOn('browser')
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:canoa/surfaces/files_surface.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter_test/flutter_test.dart';

/// A one-pixel PNG.
final _png = base64Decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==');

/// The signature and header of a PNG that claims [width] by [height] pixels and holds none.
Uint8List _pngClaiming(int width, int height) {
  int crc(List<int> bytes) {
    var value = 0xffffffff;
    for (final byte in bytes) {
      value ^= byte;
      for (var bit = 0; bit < 8; bit++) {
        value = (value & 1) != 0 ? (value >> 1) ^ 0xedb88320 : value >> 1;
      }
    }
    return (value ^ 0xffffffff) & 0xffffffff;
  }

  List<int> four(int value) => [(value >> 24) & 255, (value >> 16) & 255, (value >> 8) & 255, value & 255];
  final header = [...'IHDR'.codeUnits, ...four(width), ...four(height), 8, 2, 0, 0, 0];
  final idat = [...'IDAT'.codeUnits, 0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01];
  return Uint8List.fromList([
    0x89,
    0x50,
    0x4e,
    0x47,
    0x0d,
    0x0a,
    0x1a,
    0x0a,
    ...four(13),
    ...header,
    ...four(crc(header)),
    ...four(idat.length - 4),
    ...idat,
    ...four(crc(idat)),
    ...four(0),
    ...'IEND'.codeUnits,
    ...four(crc('IEND'.codeUnits)),
  ]);
}

Future<void> _show(WidgetTester tester, Uint8List bytes) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(body: ImagePreview(bytes: bytes)),
    ),
  );
  await tester.pump();
}

void main() {
  testWidgets('a small image is measured and drawn in the browser', (tester) async {
    await _show(tester, _png);
    expect(find.byKey(const ValueKey('files image')), findsOneWidget);
  });

  testWidgets('an image that declares too many pixels is refused from its header in the browser', (tester) async {
    await _show(tester, _pngClaiming(40000, 40000));
    expect(find.byType(Image), findsNothing);
    expect(find.textContaining('too large to draw'), findsOneWidget);
  });
}
