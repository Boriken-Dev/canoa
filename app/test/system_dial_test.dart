// The app's WebSocket to a remote Canoa host follows the system: the connection is made by the host the app runs
// (`network/tunnel`), through the system's proxy and its login, with the TLS made by the host (its trust, its client certificates),
// which the stand-ins below reach as a computer on a network with a proxy would. The host is the real one (`store_control`, an
// example of canoa-host): build it first, `cargo build -p canoa-host --bin canoa --examples`. The stand-ins are a proxy that wants a
// login and a remote host with certificates made for the test by an authority only the host is told to trust (OpenSSL's command
// line makes them, nothing is committed and nothing is added to this computer's trust); a test that needs them is skipped without.
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

/// What OpenSSL's command line made for the test: a certificate authority, a certificate for [_remote] it signed (with its key), and
/// a client certificate it signed, the chain and key of which are in one file. Nothing is committed and nothing is added to this
/// computer's trust: the host is told to trust the authority, and the Dart VM is not.
typedef _Certificates = ({String ca, String server, String serverKey, String client, Directory dir});

Future<_Certificates?> _certificates() async {
  final openssl = ['openssl', 'C:/Program Files/Git/usr/bin/openssl.exe'].where((candidate) {
    try {
      return Process.runSync(candidate, ['version']).exitCode == 0;
    } on ProcessException {
      return false;
    }
  }).firstOrNull;
  if (openssl == null) return null;
  final dir = await Directory.systemTemp.createTemp('canoa-dial');
  String at(String name) => '${dir.path}/$name'.replaceAll('\\', '/');
  Future<bool> run(List<String> arguments) async {
    final made = await Process.run(openssl, arguments, environment: {'MSYS2_ARG_CONV_EXCL': '*'});
    return made.exitCode == 0;
  }

  await File(at('server.ext')).writeAsString('subjectAltName=DNS:$_remote\nextendedKeyUsage=serverAuth\n');
  await File(at('client.ext')).writeAsString('extendedKeyUsage=clientAuth\n');
  final made =
      await run([
        'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', at('ca.key'), '-out', at('ca.pem'), '-days', '1',
        '-subj', '/CN=Canoa Test CA', '-addext', 'basicConstraints=critical,CA:TRUE', '-addext', 'keyUsage=critical,keyCertSign',
      ]) &&
      await run(['req', '-newkey', 'rsa:2048', '-nodes', '-keyout', at('server.key'), '-out', at('server.csr'), '-subj', '/CN=$_remote']) &&
      await run([
        'x509', '-req', '-in', at('server.csr'), '-CA', at('ca.pem'), '-CAkey', at('ca.key'), '-CAcreateserial', '-out', at('server.pem'),
        '-days', '1', '-extfile', at('server.ext'),
      ]) &&
      await run(['req', '-newkey', 'rsa:2048', '-nodes', '-keyout', at('client.key'), '-out', at('client.csr'), '-subj', '/CN=Alice Example']) &&
      await run([
        'x509', '-req', '-in', at('client.csr'), '-CA', at('ca.pem'), '-CAkey', at('ca.key'), '-CAcreateserial', '-out', at('client-cert.pem'),
        '-days', '1', '-extfile', at('client.ext'),
      ]);
  if (!made) return null;
  // The host reads a chain and its unencrypted key from one file.
  await File(at('client.pem')).writeAsString(
    '${await File(at('client-cert.pem')).readAsString()}${await File(at('client.key')).readAsString()}',
  );
  return (ca: at('ca.pem'), server: at('server.pem'), serverKey: at('server.key'), client: at('client.pem'), dir: dir);
}

