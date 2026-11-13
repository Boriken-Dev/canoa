import 'dart:convert';

import 'package:coqui_acp/coqui_acp.dart';
import 'package:coqui_acp/page.dart';
import 'package:coqui_web/coqui_web.dart';

import '../chat/agent_events.dart';
import 'delegate.dart';
import 'link_secret.dart';
import 'secret_wire.dart';
import 'usage_wire.dart';

export 'package:coqui_acp/coqui_acp.dart';
export 'delegate.dart' show DelegateNotice;
export 'link_secret.dart' show LinkSecretIdentity, linkSecretIdentity;
export 'secret_wire.dart' show AgentSecrets;

/// A proven peer, and the channel that keeps everything after the proof with that peer.
final class VerifiedPeer {
  const VerifiedPeer(this.peerId, this.channel);
  final String peerId;
  final WebChannel channel;
}

/// Must cryptographically verify the remote peer, not trust a supplied ID, and answer with a channel whose messages
/// are tied to that proof: a plain channel after a proof belongs to whoever carries it.
typedef VerifyAcpPeer = Future<VerifiedPeer> Function(WebChannel channel, String expectedPeerId);

/// What an agent's `initialize` answer says about the agent itself, read from the answer as it passes, since the shared client
/// keeps only the version and capabilities. All of it is the agent's to say.
final class AgentIdentity {
  /// `agentInfo` in v1, `info` in v2; null when the agent sent none.
  String? name, title, version;

  /// Grok's ACP sends no `agentInfo`, only `_meta.grokShell` and `_meta.agentVersion`.
  bool grokShell = false;

  /// The usage extension, when the agent advertised it.
  UsageSupport? usage;

  /// The method that opens an earlier session again, as the client picks it: `session/resume` where both are offered, or null
  /// when the agent offers none.
  String? reopenBy;

  bool _seen = false;

  /// Whether the answer to `initialize` has passed.
  bool get seen => _seen;

  /// Bounded, since an agent can send any amount.
  static String? _text(Object? value) =>
      value is String && value.trim().isNotEmpty ? (value.length > 200 ? value.substring(0, 200) : value.trim()) : null;

  String _inspect(String message) {
    // Only the first answer naming a protocol version is read; every other message passes untouched.
    if (_seen || !message.contains('protocolVersion')) return message;
    final Object? decoded;
    try {
      decoded = jsonDecode(message);
    } on FormatException {
      return message;
    }
    if (decoded is! Map || decoded['result'] is! Map) return message;
    final result = decoded['result'] as Map;
    final version = result['protocolVersion'];
    if (version != 1 && version != 2) return message;
    _seen = true;
    final info = result[version == 2 ? 'info' : 'agentInfo'];
    if (info is Map) {
      name = _text(info['name']);
      title = _text(info['title']);
      this.version = _text(info['version']);
    }
    final meta = result['_meta'];
    if (meta is Map) {
      grokShell = meta['grokShell'] == true;
      this.version ??= _text(meta['agentVersion']);
    }
    final capabilities = result[version == 2 ? 'capabilities' : 'agentCapabilities'];
    bool says(List<String> path) {
      Object? at = capabilities;
      for (final step in path) {
        if (at is! Map) return false;
        at = at[step];
      }
      return at == true || at is Map;
    }
    usage = UsageSupport.parse(capabilities is Map ? capabilities['_meta'] : null);
    reopenBy = version == 2 || says(const ['sessionCapabilities', 'resume'])
        ? 'session/resume'
        : says(const ['loadSession'])
        ? 'session/load'
        : null;
    return message;
  }
}

/// Answers a permission request [from] the client it reached, which names the conversation it belongs to along with its session
/// id: an agent chooses its own session ids, and two agents can choose the same one.
typedef AgentDecider = Future<AcpDecision> Function(AcpPermissionRequest request, VerifiedAgent from);

/// Puts an agent's question [from] the client it reached to the person, as [AgentDecider] does a permission request.
typedef AgentElicitor = Future<AcpElicitationAnswer> Function(AcpElicitation request, VerifiedAgent from);

/// An ACP client that cannot exist until its peer is proven: Canoa's own rule, on the shared client.
final class VerifiedAgent {
  VerifiedAgent._(AcpClient Function(AcpDecider? decider, AcpElicitor? elicitor) client, this.peerId, this.identity, this._usage,
      AgentDecider? decider, {AgentElicitor? elicitor, this.isChild = false, DelegateWire? delegate, this.linkSecrets})
      : _delegate = delegate {
    _client = client(
      decider == null ? null : (request) => decider(request, this),
      elicitor == null ? null : (request) => elicitor(request, this),
    );
  }

  late final AcpClient _client;
  final String peerId;

