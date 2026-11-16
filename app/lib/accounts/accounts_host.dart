import 'dart:async';

import 'package:coqui_acp/secret.dart';
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show SecretCardEnvironment;
import 'package:flutter/foundation.dart';

import '../acp/link_secret.dart';
import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import 'account_models.dart';

/// A page the sign-in asks the person to open, and the code to confirm there when it has one.
final class SignInPage {
  const SignInPage({required this.url, this.userCode, this.expiresMs});

  final String url;
  final String? userCode;
  final int? expiresMs;
}

/// A form the sign-in asks, which the host sealed to a key only it holds: what the person types goes to the host and nowhere else.
final class SignInAsk {
  const SignInAsk(this.request, this.environment);

  final AcpSecretRequestParams request;
  final SecretCardEnvironment environment;

  /// What the form is for, as the host worded it.
  String get title => request.context.message;
}

enum SignInPhase { running, done, failed, cancelled }

/// One sign-in running in the host: what it says, the page it wants opened, the form it asks and how it ended. The host's side
/// calls [say], [showPage], [ask], [complete] and [fail]; the person's side calls [answer] and [cancel].
class SignIn extends ChangeNotifier {
  SignIn({required this.name, required this.harness, this.method, this.onCancel});

  /// The account's name, and the provider it signs in to.
  final String name, harness;

  /// The way it signs in; null for a sign-in again that has one.
  final String? method;

  /// Ends what the host is doing, which closing its link does.
  final VoidCallback? onCancel;

  final List<String> lines = [];
  SignInPage? page;
  SignInAsk? asking;
  SignInPhase phase = SignInPhase.running;

  /// The account, once [phase] is [SignInPhase.done].
  AccountInfo? account;

  /// Why it failed, in the host's words: never a value the person typed.
  String? problem;
  Completer<AcpSecretRequestReply>? _reply;

  bool get running => phase == SignInPhase.running;

  void say(String text) {
    if (!running) return;
    lines.add(text);
    notifyListeners();
  }

  void showPage(SignInPage page) {
    if (!running) return;
    this.page = page;
    notifyListeners();
  }

  /// Puts [request] to the person; completes with their sealed answer, or cancelled when they give up.
  Future<AcpSecretRequestReply> ask(AcpSecretRequestParams request, SecretCardEnvironment environment) {
    if (!running) return Future.value(const AcpSecretCancelled());
    final before = _reply;
    if (before != null && !before.isCompleted) before.complete(const AcpSecretCancelled());
    final reply = _reply = Completer();
    asking = SignInAsk(request, environment);
    notifyListeners();
    return reply.future;
  }

  void complete(AccountInfo made) {
    if (!running) return;
    phase = SignInPhase.done;
    account = made;
    asking = null;
    notifyListeners();
  }

  void fail(String why) {
    if (!running) return;
    phase = SignInPhase.failed;
    problem = why;
    asking = null;
    if (_reply case final waiting? when !waiting.isCompleted) waiting.complete(const AcpSecretCancelled());
    notifyListeners();
  }

  /// The person's answer to [asking]; the host is told once.
  void answer(AcpSecretRequestReply reply) {
    final waiting = _reply;
    if (waiting == null || waiting.isCompleted) return;
    asking = null;
    waiting.complete(reply);
    notifyListeners();
  }

  /// Gives up: what is asked is cancelled and the host is told to stop.
  void cancel() {
    if (!running) return;
    phase = SignInPhase.cancelled;
    asking = null;
    if (_reply case final waiting? when !waiting.isCompleted) waiting.complete(const AcpSecretCancelled());
    onCancel?.call();
    notifyListeners();
  }
}

/// What the app asks of the host about accounts. Nothing it returns holds a credential.
abstract interface class AccountsHost {
  /// The kinds and sign-in methods of [harness]'s provider.
  Future<AccountCatalog> catalog(String harness);

  /// The accounts in canoa.cfg, as the host reads them now.
  Future<List<AccountInfo>> list();

  /// Forgets an account's stored credential and its place, and takes it out of canoa.cfg.
  Future<void> remove(String name);

  /// The agent entries of the host that run a provider with accounts, and the accounts each offers.
  Future<List<AccountEntry>> entries();

  /// Makes the entry [entry] offer exactly [accounts], in that order, in the host's canoa.cfg, which the host alone writes. An entry
  /// it does not have is made when [harness] says what it runs. The host reads it when it starts.
  Future<void> offer(String entry, List<String> accounts, {String? harness});

  /// Signs in by [method] and makes the account [name], or signs in again to it when [again].
  SignIn signIn({required String name, required String harness, String? method, Map<String, String> inputs = const {}, bool again = false});
}

/// How long a sign-in may wait for the person: the host gives up on a question after ten minutes.
const Duration signInWait = Duration(minutes: 11);