/// What a remote host sends to whoever opens a WebSocket to it: `hello`, then whether it was shown a client certificate.
Future<HttpServer> _origin({_Certificates? tls, bool wantsClientCertificate = false}) async {
  final HttpServer server;
  if (tls == null) {
    server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  } else {
    // Only the test's authority is trusted here, for the client certificates it asks for; the Dart VM's own roots are not used.
    final context = SecurityContext()
      ..useCertificateChain(tls.server)
      ..usePrivateKey(tls.serverKey)
      ..setTrustedCertificates(tls.ca);
    server = await HttpServer.bindSecure(
      InternetAddress.loopbackIPv4,
      0,
      context,
      requestClientCertificate: wantsClientCertificate,
    );
  }
  server.listen((request) async {
    final shown = request.certificate?.subject;
    final socket = await WebSocketTransformer.upgrade(request);
    socket.add(wantsClientCertificate ? 'hello ${shown ?? 'nobody'}' : 'hello');
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
    _Proxy? proxy;
    HttpServer? origin;
    _Certificates? certificates;

    /// The remote host, and the proxy that reaches it.
    Future<void> startRemote({bool wantsClientCertificate = false}) async {
      final remote = await _origin(tls: certificates, wantsClientCertificate: wantsClientCertificate);
      origin = remote;
      proxy = await _Proxy.start(remote.port);
    }

    /// The host with the system's proxy named by its environment: [password] is what it logs in with, and [bypass] the names that
    /// go direct. A person's own proxy variables are replaced, so the run depends on nothing of this computer's. [network] is the
    /// text of its `[network]` table: the authority it trusts and the client certificate it presents, and none of that is
    /// the Dart VM's.
    Future<void> startHost({String password = _password, String bypass = 'unrelated.invalid', String network = ''}) async {
      if (network.isNotEmpty) await File('${dir.path}/network.toml').writeAsString(network);
      final url = 'http://$_user:$password@127.0.0.1:${proxy!.port}';
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

    String trusting(_Certificates made, {String? client}) =>
        '[network]\nroute = "pinned"\n[network.routes.pinned]\nprovider = "system"\n'
        'settings = { ca_file = "${made.ca}"${client == null ? '' : ', client_certificate = "$client"'} }\n';

    Future<WebChannel> dial() => connectSystemChannel(
      WebEndpoints(webSocket: Uri.parse('wss://$_remote:${origin!.port}/channel')),
      timeout: const Duration(seconds: 30),
    );

    setUp(() async {
      dir = await Directory.systemTemp.createTemp('canoa-dial-host');
      certificates = await _certificates();
    });

    tearDown(() async {
      resetSystemDial();
      await proxy?.close();
      await origin?.close(force: true);
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
      for (final each in [dir, ?certificates?.dir]) {
        try {
          await each.delete(recursive: true);
        } on FileSystemException {
          // Windows may hold a file a moment longer.
        }
      }
    });

    test('a wss connection to a remote host goes through the system proxy, logged in, to a server only the host\'s trust vouches for', () async {
      final made = certificates;
      if (made == null) markTestSkipped('no OpenSSL command line to make the stand-in host\'s certificates');
      if (made == null) return;
      await startRemote();
      // The authority is in the host's network route and nowhere in the Dart VM's trust: only the host makes the TLS.
      await startHost(network: trusting(made));
      final channel = await dial();
      expect(await channel.messages.first.timeout(const Duration(seconds: 10)), 'hello');
      await channel.close();
      expect(proxy!.allowed, [
        'CONNECT $_remote:${origin!.port} HTTP/1.1',
      ], reason: 'the proxy saw the tunnel with the login: ${proxy!.asked}');
    });

    test('a server whose authority the host does not trust is not connected to, whatever the Dart VM would say', () async {
      final made = certificates;
      if (made == null) markTestSkipped('no OpenSSL command line to make the stand-in host\'s certificates');
      if (made == null) return;
      await startRemote();
      // The operating system's trust holds no authority of this test.
      await startHost();
      await expectLater(
        dial(),
        throwsA(isA<TransportUnavailable>().having((e) => e.toString(), 'message', contains('TLS'))),
      );
    });

    test('a server that asks for a client certificate is shown the one the host\'s settings name', () async {
      final made = certificates;
      if (made == null) markTestSkipped('no OpenSSL command line to make the stand-in host\'s certificates');
      if (made == null) return;
      await startRemote(wantsClientCertificate: true);
      await startHost(network: trusting(made, client: made.client));
      final channel = await dial();
      expect(await channel.messages.first.timeout(const Duration(seconds: 10)), contains('Alice Example'));
      await channel.close();
    });

    test('a server that asks for a client certificate is shown none when the host\'s settings name none', () async {
      final made = certificates;
      if (made == null) markTestSkipped('no OpenSSL command line to make the stand-in host\'s certificates');
      if (made == null) return;
      await startRemote(wantsClientCertificate: true);
      await startHost(network: trusting(made, client: 'none'));
      final channel = await dial();
      expect(await channel.messages.first.timeout(const Duration(seconds: 10)), 'hello nobody');
      await channel.close();
    });

    test('a proxy that refuses the login leaves the dial failed, saying why, and nothing reaches the host', () async {
      final made = certificates;
      if (made == null) markTestSkipped('no OpenSSL command line to make the stand-in host\'s certificates');
      if (made == null) return;
      await startRemote();
      await startHost(password: 'wrong', network: trusting(made));
      await expectLater(
        dial(),
        throwsA(isA<TransportUnavailable>().having((e) => e.toString(), 'message', contains('proxy'))),
      );
      expect(proxy!.allowed, isEmpty);
    });

    test('a name the system sends direct is not asked of the proxy: the host opens it itself', () async {
      final made = certificates;
      if (made == null) markTestSkipped('no OpenSSL command line to make the stand-in host\'s certificates');
      if (made == null) return;
      await startRemote();
      await startHost(bypass: _remote, network: trusting(made));
      // The name resolves to nothing here: what shows is that the host tried the address itself.
      await expectLater(dial(), throwsA(isA<TransportUnavailable>()));
      expect(proxy!.asked, isEmpty);
    });
  });

  test('this computer\'s own address is dialled directly, without asking the host', () async {
    final origin = await _origin();
    addTearDown(() => origin.close(force: true));
    final host = _SilentHost();
    useHostDials(HostControl.dialing(() async => host));
    addTearDown(resetSystemDial);
    final channel = await connectSystemChannel(
      WebEndpoints(webSocket: Uri.parse('ws://127.0.0.1:${origin.port}/'), allowPlainWebSocket: true),
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
      WebEndpoints(webSocket: Uri.parse('ws://127.0.0.1:${origin.port}/'), allowPlainWebSocket: true),
    );
    expect(await channel.messages.first.timeout(const Duration(seconds: 10)), 'hello');
    await channel.close();
  });
}
