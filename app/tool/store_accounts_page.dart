// What the app does with the program Store and accounts of the host that served it, as a bare page for a browser test that cannot
// click through Flutter's canvas: the app's own code (HostGrants, ProgramStores, ControlAccountsHost, the served config) run by the
// real browser's key against a real `canoa portal` host. Built as a web target beside the app:
//   flutter build web --csp --no-web-resources-cdn -t tool/store_accounts_page.dart --output <dir>      (python tool/build_noise.py first)
// and loaded as <dir>/index.html, which runs this file's main.
import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';

import 'package:canoa/accounts/accounts_host.dart';
import 'package:canoa/acp/browser_connect.dart';
import 'package:canoa/acp/connect.dart';
import 'package:canoa/desktop/host_grants.dart';
import 'package:canoa/served/served_app.dart';
import 'package:canoa/store/programs.dart';
import 'package:coqui_acp/secret.dart';

/// Waits until [test] holds, polling; throws after [within].
Future<void> _until(bool Function() test, String what, {Duration within = const Duration(seconds: 60)}) async {
  final end = DateTime.now().add(within);
  while (!test()) {
    if (DateTime.now().isAfter(end)) throw StateError('timed out waiting for $what');
    await Future<void>.delayed(const Duration(milliseconds: 100));
  }
}

/// The page's only step. [options] holds `entry` (the Claude entry, run by a stand-in program), `name` (the account to add), `key`
/// (the API key typed into the sealed form) and `store` (the id of the made-up Store).
Future<String> _run(String options) async {
  final asked = jsonDecode(options) as Map<String, Object?>;
  final report = <String, Object?>{};
  final app = (await ServedApp.discover(base: Uri.base.replace(path: '/', query: '', fragment: '')))!;
  try {
    await app.probe();
    report['reachable'] = app.reachable;
    report['problem'] = app.problem;

    final grants = HostGrants(() => app.control.ready(wait: false));
    await grants.refresh();
    report['grants'] = {
      'known': grants.known,
      'programs': grants.may(HostGrants.programs),
      'accounts': grants.may(HostGrants.accounts),
    };

    // The Store: the made-up program is planned, installed on a link of its own and reported as installed.
    final provider = asked['store']! as String;
    final stores = ProgramStores(control: () => app.control.ready(wait: false), open: app.control.open);
    await stores.refresh();
    report['stores'] = [for (final status in stores.statuses) status.info.provider];
    report['copiesBefore'] = stores.status(provider)?.copies.length;
    final plan = await stores.plan(provider, 'install');
    report['plan'] = {'action': plan.action, 'version': plan.version};
    final installed = await stores.apply(plan).done;
    report['installed'] = installed;
    report['copiesAfter'] = stores.status(provider)?.copies.length;

    // The account: signed in by an API key typed into the form the host sealed to this browser's key.
    final name = asked['name']! as String;
    final host = ControlAccountsHost(app.control);
    final run = host.signIn(name: name, harness: 'claude', method: 'api-key');
    await _until(() => run.asking != null || !run.running, 'the sign-in to ask for the key');
    if (run.asking == null) throw StateError('the sign-in ended without asking: ${run.problem}');
    final ask = run.asking!;
    final environment = ask.environment;
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final collector = await environment.verifier.collector(
      environment.expected,
      AcpSecretMethod.request,
      ask.request.context,
      ask.request.recipient,
      now,
    );
    final sealed = {'api_key': await collector.seal('api_key', asked['key']! as String)};
    const plain = '{}';
    final auth = await collector.authenticate(environment.prover, plain, sealed, now);
    report['binding'] = auth.method;
    run.answer(AcpSecretAccepted(plain, sealed, auth));
    await _until(() => !run.running, 'the sign-in to end');
    report['signIn'] = run.phase.name;
    report['signInProblem'] = run.problem;
    final listed = await host.list();
    report['accounts'] = [for (final each in listed) each.name];

    // The entry offers the account, which the host serves from its next start; then a session runs on it.
    final entry = asked['entry']! as String;
    final current = (await app.loadConfig())!;
    final restart = await app.configStore.save(current.withAccountOffered(account: name, harness: 'claude', entry: entry));
    report['restart'] = restart;
    if (restart) {
      await app.files.restart();
      await app.files.awaitHost();
    }
    final listedEntry = app.host.entries.firstWhere((each) => each.name == entry);
    final client = await connectBrowserAcp(
      chatEndpoints(webSocket: listedEntry.url),
      pinnedPeerId: app.host.peer,
      method: app.host.method,
      initializeMeta: {'dev.boriken.canoa/account': name},
    );
    final cwd = (await app.files.listFolders()).entries.first.path;
    final opened = await client.openSession(cwd: cwd);
    report['session'] = opened.id;
    await client.close();
    report['ok'] = true;
  } on Object catch (error, stack) {
    report['error'] = '$error';
    report['stack'] = '$stack'.split('\n').take(6).join('\n');
  } finally {
    await app.close();
  }
  return jsonEncode(report);
}

void main() {
  globalContext
    ..setProperty('canoaRun'.toJS, ((JSString options) => _run(options.toDart).catchError((Object error) => jsonEncode({'error': '$error'})).then((answer) => answer.toJS).toJS).toJS)
    ..setProperty('canoaReady'.toJS, true.toJS);
}
