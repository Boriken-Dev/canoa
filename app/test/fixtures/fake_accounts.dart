// A host's accounts the tests control: its catalog, the accounts it lists, the sign-ins it was asked for (each driven by the test),
// and the sealed form a sign-in asks, bound by an in-process link's secret as the real one is.
import 'dart:typed_data';

import 'package:canoa/accounts/account_models.dart';
import 'package:canoa/accounts/accounts_host.dart';
import 'package:canoa/acp/link_secret.dart';
import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show SecretCardEnvironment;

/// The 32 bytes the host and the app share on the link, fixed so both ends derive the same keys.
final Uint8List linkSecret = Uint8List.fromList(List<int>.generate(32, (i) => i + 9));

const claudeCatalog = {
  'harness': 'claude',
  'kinds': [
    {
      'id': 'claude-login',
      'provides': ['claude-code'],
      'title': 'Claude subscription',
      'rotates': true,
      'state_dir': true,
      'notices': ['A login on disk the agent\'s commands can read'],
    },
    {
      'id': 'anthropic-api-key',
      'provides': ['anthropic-api'],
      'title': 'Anthropic API key',
      'settings': {
        'type': 'object',
        'properties': {'base_url': {'type': 'string', 'title': 'Base URL'}},
      },
      'endpoints': [
        {'key': 'base_url', 'purpose': 'model_api', 'default': 'https://api.anthropic.com'},
      ],
    },
  ],
  'logins': [
    {'id': 'subscription', 'title': 'Subscription', 'description': 'Sign in with your Claude plan', 'produces': 'claude-login', 'steps': ['open_url', 'paste_code']},
    {'id': 'api-key', 'title': 'API key', 'description': 'Paste a key from the console', 'produces': 'anthropic-api-key', 'steps': ['ask']},
    {
      'id': 'token-first',
      'title': 'Token first',
      'produces': 'claude-login',
      'inputs': {
        'type': 'object',
        'properties': {'token': {'type': 'string', 'title': 'Token', 'writeOnly': true, 'x-sensitive': true}},
      },
    },
    {
      'id': 'sso',
      'title': 'Single sign-on',
      'produces': 'claude-login',
      'inputs': {
        'type': 'object',
        'properties': {'email': {'type': 'string', 'title': 'Work email'}},
        'required': ['email'],
      },
    },
  ],
};

const keyForm = '{"type":"object","properties":{"api_key":{"type":"string","title":"API key","writeOnly":true,"x-sensitive":true}},"required":["api_key"]}';

String _secretForm(String field) =>
    '{"type":"object","properties":{"$field":{"type":"string","title":"$field","writeOnly":true,"x-sensitive":true}},"required":["$field"]}';

/// The app's half of the link binding, and the host's prover for it.
Future<({LinkSecretIdentity app, AcpSecretProver host, AcpSecretVerifier hostVerifier})> linkEnds() async {
  final app = await linkSecretIdentity(linkSecret);
  final key = await AcpLinkKey.derive(linkSecret);
  final host = AcpSecretProver.linked(key, AcpLinkRole.host);
  return (app: app, host: host, hostVerifier: AcpSecretVerifier(own: host.id, link: key, linkRole: AcpLinkRole.host));
}

/// The sealed request the host asks a sign-in's form with, and the environment the app checks and answers it with.
Future<(AcpSecretRequestParams, SecretCardEnvironment, AcpSecretReceiver)> keyRequest({
  String id = 'canoa-secret-1',
  String message = 'Anthropic API key',
  String? field,
}) async {
  final ends = await linkEnds();
  final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
  final context = AcpSecretContext(sessionId: 'accounts', requestId: id, message: message, schema: field == null ? keyForm : _secretForm(field));
  final receiver = await AcpSecretReceiver.create(AcpSecretMethod.request, context, ends.host, ends.app.identity.prover.id, now);
  ends.app.identity.verifier.trust(ends.app.host);
  final environment = SecretCardEnvironment(
    prover: ends.app.identity.prover, verifier: ends.app.identity.verifier, expected: ends.app.host, host: 'Canoa',
  );
  return (AcpSecretRequestParams(context: context, recipient: receiver.recipient), environment, receiver);
}

/// Codex as its provider says it: a browser sign-in that takes the address the browser ends on, and a device code.
const codexCatalog = {
  'harness': 'codex',
  'kinds': [
    {'id': 'codex-chatgpt', 'title': 'ChatGPT login', 'rotates': true, 'state_dir': true},
  ],
  'logins': [
    {'id': 'chatgpt', 'title': 'Sign in with ChatGPT', 'produces': 'codex-chatgpt', 'steps': ['open_url', 'paste_code'], 'remote': true},
    {'id': 'chatgpt-device', 'title': 'Sign in with ChatGPT by a code', 'produces': 'codex-chatgpt', 'steps': ['open_url', 'user_code'], 'remote': true},
  ],
};

