import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';

/// An [http.Client] whose requests are sent by the Canoa host this app runs (`network/fetch`), not by the app's own library.
///
/// The host follows the system: the proxy Windows, macOS or the desktop names (PAC and WPAD included) and its login, the operating
/// system's certificate verifier, and the client certificate its keystores hold for a server that asks for one, with the person's
/// `[host.network]` over them. `dart:io` does none of that, so the requests the desktop app makes for itself go there.
///
/// A request is held whole in memory both ways (the host passes on at most 16 MiB), which suits the small JSON the app reads. When
/// the host cannot be reached the request fails and says so; it is not sent another way, since that would leave the proxy and the
/// login behind without the person knowing.
final class HostHttpClient extends http.BaseClient {
  HostHttpClient(this._control, {this.timeout = const Duration(seconds: 30)});

  final HostControl _control;

  /// How long the host has to complete a request.
  final Duration timeout;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    final body = await request.finalize().toBytes();
    final Object? result;
    try {
      final client = await _control.ready();
      result = await client.call(
        'network/fetch',
        {
          'url': request.url.toString(),
          'method': request.method,
          'headers': [for (final entry in request.headers.entries) [entry.key, entry.value]],
          if (body.isNotEmpty) 'body': base64Encode(body),
          'seconds': timeout.inSeconds < 1 ? 1 : timeout.inSeconds,
        },
        null,
        timeout + const Duration(seconds: 10),
      );
    } on ControlFailed catch (error) {
      throw http.ClientException(error.message, request.url);
    } on TimeoutException {
      throw http.ClientException('The host did not answer in time.', request.url);
    } on Object catch (error) {
      throw http.ClientException('The host that sends the app\'s requests cannot be reached: $error', request.url);
    }
    if (result is! Map<Object?, Object?> || result['status'] is! int || result['body'] is! String) {
      throw http.ClientException('The host answered a request with something that is not a reply.', request.url);
    }
    final Uint8List bytes;
    try {
      bytes = base64Decode(result['body']! as String);
    } on FormatException {
      throw http.ClientException('The host answered a request with a body that is not base64.', request.url);
    }
    return http.StreamedResponse(
      Stream.value(bytes),
      result['status']! as int,
      contentLength: bytes.length,
      request: request,
      headers: _headers(result['headers']),
    );
  }

  /// The headers as `package:http` keeps them: lower case names, a repeated one joined with a comma.
  static Map<String, String> _headers(Object? pairs) {
    final headers = <String, String>{};
    if (pairs is! List<Object?>) return headers;
    for (final pair in pairs) {
      if (pair is! List<Object?> || pair.length != 2 || pair[0] is! String || pair[1] is! String) continue;
      final name = (pair[0]! as String).toLowerCase();
      final value = pair[1]! as String;
      headers[name] = headers.containsKey(name) ? '${headers[name]}, $value' : value;
    }
    return headers;
  }
}
