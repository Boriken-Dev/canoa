import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import 'mcp_registry.dart';

/// Reads [uri] as a registry answer: no redirect is followed, nothing past [maxRegistryBytes] is read, and the request ends after
/// [timeout]. The registry sees the address and the search text, and nothing else of this computer.
Future<String> fetchRegistry(Uri uri, {Duration timeout = const Duration(seconds: 20)}) async {
  final client = http.Client();
  try {
    final request = http.Request('GET', uri)
      ..followRedirects = false
      ..headers['accept'] = 'application/json';
    final response = await client.send(request).timeout(timeout);
    if (response.statusCode != 200) {
      throw RegistryRefusal('The registry answered ${response.statusCode}${response.statusCode ~/ 100 == 3 ? ' (a redirect, which Canoa does not follow)' : ''}');
    }
    final bytes = <int>[];
    await for (final chunk in response.stream.timeout(timeout)) {
      bytes.addAll(chunk);
      if (bytes.length > maxRegistryBytes) throw const RegistryRefusal('The registry\'s answer is larger than Canoa reads');
    }
    return utf8.decode(bytes, allowMalformed: true);
  } on TimeoutException {
    throw const RegistryRefusal('The registry did not answer in time');
  } on http.ClientException catch (error) {
    throw RegistryRefusal('The registry cannot be reached: ${error.message}');
  } finally {
    client.close();
  }
}
