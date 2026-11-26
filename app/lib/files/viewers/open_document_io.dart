import 'dart:io';

import 'package:url_launcher/url_launcher.dart';

/// Opens [document], one HTML page a viewer drew, in the person's browser: it is written to a folder of its own in the temporary
/// folder and opened as a file, which a browser gives an origin of its own, so the page reaches nothing the app keeps. True when a
/// browser took it.
Future<bool> openDocument(String document, String name) async {
  final folder = await Directory.systemTemp.createTemp('canoa-viewer-');
  final safe = name.replaceAll(RegExp(r'[^A-Za-z0-9._-]'), '_');
  final file = File('${folder.path}${Platform.pathSeparator}${safe.isEmpty ? 'page' : safe}.html');
  await file.writeAsString(document);
  return launchUrl(file.uri, mode: LaunchMode.externalApplication);
}
