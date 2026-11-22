import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/painting.dart';

import '../brand.dart';
import '../theme/canoa_mark.dart';

/// The sizes of the tray icon's `.ico` entries: Windows picks the one nearest the notification area's small-icon size, which
/// follows the display's scaling.
const List<int> trayIconSizes = [16, 20, 24, 32, 40, 48];

/// The size of the tray icon's PNG, which Linux and macOS take.
const int trayPngSize = 32;

/// The brand's mark at [size] pixels square in [color], on a clear background, as PNG bytes.
Future<Uint8List> renderMarkPng(Color color, int size) async {
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  CanoaMarkPainter(color).paint(canvas, Size.square(size.toDouble()));
  final picture = recorder.endRecording();
  try {
    final image = await picture.toImage(size, size);
    try {
      final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
      if (bytes == null) throw StateError('the mark could not be encoded');
      return bytes.buffer.asUint8List();
    } finally {
      image.dispose();
    }
  } finally {
    picture.dispose();
  }
}

/// An `.ico` holding one PNG for each of [entries] (size and bytes): the form Windows takes for a tray icon, since it loads the file
/// as an icon and no PNG.
Uint8List icoOf(List<(int, Uint8List)> entries) {
  final header = ByteData(6 + 16 * entries.length)
    ..setUint16(2, 1, Endian.little)
    ..setUint16(4, entries.length, Endian.little);
  var offset = header.lengthInBytes;
  for (final (index, (size, png)) in entries.indexed) {
    final at = 6 + 16 * index;
    header
      ..setUint8(at, size >= 256 ? 0 : size)
      ..setUint8(at + 1, size >= 256 ? 0 : size)
      ..setUint16(at + 4, 1, Endian.little)
      ..setUint16(at + 6, 32, Endian.little)
      ..setUint32(at + 8, png.length, Endian.little)
      ..setUint32(at + 12, offset, Endian.little);
    offset += png.length;
  }
  final out = BytesBuilder(copy: false)..add(header.buffer.asUint8List());
  for (final (_, png) in entries) {
    out.add(png);
  }
  return out.toBytes();
}

/// The tray icon of the brand's mark in [color]: an `.ico` on Windows, a PNG elsewhere.
Future<Uint8List> renderTrayIcon(Color color, {required bool ico}) async {
  if (!ico) return renderMarkPng(color, trayPngSize);
  return icoOf([for (final size in trayIconSizes) (size, await renderMarkPng(color, size))]);
}

/// Writes the tray icon in [color] into [folder] and gives its path, which the tray takes in place of an asset's. A file is kept
/// under the colour's name, so asking again for a colour reuses it, and the files of other colours are removed.
Future<String> writeTrayIcon(Color color, {required bool ico, Directory? folder}) async {
  final where = folder ?? Directory('${Directory.systemTemp.path}${Platform.pathSeparator}${Brand.id}-tray');
  await where.create(recursive: true);
  final extension = ico ? 'ico' : 'png';
  final name = 'tray-${(color.toARGB32() & 0xFFFFFF).toRadixString(16).padLeft(6, '0')}.$extension';
  final file = File('${where.path}${Platform.pathSeparator}$name');
  if (!file.existsSync()) await file.writeAsBytes(await renderTrayIcon(color, ico: ico), flush: true);
  await for (final other in where.list()) {
    if (other is File && other.path != file.path && other.uri.pathSegments.last.startsWith('tray-')) {
      try {
        await other.delete();
      } on FileSystemException {
        // The tray may still hold the previous icon open; it goes with the next change.
      }
    }
  }
  return file.path;
}
