import 'dart:async';
import 'dart:convert';

import 'package:coqui_acp/secret.dart';

import 'client.dart' show VerifiedAgent;

/// What the host's secret prompts reach: a request for values, a show of values and the show's delivery, answered from the person's
/// own screen. The answers go back to the host on the connection that carried the question, never into the agent's conversation.
abstract interface class AgentSecrets {
  /// What this app offers the host, or null when it has no identity to bind answers with: the host is then told nothing, and serves
  /// its agents no secret tools.
  AcpSecretsCapability? get secretsCapability;

  Future<AcpSecretRequestReply> askSecret(AcpSecretRequestParams request, VerifiedAgent from);
  Future<AcpSecretShowOpened> openSecretShow(AcpSecretShowOpen open, VerifiedAgent from);

  /// True when the person saw the values and closed them.
  Future<bool> deliverSecretShow(AcpSecretShowDeliver deliver, VerifiedAgent from);
}

/// The secret-prompt extension on a connection to a host that was proven by its peer identity, beside the shared ACP client, which
/// never sees these messages: the capability added to `initialize`, and the host's three requests, answered on the same connection.
/// An answer holds only what the extension's types allow: the plain answers as text and each secret sealed to a key only the host
/// holds, so the agent, which the shared client serves, is handed none of it.
final class SecretWire {
  SecretWire(this._send, this._secrets);

  final Future<void> Function(String) _send;
  final AgentSecrets _secrets;

  /// The client this wire serves, set when it is made; a question that comes before is refused.
  VerifiedAgent? agent;

  /// [message] on its way to the host, with the offer added when it is `initialize` and the app has an identity to offer it with.
  String outgoing(String message) {
    final offer = _secrets.secretsCapability;
    if (offer == null || !message.contains('"initialize"')) return message;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return message;
    }
    if (decoded is! Map || decoded['method'] != 'initialize' || decoded['params'] is! Map) return message;
    final params = decoded['params'] as Map;
    final key = params['capabilities'] is Map ? 'capabilities' : 'clientCapabilities';
    final capabilities = params[key] is Map ? params[key] as Map : (params[key] = <String, Object?>{});
    final meta = capabilities['_meta'] is Map ? capabilities['_meta'] as Map : (capabilities['_meta'] = <String, Object?>{});
    meta[acpSecretsCapability] = offer.toJson();
    return jsonEncode(decoded);
  }

  static const _methods = {acpSecretRequestMethod, acpSecretShowOpenMethod, acpSecretShowDeliverMethod};

  /// Whether [message] is one of the host's secret requests, which this wire answers itself so the shared client never sees it.
  /// Without an offer made to the host, a request is refused all the same, and still not passed on.
  bool answers(String message) {
    if (!message.contains('_coqui_acp/secret_')) return false;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return false;
    }
    if (decoded is! Map || !_methods.contains(decoded['method']) || decoded['id'] == null) return false;
    final id = decoded['id'];
    final method = decoded['method'] as String;
    final params = decoded['params'];
    unawaited(() async {
      Map<String, Object?> body;
      try {
        body = {'result': await _run(method, params)};
      } on AcpSecretException {
        body = {'error': {'code': -32602, 'message': 'invalid params'}};
      } on Object {
        body = {'error': {'code': -32603, 'message': 'not answered'}};
      }
      try {
        await _send(jsonEncode({'jsonrpc': '2.0', 'id': id, ...body}));
      } on Object {
        // The link is gone; the host asks nothing more of it.
      }
    }());
    return true;
  }

  Future<Map<String, Object?>> _run(String method, Object? params) async {
    final offer = _secrets.secretsCapability;
    final from = agent;
    if (offer == null || from == null) throw const AcpSecretException(AcpSecretFailure.unsupported, 'no secrets offered');
    switch (method) {
      case acpSecretRequestMethod:
        if (!offer.request) throw const AcpSecretException(AcpSecretFailure.unsupported);
        final request = AcpSecretRequestParams.fromJson(params);
        return (await _secrets.askSecret(request, from)).toJson();
      case acpSecretShowOpenMethod:
        if (!offer.show) throw const AcpSecretException(AcpSecretFailure.unsupported);
        final open = AcpSecretShowOpen.fromJson(params);
        return (await _secrets.openSecretShow(open, from)).toJson();
      default:
        if (!offer.show) throw const AcpSecretException(AcpSecretFailure.unsupported);
        final deliver = AcpSecretShowDeliver.fromJson(params);
        return acpSecretShowDone(shown: await _secrets.deliverSecretShow(deliver, from));
    }
  }
}