  /// Whether this is a program this app started, named in [peerId] as `local:<label>`.
  final bool isChild;

  /// What binds this app's secret answers on the in-process link, and the host's identity on it: set only for a link to the host in
  /// this process, which the host binds by a secret it minted. Null for any other connection.
  final LinkSecretIdentity? linkSecrets;

  /// What the agent said about itself when it was initialized.
  final AgentIdentity identity;

  final UsageWire _usage;
  final DelegateWire? _delegate;

  /// The host's events about the tasks a session handed on, as their params; empty when this connection takes none.
  Stream<DelegateNotice> get taskEvents => _delegate?.events ?? Stream<DelegateNotice>.empty();

  /// Whether this connection told the host it can show tasks, answer for them and stop them.
  bool get takesTasks => _delegate?.optIn ?? false;

  /// Asks the host to stop [sessionId]'s tasks, or only [taskId]. Whether the host said it did.
  Future<bool> stopTasks(String sessionId, {String? taskId}) async => await _delegate?.stop(sessionId, taskId: taskId) ?? false;

  /// The updates the host kept of [taskId]; null when it has none to give.
  Future<List<Map<String, Object?>>?> taskHistory(String sessionId, String taskId) async => _delegate?.history(sessionId, taskId);

  /// Whether the agent advertised the usage extension.
  bool get reportsUsage => identity.usage != null;

  /// The reports the agent sends when one changes.
  Stream<UsageNotice> get usageNotices => _usage.notices;

  /// Asks the agent for its report, with [sessionId]'s spend; null when it has none to give.
  Future<UsageReport?> fetchUsage({String? sessionId}) => reportsUsage ? _usage.fetch(sessionId: sessionId) : Future<UsageReport?>.value();

  static Future<VerifiedAgent> open(
    WebChannel channel, {
    required String expectedPeerId,
    required VerifyAcpPeer verifyPeer,
    Duration verificationTimeout = const Duration(seconds: 10),
    AgentDecider? decider,
    AgentElicitor? elicitor,
    Map<String, Object?> initializeMeta = const {},
    ProfileApprover? approver,
    AgentSecrets? secrets,
  }) async {
    if (expectedPeerId.isEmpty) throw ArgumentError.value(expectedPeerId, 'expectedPeerId');
    try {
      final actual = await verifyPeer(channel, expectedPeerId).timeout(verificationTimeout);
      if (actual.peerId != expectedPeerId) {
        await actual.channel.close();
        throw const AcpFailure('Verified peer does not match expected identity');
      }
      final proven = actual.channel;
      final identity = AgentIdentity();
      final usage = UsageWire(proven.send, initializeMeta: initializeMeta, approver: approver);
      // A person who can answer permission requests can answer a task's, which come the same way.
      final delegate = DelegateWire(proven.send, optIn: decider != null);
      // Only a host whose peer id was proven can ask for a secret: the answer is bound to that identity, and a program this app
      // started, or a provider, has none.
      final secretWire = secrets == null ? null : SecretWire(proven.send, secrets);
      final client = VerifiedAgent._(
        (decider, elicitor) => AcpClient(
          incoming: proven.messages
              .where((message) => !usage.answersApproval(message) && !(secretWire?.answers(message) ?? false) && !delegate.takes(message))
              .map(identity._inspect)
              .map(usage.incoming),
          send: (message) => proven.send(_withOffer(secretWire, delegate.outgoing(usage.outgoing(message)))),
          closeSource: proven.close,
          decider: decider,
          elicitor: elicitor,
          clientName: 'canoa',
          clientVersion: '0.0.1',
        ),
        actual.peerId,
        identity,
        usage,
        decider,
        elicitor: elicitor,
        delegate: delegate,
      );
      secretWire?.agent = client;
      return client;
    } catch (_) {
      await channel.close();
      rethrow;
    }
  }

  static String _withOffer(SecretWire? wire, String message) => wire == null ? message : wire.outgoing(message);

