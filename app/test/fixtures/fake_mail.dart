// A host's mail accounts the tests control: what it lists, what it refuses, and what each call was given.
import 'package:canoa/mail/mail_accounts.dart';

const canary = 'pw-CANARY-9d41c7e2';

MailAccountInfo mailAccount(
  String name, {
  String backend = 'ews',
  String? server = 'https://mail.example.test/EWS/Exchange.asmx',
  String? user,
  String state = 'password stored',
}) => MailAccountInfo(name: name, backend: backend, state: state, server: server, user: user);

final class FakeMailHost implements MailHost {
  FakeMailHost({List<MailAccountInfo> accounts = const [], this.refuseAdd, Map<String, MailTest>? tests}) : accounts = [...accounts], tests = {...?tests};

  final List<MailAccountInfo> accounts;

  /// What each account's test answers; an account with none signs in with seven folders.
  final Map<String, MailTest> tests;

  /// The host's words for refusing the next add, or null to take it.
  String? refuseAdd;

  final added = <NewMailAccount>[];
  final tested = <String>[];
  final removed = <String>[];

  @override
  Future<List<MailAccountInfo>> list() async => [...accounts];

  @override
  Future<MailAccountInfo> add(NewMailAccount account) async {
    if (refuseAdd case final why?) throw StateError(why);
    added.add(account);
    final made = MailAccountInfo(
      name: account.name,
      backend: account.backend,
      state: account.backend == 'outlook'
          ? 'signed in by Outlook'
          : account.password != null
          ? 'password stored'
          : 'signs in with the Windows logon',
      server: account.server,
      email: account.email,
      user: account.user,
    );
    accounts.add(made);
    return made;
  }

  @override
  Future<MailTest> test(String name) async {
    tested.add(name);
    return tests[name] ?? const MailTest(ok: true, folders: 7);
  }

  @override
  Future<void> remove(String name) async {
    removed.add(name);
    accounts.removeWhere((each) => each.name == name);
  }
}
