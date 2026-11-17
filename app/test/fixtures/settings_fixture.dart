import 'package:canoa/accounts/accounts_controller.dart';
import 'package:canoa/config/canoa_config.dart';
import 'package:canoa/ledger/session_ledger.dart';
import 'package:canoa/permissions/permission_rules.dart';
import 'package:canoa/profile/profile_home.dart';
import 'package:canoa/settings/settings_page.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/store/programs.dart';
import 'package:canoa/theme/look.dart';
import 'package:canoa/voice/host_speech.dart';
import 'package:canoa/voice/speech_backend.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

/// Settings as a page on its own, with every configuration it applies kept: Settings has no Save, so what a test reads is
/// the last one applied.
final class SettingsHarness {
  final List<CanoaConfig> applied = [];

  /// What canoa.cfg would hold now.
  CanoaConfig get last => applied.last;
}

Future<SettingsHarness> openSettings(
  WidgetTester tester, {
  CanoaConfig config = const CanoaConfig(),
  String? section,
  Size size = const Size(1100, 1800),
  PermissionRules? rules,
  SessionLedger? ledger,
  List<String> agents = const [],
  List<AgentSource> presets = const [],
  List<String> hosted = const [],
  Future<SourceCheck> Function(AgentSource source)? checkSource,
  ProgramStores? programs,
  Future<String?> Function(String template)? hostWorkspacePreview,
  Future<bool> Function()? onAllowNotifications,
  ProfileHome? home,
  VoidCallback? onLoadout,
  ThemeData? theme,
  String version = '',
  AccountsController? accounts,
  List<SpeechBackend> voiceBackends = const [],
  HostSpeechCatalog? hostSpeech,
}) async {
  await tester.binding.setSurfaceSize(size);
  addTearDown(() => tester.binding.setSurfaceSize(null));
  final harness = SettingsHarness();
  await tester.pumpWidget(MaterialApp(
    debugShowCheckedModeBanner: false,
    theme: theme ?? themeFor(LookId.canoa, Brightness.dark),
    home: SettingsPage(
      config: config,
      apply: (changed) async {
        harness.applied.add(changed);
        return changed;
      },
      section: section,
      rules: rules,
      ledger: ledger,
      profileAgents: agents,
      presets: presets,
      hosted: hosted,
      checkSource: checkSource,
      programs: programs,
      hostWorkspacePreview: hostWorkspacePreview,
      onAllowNotifications: onAllowNotifications,
      home: home,
      onLoadout: onLoadout,
      version: version,
      accounts: accounts,
      voiceBackends: voiceBackends,
      hostSpeech: hostSpeech,
    ),
  ));
  await tester.pumpAndSettle();
  return harness;
}

/// Picks a section from the list at the left (or the list page at phone width).
Future<void> goTo(WidgetTester tester, String id) async {
  await tester.tap(find.byKey(Key('settings-nav-$id')));
  await tester.pumpAndSettle();
}

/// Picks [label] from the dropdown with [key].
Future<void> choose(WidgetTester tester, String key, String label) async {
  await tester.tap(find.byKey(Key(key)));
  await tester.pumpAndSettle();
  await tester.tap(find.text(label).last);
  await tester.pumpAndSettle();
}