  /// A child of this app, speaking ACP on its standard streams. It is the person's own program, so nothing needs proving.
  factory VerifiedAgent.fromChild({
    required String label,
    required Stream<String> messages,
    required Future<void> Function(String) send,
    required Future<void> Function() close,
    AgentDecider? decider,
    AgentElicitor? elicitor,
    Map<String, Object?> initializeMeta = const {},
    ProfileApprover? approver,
    AgentSecrets? secrets,
    LinkSecretIdentity? linkSecrets,
  }) {
    final identity = AgentIdentity();
    final usage = UsageWire(send, initializeMeta: initializeMeta, approver: approver);
    final delegate = DelegateWire(send, optIn: decider != null);
    // Only the link to the host in this process, which has the secret it shares with it, can be asked for one.
    final secretWire = secrets == null || linkSecrets == null ? null : SecretWire(send, secrets, offer: linkSecrets.identity.offer);
    final client = VerifiedAgent._(
      (decider, elicitor) => AcpClient(
        incoming: messages
            .where((message) => !usage.answersApproval(message) && !(secretWire?.answers(message) ?? false) && !delegate.takes(message))
            .map(identity._inspect)
            .map(usage.incoming),
        send: (message) => send(_withOffer(secretWire, delegate.outgoing(usage.outgoing(message)))),
        closeSource: close,
        decider: decider,
        elicitor: elicitor,
        clientName: 'canoa',
        clientVersion: '0.0.1',
        // A command that downloads itself the first time (an `npx` adapter, say) answers nothing until it has.
        requestTimeout: const Duration(minutes: 3),
      ),
      'local:$label',
      identity,
      usage,
      decider,
      elicitor: elicitor,
      isChild: true,
      delegate: delegate,
      linkSecrets: secretWire == null ? null : linkSecrets,
    );
    secretWire?.agent = client;
    return client;
  }

  /// Over a connection a browser's provider holds. The proof is the provider's, and without one there is no client.
  static Future<VerifiedAgent> fromProvider(AcpPageConnection connection, {AgentDecider? decider, AgentElicitor? elicitor}) async {
    final agent = connection.agent;
    final peer = agent.peer;
    if (!agent.verified || peer == null) {
      await connection.close();
      throw const AcpFailure('The browser has not proven who this agent is');
    }
    final identity = AgentIdentity();
    final usage = UsageWire(connection.send);
    return VerifiedAgent._(
      (decider, elicitor) => AcpClient(
        incoming: connection.messages.map(identity._inspect).map(usage.incoming),
        send: (message) => connection.send(usage.outgoing(message)),
        closeSource: connection.close,
        decider: decider,
        elicitor: elicitor,
        clientName: 'canoa',
        clientVersion: '0.0.1',
      ),
      peer,
      identity,
      usage,
      decider,
      elicitor: elicitor,
    );
  }

  Stream<AcpSessionUpdate> get updates => _client.updates;
  Stream<AcpEvent> get events => _client.events;
  bool get isClosed => _client.isClosed;
  int? get protocolVersion => _client.protocolVersion;
  Future<void> initialize() => _client.initialize();
  Future<AcpDescription> describe() => _client.describe();
  Future<String> createSession({required String cwd, AcpSessionSetup setup = AcpSessionSetup.none}) =>
      _client.createSession(cwd: cwd, setup: setup);
  Future<AcpOpenedSession> openSession({required String cwd, AcpSessionSetup setup = AcpSessionSetup.none}) =>
      _client.openSession(cwd: cwd, setup: setup);
  bool takesMcp(AcpMcpTransport transport) => _client.takesMcp(transport);
  bool get takesSystemPrompt => _client.takesSystemPrompt;
  bool get takesAdditionalDirectories => _client.takesAdditionalDirectories;
  Future<Map<String, dynamic>> setConfigOption(String sessionId, String configId, String value) =>
      _client.setConfigOption(sessionId, configId, value);
  Future<void> setMode(String sessionId, String modeId) => _client.setMode(sessionId, modeId);
  Future<AcpOpenedSession> resumeSession(String id, {required String cwd, bool replayHistory = false}) =>
      _client.resumeSession(id, cwd: cwd, replayHistory: replayHistory);
  bool get takesImages => _client.takesImages;
  bool get takesFiles => _client.takesFiles;
  bool get canReopenSessions => _client.canReopenSessions;
  Future<AcpSessionPage> listSessions({String? cwd, String? cursor}) => _client.listSessions(cwd: cwd, cursor: cursor);
  Future<AcpOpenedSession> reopenSession(String id, {required String cwd, AcpSessionSetup setup = AcpSessionSetup.none}) =>
      _client.reopenSession(id, cwd: cwd, setup: setup);
  Future<String?> prompt(String sessionId, String text, {List<AcpAttachment> attachments = const []}) =>
      _client.prompt(sessionId, text, attachments: attachments);
  Future<void> cancel(String sessionId) => _client.cancel(sessionId);
  Future<void> closeSession(String sessionId) => _client.closeSession(sessionId);
  final _afterClose = <Future<void> Function()>[];

  /// Runs [action] once this client is closed, as for files written for its sessions.
  void afterClose(Future<void> Function() action) => _afterClose.add(action);

  Future<void> close() async {
    try {
      await _client.close();
    } finally {
      _usage.close();
      _delegate?.close();
      final actions = [..._afterClose];
      _afterClose.clear();
      for (final action in actions) {
        await action();
      }
    }
  }
}