/// The host's accounts over its control channel: management is local, so only this process's own link asks.
final class ControlAccountsHost implements AccountsHost {
  ControlAccountsHost(this._control, {DateTime Function()? now}) : _now = now ?? DateTime.now;

  final HostControl _control;
  final DateTime Function() _now;

  Future<Object?> _call(String method, Map<String, Object?> params) async =>
      (await _control.ready(wait: false)).call(method, params);

  @override
  Future<AccountCatalog> catalog(String harness) async => AccountCatalog.parse(await _call('accounts/catalog', {'harness': harness}));

  @override
  Future<List<AccountInfo>> list() async {
    final result = await _call('accounts/list', const {});
    final accounts = result is Map ? result['accounts'] : null;
    return [if (accounts is List) for (final each in accounts) ?AccountInfo.parse(each)];
  }

  @override
  Future<void> remove(String name) async {
    await _call('accounts/remove', {'name': name});
  }

  @override
  Future<List<AccountEntry>> entries() async {
    final result = await _call('accounts/entries', const {});
    final entries = result is Map ? result['entries'] : null;
    return [if (entries is List) for (final each in entries) ?AccountEntry.parse(each)];
  }

  @override
  Future<void> offer(String entry, List<String> accounts, {String? harness}) async {
    await _call('accounts/offer', {'entry': entry, 'accounts': accounts, 'harness': ?harness});
  }

  @override
  SignIn signIn({required String name, required String harness, String? method, Map<String, String> inputs = const {}, bool again = false}) {
    ControlClient? client;
    final run = SignIn(name: name, harness: harness, method: method, onCancel: () => unawaited(client?.close()));
    unawaited(() async {
      try {
        // A link of its own: closing it is how the host is told to stop.
        client = await _control.open(wait: false);
        if (!run.running) return;
        final secrets = await client!.linkSecrets();
        final result = await client!.call(
          again ? 'accounts/login' : 'accounts/add',
          {'name': name, if (!again) 'harness': harness, 'method': ?method, if (inputs.isNotEmpty) 'inputs': inputs},
          ControlListener(
            onNotification: (method, params) => _heard(run, method, params),
            onRequest: (method, params) => _asked(run, secrets, method, params),
          ),
          signInWait,
        );
        final made = result is Map && result['account'] is Map
            ? AccountInfo.parse({...(result['account'] as Map).cast<String, Object?>(), 'state': 'signed in'})
            : null;
        if (made == null) {
          run.fail('the host did not say which account it made');
        } else {
          run.complete(made);
        }
      } on ControlFailed catch (error) {
        run.fail(error.message);
      } on Object catch (error) {
        // A cancel closes the link, which ends the request with an error that is not news.
        if (run.running) run.fail('$error');
      } finally {
        unawaited(client?.close());
      }
    }());
    return run;
  }

  void _heard(SignIn run, String method, Map<String, Object?> params) {
    if (method != 'accounts/progress') return;
    if (params['say'] case final String text when text.isNotEmpty) run.say(text);
    if (params['open_url'] case final Map<Object?, Object?> page when page['url'] is String) {
      run.showPage(SignInPage(
        url: page['url'] as String,
        userCode: page['user_code'] as String?,
        expiresMs: page['expires_ms'] is int ? page['expires_ms'] as int : null,
      ));
    }
  }

  Future<Object?> _asked(SignIn run, LinkSecretIdentity? secrets, String method, Map<String, Object?> params) async {
    if (method != acpSecretRequestMethod) throw StateError('the caller answers no $method');
    final asked = await openSealedAsk(secrets, params, () => _now().millisecondsSinceEpoch ~/ 1000);
    if (asked == null) return const AcpSecretCancelled().toJson();
    return (await run.ask(asked.request, asked.environment)).toJson();
  }
}

/// Checks a form the host sealed to the key this link was proved by and readies the app's answer to it: what the person types is
/// sealed to the host and goes nowhere else. Null, which answers the host with a cancel, where the link has no secret to bind by
/// (nothing is asked) or the request is not bound to the host this link proved (it is never drawn as a form). A sign-in and a
/// program install put their questions to the person this way and no other.
Future<SignInAsk?> openSealedAsk(LinkSecretIdentity? secrets, Map<String, Object?> params, int Function() seconds) async {
  if (secrets == null) return null;
  final request = AcpSecretRequestParams.fromJson(params);
  secrets.identity.verifier.trust(secrets.host);
  final environment = SecretCardEnvironment(
    prover: secrets.identity.prover,
    verifier: secrets.identity.verifier,
    expected: secrets.host,
    host: 'Canoa',
    now: seconds,
  );
  try {
    await environment.verifier.collector(environment.expected, AcpSecretMethod.request, request.context, request.recipient, environment.seconds);
  } on AcpSecretException {
    return null;
  }
  return SignInAsk(request, environment);
}
