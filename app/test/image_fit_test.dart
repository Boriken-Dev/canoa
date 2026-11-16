// An image over the attachment cap is made smaller instead of refused: a PNG at a 2048 px, else a 1536 px long edge, decoded
// straight to that size so the full-size pixels are never held; one that fits goes as it is.
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:canoa/chat/clipboard.dart';
import 'package:canoa/chat/image_fit.dart';
import 'package:canoa/session_store.dart';
import 'package:flutter_test/flutter_test.dart';

const _signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

/// A PNG of [width] by [height]: a gradient with noise, so it compresses poorly the way a photograph does.
Future<Uint8List> _png(int width, int height, {int noise = 0x1f}) async {
  final pixels = Uint8List(width * height * 4);
  var seed = 12345;
  for (var y = 0; y < height; y++) {
    for (var x = 0; x < width; x++) {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      final at = (y * width + x) * 4;
      pixels[at] = (x * 255 ~/ width + (seed & noise)) & 0xff;
      pixels[at + 1] = (y * 255 ~/ height + ((seed >> 8) & noise)) & 0xff;
      pixels[at + 2] = ((x + y) * 255 ~/ (width + height) + ((seed >> 16) & noise)) & 0xff;
      pixels[at + 3] = 255;
    }
  }
  final image = await _decode(pixels, width, height);
  final data = await image.toByteData(format: ui.ImageByteFormat.png);
  image.dispose();
  return data!.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes);
}

Future<ui.Image> _decode(Uint8List pixels, int width, int height) async {
  final buffer = await ui.ImmutableBuffer.fromUint8List(pixels);
  final descriptor = ui.ImageDescriptor.raw(buffer, width: width, height: height, pixelFormat: ui.PixelFormat.rgba8888);
  final codec = await descriptor.instantiateCodec();
  final frame = await codec.getNextFrame();
  codec.dispose();
  descriptor.dispose();
  buffer.dispose();
  return frame.image;
}

Future<(int, int)> _size(Uint8List png) async {
  final codec = await ui.instantiateImageCodec(png);
  final frame = await codec.getNextFrame();
  final size = (frame.image.width, frame.image.height);
  frame.image.dispose();
  codec.dispose();
  return size;
}

void main() {
  testWidgets('a 4K PNG over the cap attaches downscaled to a 2048 px edge', (tester) async {
    await tester.runAsync(() async {
      final big = await _png(3840, 2160);
      expect(big.length, greaterThan(maxAttachedImage), reason: 'a 4K screenshot, over the cap as it is');
      final store = SessionStore();
      final session = store.createDraft();
      expect(await store.addAttachmentFitted(session, 'screen.png', big, mimeType: 'image/png'), isNull);
      final attached = session.attachments.single;
      expect(attached.name, 'screen.png');
      expect(attached.mimeType, 'image/png');
      expect(attached.bytes!.sublist(0, 8), _signature);
      expect(attached.bytes!.length, lessThanOrEqualTo(maxAttachedImage));
      final (width, height) = await _size(Uint8List.fromList(attached.bytes!));
      expect(width, 2048, reason: 'the first edge that fits');
      expect(height, 1152, reason: 'in proportion');
    });
  });

  testWidgets('a picture that is still too big at 2048 px is tried at 1536 px', (tester) async {
    await tester.runAsync(() async {
      final big = await _png(3840, 2160);
      final at2048 = await fitImage(big, edges: [2048]);
      expect(at2048, isNotNull);
      final fitted = await fitImage(big, limit: at2048!.bytes.length - 1);
      expect(fitted, isNotNull);
      expect((fitted!.width, fitted.height), (1536, 864));
      expect(fitted.bytes.length, lessThan(at2048.bytes.length));
      expect(await fitImage(big, limit: 1024), isNull, reason: 'nothing at 2048 or 1536 px fits a limit that small');
    });
  });

  testWidgets('what fits is attached as it is, and what cannot be read as an image is refused with the reason', (
    tester,
  ) async {
    await tester.runAsync(() async {
      final store = SessionStore();
      final session = store.createDraft();
      final small = await _png(64, 64);
      expect(await store.addAttachmentFitted(session, 'small.png', small, mimeType: 'image/png'), isNull);
      expect(session.attachments.single.bytes, small, reason: 'not re-encoded');
      final broken = List.filled(maxAttachedImage + 1, 7);
      expect(
        await store.addAttachmentFitted(session, 'broken.png', broken, mimeType: 'image/png'),
        'broken.png is larger than an image can be here, and could not be made smaller.',
      );
      expect(session.attachments, hasLength(1));
    });
  });

  testWidgets('a pasted screenshot over the cap arrives as a PNG that fits', (tester) async {
    await tester.runAsync(() async {
      final big = await _png(3840, 2160);
      final got = await SystemClipboard(readImage: () async => big).image();
      expect(got!.sublist(0, 8), _signature);
      expect(got.length, lessThanOrEqualTo(maxAttachedImage));
      expect((await _size(got)).$1, 2048);
    });
  });

  // The peak resident memory of one way of handling a 4K PNG, in a process of its own so that nothing else sets the peak:
  // CANOA_IMAGE_MEMORY=make:<file> writes the picture, =fit:<file> fits it, =whole:<file> decodes it at its own size.
  final memory = Platform.environment['CANOA_IMAGE_MEMORY'];
  testWidgets('measured: peak memory of fitting a 4K image against decoding it whole', (tester) async {
    await tester.runAsync(() async {
      final colon = memory!.indexOf(':');
      final mode = memory.substring(0, colon);
      final file = File(memory.substring(colon + 1));
      if (mode == 'make') {
        await file.writeAsBytes(await _png(3840, 2160));
        return;
      }
      final bytes = await file.readAsBytes();
      final before = ProcessInfo.maxRss;
      if (mode == 'fit') {
        final fitted = await fitImage(bytes);
        expect(fitted, isNotNull);
      } else {
        final codec = await ui.instantiateImageCodec(bytes);
        final frame = await codec.getNextFrame();
        final data = await frame.image.toByteData(format: ui.ImageByteFormat.png);
        expect(data, isNotNull);
        frame.image.dispose();
        codec.dispose();
      }
      // ignore: avoid_print
      print(
        'MEMORY $mode: ${bytes.length} B in, peak RSS grew ${((ProcessInfo.maxRss - before) / 1048576).toStringAsFixed(1)} MB',
      );
    });
  }, skip: memory == null);
}
