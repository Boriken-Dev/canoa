// The app's WebSocket to a remote Canoa host follows the system's proxy and its login: the connection is carried by the host the
// app runs (`network/tunnel`), which the stand-ins below reach as a computer on a network with a proxy would. The host is the real
// one (`store_control`, an example of canoa-host): build it first, `cargo build -p canoa-host --bin canoa --examples`. The
// stand-ins are a proxy that wants a login and a remote host with a certificate made for the test (OpenSSL's command line makes
// it, nothing is committed); a test that needs them is skipped without them.
@TestOn('vm')
@Timeout(Duration(minutes: 2))
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/desktop/host_tool.dart';
import 'package:canoa/net/system_dial.dart';
import 'package:coqui_transport_web/coqui_transport_web.dart';
import 'package:flutter_test/flutter_test.dart';

import 'fixtures/control_process.dart';
import 'fixtures/examples.dart';

const _user = 'alice';
const _password = 's3cret';
const _remote = 'remote.example.test';

/// A link to a host that answers nothing and keeps what it was asked.
final class _SilentHost implements HostLink {
  final asked = <String>[];

  @override
  Stream<String> get messages => const Stream<String>.empty();

  @override
  Future<void> send(String message) async => asked.add(message);

  @override
  Future<void> close() async {}
}

/// What a proxy was asked: the request line and the login that came with it.
typedef _Asked = ({String line, String? login});

/// A proxy that carries a tunnel only for the right login, to one origin, and keeps what it was asked.
final class _Proxy {
  _Proxy._(this._server, this._origin);

  final ServerSocket _server;
  final int _origin;
  final asked = <_Asked>[];

  static Future<_Proxy> start(int origin) async {
    final proxy = _Proxy._(await ServerSocket.bind(InternetAddress.loopbackIPv4, 0), origin);
    proxy._server.listen(proxy._serve);
    return proxy;
  }

  int get port => _server.port;

  static String get login => 'Basic ${base64Encode(utf8.encode('$_user:$_password'))}';

  /// The tunnels it let through, by the login's owner.
  List<String> get allowed => [
    for (final entry in asked)
      if (entry.login == login) entry.line,
  ];

  Future<void> _serve(Socket client) async {
    final head = StringBuffer();
    final queue = StreamIterator<List<int>>(client);
    var tail = <int>[];
    try {
      while (true) {
        head.clear();
        while (!head.toString().endsWith('\r\n\r\n')) {
          if (tail.isEmpty) {
            if (!await queue.moveNext()) return;
            tail = List<int>.of(queue.current);
          }
          head.writeCharCode(tail.removeAt(0));
        }
        final lines = head.toString().split('\r\n');
        final login = [
          for (final line in lines)
            if (line.toLowerCase().startsWith('proxy-authorization:')) line.substring(line.indexOf(':') + 1).trim(),
        ].firstOrNull;
        asked.add((line: lines.first, login: login));
        if (login != _Proxy.login) {
          client.write(
            'HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm="stand-in"\r\nContent-Length: 0\r\n\r\n',
          );
          await client.flush();
          continue;
        }
        final origin = await Socket.connect(InternetAddress.loopbackIPv4, _origin);
        client.write('HTTP/1.1 200 Connection Established\r\n\r\n');
        await client.flush();
        if (tail.isNotEmpty) origin.add(tail);
        unawaited(origin.addStream(_rest(queue)).whenComplete(origin.destroy));
        unawaited(client.addStream(origin).whenComplete(client.destroy));
        return;
      }
    } on Object {
      client.destroy();
    }
  }

  static Stream<List<int>> _rest(StreamIterator<List<int>> queue) async* {
    while (await queue.moveNext()) {
      yield queue.current;
    }
  }

  Future<void> close() => _server.close();
}

/// A certificate for [_remote], made by OpenSSL's command line, and the server context that uses it.
Future<({SecurityContext server, String certificate, Directory dir})?> _certificate() async {
  final openssl = ['openssl', 'C:/Program Files/Git/usr/bin/openssl.exe'].where((candidate) {
    try {
      return Process.runSync(candidate, ['version']).exitCode == 0;
    } on ProcessException {
      return false;
    }
  }).firstOrNull;
  if (openssl == null) return null;
  final dir = await Directory.systemTemp.createTemp('canoa-dial');
  final key = '${dir.path}/key.pem';
  final certificate = '${dir.path}/cert.pem';
  final made = await Process.run(
    openssl,
    [
      'req',
      '-x509',
      '-newkey',
      'rsa:2048',
      '-nodes',
      '-keyout',
      key,
      '-out',
      certificate,
      '-days',
      '1',
      '-subj',
      '/CN=$_remote',
      '-addext',
      'subjectAltName=DNS:$_remote',
    ],
    environment: {'MSYS2_ARG_CONV_EXCL': '*'},
  );
  if (made.exitCode != 0) return null;
  final server = SecurityContext()
    ..useCertificateChain(certificate)
    ..usePrivateKey(key);
  return (server: server, certificate: certificate, dir: dir);
}