/// OpenCode as its provider says it: a key for one provider (asked first which variable it is for), and the sign-in its own `auth.json`
/// keeps, which no method drives.
const opencodeCatalog = {
  'harness': 'opencode',
  'kinds': [
    {
      'id': 'opencode-login',
      'provides': ['opencode'],
      'title': 'OpenCode sign-in',
      'rotates': true,
      'state_dir': true,
      'notices': ['the login is kept in a file that what the agent runs can read'],
    },
    {
      'id': 'opencode-provider-key',
      'provides': ['opencode-key'],
      'title': 'OpenCode provider key',
      'state_dir': true,
      'settings': {
        'type': 'object',
        'properties': {
          'variable': {
            'type': 'string',
            'examples': ['OPENAI_API_KEY', 'GROQ_API_KEY'],
          },
        },
        'required': ['variable'],
      },
      'notices': ["the credential is in the environment of every program the agent runs, in the variable the account names, over the person's own value of it"],
    },
  ],
  'logins': [
    {
      'id': 'provider-key',
      'title': "A provider's key, kept by OpenCode",
      'description': 'Runs `opencode auth login` for a provider and types the key it asks for',
      'produces': 'opencode-login',
      'steps': ['ask'],
      'remote': true,
      'inputs': {
        'type': 'object',
        'properties': {
          'provider': {'type': 'string', 'title': 'Provider', 'examples': ['openrouter', 'openai']},
          'method': {'type': 'string', 'title': 'Login method', 'examples': ['Manually enter API Key']},
        },
        'required': ['provider'],
      },
    },
    {
      'id': 'provider-code',
      'title': 'Sign in with a code',
      'description': 'Runs `opencode auth login` for a provider that shows a page and a code to confirm there',
      'produces': 'opencode-login',
      'steps': ['open_url', 'user_code'],
      'remote': true,
      'inputs': {
        'type': 'object',
        'properties': {
          'provider': {'type': 'string', 'title': 'Provider', 'examples': ['github-copilot', 'openai']},
          'method': {'type': 'string', 'title': 'Login method', 'examples': ['ChatGPT Pro/Plus (headless)']},
        },
        'required': ['provider'],
      },
    },
    {
      'id': 'api-key',
      'title': 'Provider API key',
      'description': "Asks which environment variable the key goes in (a provider's own is suggested, any other name works) and then the key",
      'produces': 'opencode-provider-key',
      'steps': ['ask'],
      'remote': true,
      'inputs': {
        'type': 'object',
        'properties': {
          'variable': {'type': 'string', 'title': 'Variable', 'examples': ['OPENAI_API_KEY', 'GROQ_API_KEY']},
        },
        'required': ['variable'],
      },
    },
  ],
};

final class FakeAccountsHost implements AccountsHost {
  FakeAccountsHost({List<AccountInfo>? accounts}) : accounts = accounts ?? [];

  final List<AccountInfo> accounts;
  final catalogs = <String, Object?>{'claude': claudeCatalog, 'codex': codexCatalog, 'opencode': opencodeCatalog};
  final signIns = <SignIn>[];
  final inputsGiven = <Map<String, String>>[];
  final removed = <String>[];

  /// The agent entries the host lists, and what it was asked to make each offer, in order.
  final List<AccountEntry> hostEntries = [];
  final offers = <(String, List<String>)>[];
  final offerHarness = <String, String?>{};
  int cancelled = 0;
  String? removeRefusal;

  @override
  Future<AccountCatalog> catalog(String harness) async => AccountCatalog.parse(catalogs[harness] ?? {'harness': harness});

  @override
  Future<List<AccountInfo>> list() async => List.of(accounts);

  @override
  Future<List<AccountEntry>> entries() async => List.of(hostEntries);

  @override
  Future<void> offer(String entry, List<String> accounts, {String? harness}) async {
    offers.add((entry, List.of(accounts)));
    offerHarness[entry] = harness;
    final at = hostEntries.indexWhere((each) => each.name == entry);
    if (at >= 0) {
      final one = hostEntries[at];
      hostEntries[at] = AccountEntry(name: one.name, harness: one.harness, accounts: List.of(accounts), open: one.open, accepts: one.accepts, usable: one.usable);
    }
  }

  /// The moves the app asked for, and what the host answers: nothing moved (accounts that share a folder), a new id for the
  /// conversation, or a refusal.
  final moves = <({String entry, String session, String cwd, String? from, String to})>[];
  AccountMove? moved;
  String? moveRefusal;

  @override
  Future<AccountMove> moveSession({required String entry, required String session, required String cwd, String? from, required String to}) async {
    moves.add((entry: entry, session: session, cwd: cwd, from: from, to: to));
    if (moveRefusal != null) throw StateError(moveRefusal!);
    return moved ?? AccountMove(moved: false, session: session);
  }

  @override
  Future<void> remove(String name) async {
    if (removeRefusal != null) throw StateError(removeRefusal!);
    removed.add(name);
    accounts.removeWhere((each) => each.name == name);
  }

  @override
  SignIn signIn({required String name, required String harness, String? method, Map<String, String> inputs = const {}, bool again = false}) {
    inputsGiven.add(inputs);
    final run = SignIn(name: name, harness: harness, method: method, onCancel: () => cancelled++);
    signIns.add(run);
    return run;
  }
}

AccountInfo account(String name, {String kind = 'anthropic-api-key', String state = 'signed in', String harness = 'claude', String id = '', List<String> provides = const []}) =>
    AccountInfo.parse({'name': name, 'harness': harness, 'kind': kind, 'id': id.isEmpty ? 'id-$name' : id, 'state': state, if (provides.isNotEmpty) 'provides': provides})!;