/// A remote host that sends `hello` to whoever opens a WebSocket to it.
Future<HttpServer> _origin({SecurityContext? tls}) async {
  final server = tls == null
      ? await HttpServer.bind(InternetAddress.loopbackIPv4, 0)
      : await HttpServer.bindSecure(InternetAddress.loopbackIPv4, 0, tls);
  server.listen((request) async {
    final socket = await WebSocketTransformer.upgrade(request);
    socket.add('hello');
    socket.listen((_) {});
  });
  return server;
}

void main() {
  final skip = exampleMissing('store_control');

  group('through the real host', skip: skip, () {
    late Directory dir;
    late ControlProcess example;
    late HostControl control;
    late _Proxy proxy;
    late HttpServer origin;
    late Directory certDir;
    var tls = false;

    /// The host with the system's proxy named by its environment: [login] is what it logs in with, and [bypass] the names that go
    /// direct. A person's own proxy variables are replaced, so the run depends on nothing of this computer's.
    Future<void> startHost({String password = _password, String bypass = 'unrelated.invalid'}) async {
      final url = 'http://$_user:$password@127.0.0.1:${proxy.port}';
      final process = await Process.start(
        exampleBinary('store_control')!,
        [dir.path],
        environment: {
          for (final name in ['HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'all_proxy']) name: url,
          'NO_PROXY': bypass,
          'no_proxy': bypass,
        },
      );
      example = ControlProcess(process);
      control = HostControl.dialing(() async => example.open());
      useHostDials(control);
    }

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('canoa-dial-host');
      final made = await _certificate();
      tls = made != null;
      if (made != null) {
        certDir = made.dir;
        SecurityContext.defaultContext.setTrustedCertificates(made.certificate);
      }
      origin = await _origin(tls: made?.server);
      proxy = await _Proxy.start(origin.port);
    });

    tearDown(() async {
      resetSystemDial();
      await proxy.close();
      await origin.close(force: true);
      try {
        await example.process.stdin.close();
        await example.process.exitCode.timeout(
          const Duration(seconds: 20),
          onTimeout: () {
            example.process.kill();
            return -1;
          },
        );
      } on Object {
        // Never started.
      }
      for (final each in [dir, if (tls) certDir]) {
        try {
          await each.delete(recursive: true);
        } on FileSystemException {
          // Windows may hold a file a moment longer.
        }
      }
    });

    test('a wss connection to a remote host goes through the system proxy, logged in, and the app talks TLS to the host itself', () async {
      if (!tls) markTestSkipped('no OpenSSL command line to make the stand-in host\'s certificate');
      if (!tls) return;
      await startHost();
      final channel = await connectSystemChannel(
        WebEndpoints(webSocket: Uri.parse('wss://$_remote:${origin.port}/channel')),
        timeout: const Duration(seconds: 30),
      );
      expect(await channel.messages.first.timeout(const Duration(seconds: 10)), 'hello');
      await channel.close();
      expect(proxy.allowed, [
        'CONNECT $_remote:${origin.port} HTTP/1.1',
      ], reason: 'the proxy saw the tunnel with the login: ${proxy.asked}');
    });

    test('a proxy that refuses the login leaves the dial failed, saying why, and nothing reaches the host', () async {
      if (!tls) markTestSkipped('no OpenSSL command line to make the stand-in host\'s certificate');
      if (!tls) return;
      await startHost(password: 'wrong');
      await expectLater(
        connectSystemChannel(
          WebEndpoints(webSocket: Uri.parse('wss://$_remote:${origin.port}/channel')),
          timeout: const Duration(seconds: 30),
        ),
        throwsA(isA<TransportUnavailable>().having((e) => e.toString(), 'message', contains('proxy'))),
      );
      expect(proxy.allowed, isEmpty);
    });

    test('a name the system sends direct is opened by the app itself, and the proxy is not asked', () async {
      if (!tls) markTestSkipped('no OpenSSL command line to make the stand-in host\'s certificate');
      if (!tls) return;
      await startHost(bypass: _remote);
      // The name resolves to nothing here: what shows is that the app tried the address itself.
      await expectLater(
        connectSystemChannel(
          WebEndpoints(webSocket: Uri.parse('wss://$_remote:${origin.port}/channel')),
          timeout: const Duration(seconds: 30),
        ),
        throwsA(isA<TransportUnavailable>()),
      );
      expect(proxy.asked, isEmpty);
    });
  });

  test('this computer\'s own address is dialled directly, without asking the host', () async {
    final origin = await _origin();
    addTearDown(() => origin.close(force: true));
    final host = _SilentHost();
    useHostDials(HostControl.dialing(() async => host));
    addTearDown(resetSystemDial);
    final channel = await connectSystemChannel(
      WebEndpoints(webSocket: Uri.parse('ws://127.0.0.1:${origin.port}/'), allowLocalPlainWebSocket: true),
    );
    expect(await channel.messages.first.timeout(const Duration(seconds: 10)), 'hello');
    await channel.close();
    expect(host.asked, isEmpty);
  });

  test('with no host to carry it, the dial is the platform\'s, as before', () async {
    final origin = await _origin();
    addTearDown(() => origin.close(force: true));
    resetSystemDial();
    final channel = await connectSystemChannel(
      WebEndpoints(webSocket: Uri.parse('ws://127.0.0.1:${origin.port}/'), allowLocalPlainWebSocket: true),
    );
    expect(await channel.messages.first.timeout(const Duration(seconds: 10)), 'hello');
    await channel.close();
  });
}
