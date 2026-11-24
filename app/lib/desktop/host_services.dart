import 'dart:async';

import 'package:flutter/widgets.dart' show BuildContext, VoidCallback;

import '../accounts/accounts_controller.dart';
import '../history/agent_switch.dart';
import '../ledger/running_sessions.dart';
import '../mail/mail_accounts.dart';
import '../profile/profile_items.dart';
import '../providers/provider_catalog.dart';
import '../secrets/secrets_controller.dart';
import '../secrets/secrets_host.dart';
import '../served/host_files.dart' show HostFiles;
import '../settings/extension_settings.dart';
import '../settings/provider_switches.dart';
import '../store/catalog_sources.dart';
import '../settings/host_sections.dart';
import '../settings/shell_settings.dart';
import '../shell/panel_tabs.dart';
import '../store/programs.dart';
import '../store/store_extensions.dart';
import '../surfaces/terminal_controller.dart' show ControlTerminals;
import '../theme/vendor_looks.dart';
import '../usage/usage_model.dart';
import '../voice/host_speech.dart';
import '../voice/host_speech_output.dart';
import '../voice/record_capture.dart';
import '../workspace/host_folders.dart';
import 'control_client.dart' show ControlFailed, hostWords;
import 'host_control.dart';
import 'host_grants.dart';

/// Groups the control-channel clients, controllers and caches that reach the host this app talks to, keeping widget state for UI.
final class HostServices {
  HostServices({
    required this.control,
    HostFiles? hostFilesOverride,
    HostFolders? hostFoldersOverride,
    AudioCapture? speechCapture,
    MailHost? mailHostOverride,
    AgentMoves? agentMovesOverride,
    this._accountsController,
    VoidCallback? onGrantsChanged,
    VoidCallback? onFoldersChanged,
    VoidCallback? onProgramsChanged,
    VoidCallback? onExtensionsChanged,
    Future<void> Function()? afterProgramChange,
    Future<bool> Function()? onRestartForExtensions,
    Future<void> Function()? onExtensionsSaved,
    Future<void> Function(BuildContext, String)? onAfterExtensionInstall,
  }) : hostFiles = hostFilesOverride ?? (control == null ? null : HostFiles(control)),
       hostFolders = hostFoldersOverride ?? (control == null ? null : HostFolders(control)) {
    final c = control;
    grants = c == null ? null : (HostGrants(() => c.ready(wait: false))..addListener(onGrantsChanged ?? () {}));
    running = c == null
        ? null
        : RunningSessions(() async {
            final answer = await (await c.ready(wait: false)).call('sessions/running');
            if (answer is! Map || answer['sessions'] is! List) return const [];
            return [for (final each in answer['sessions'] as List) ?RunningSession.parse(each)];
          });
    programsHeld = c == null
        ? null
        : (ProgramStores(control: () => c.ready(wait: false), open: c.open, afterChange: afterProgramChange)
            ..addListener(onProgramsChanged ?? () {}));
    extensionsHeld = c == null
        ? null
        : (StoreExtensions(
            control: () => c.ready(wait: false),
            open: c.open,
            mayInstall: () => grants?.may(HostGrants.hosting) ?? false,
            restart: onRestartForExtensions,
            saved: onExtensionsSaved,
            afterInstall: onAfterExtensionInstall,
          )..addListener(onExtensionsChanged ?? () {}));
    hostSpeech = c == null
        ? null
        : HostSpeechCatalog(
            c,
            capture: speechCapture ?? const RecordCapture(),
            grants: grants,
            programs: ProgramStores(control: () => c.ready(wait: false), open: c.open, kind: 'programs'),
          );
    hostVoices = hostSpeech == null ? null : HostVoiceCatalog(hostSpeech!.call);
    terminals = c == null ? null : ControlTerminals(c);
    panelTabs = c == null ? null : PanelTabKinds((method, params) async => (await c.ready(wait: false)).call(method, params));
    hostSections = c == null ? HostSections.withoutHost() : HostSections((method, params) async => (await c.ready(wait: false)).call(method, params));
    secretsHeld = c == null ? null : SecretsController(host: ControlSecretsHost(c));
    final mailHost = mailHostOverride ?? (c == null ? null : ControlMailHost(c));
    mailHeld = mailHost == null ? null : MailAccountsController(host: mailHost);
    moves = agentMovesOverride ?? (c == null ? null : ControlAgentMoves(() => c.ready()));
    portable = moves == null ? null : PortableAgents(moves!);
    extensionSettingsHeld = hostFiles == null
        ? null
        : ExtensionSettings(
            files: hostFiles!,
            types: () async {
              final held = extensionsHeld;
              if (held == null) return const {};
              await held.refresh();
              return held.addedBy;
            },
            restart: onRestartForExtensions,
            saved: onExtensionsSaved,
          );
    providerSwitchesHeld = c == null ? null : ProviderSwitches((method, params) async => (await c.ready(wait: false)).call(method, params), restart: onRestartForExtensions);
    catalogSourcesHeld = c == null ? null : CatalogSources((method, params) async => (await c.ready(wait: false)).call(method, params));
    shellSettingsHeld = hostFiles == null
        ? null
        : ShellSettings(files: hostFiles!, restart: onRestartForExtensions, saved: onExtensionsSaved);
    if (hostFolders != null && onFoldersChanged != null) hostFolders!.addListener(onFoldersChanged);
  }

  final HostControl? control;
  final AccountsController? Function()? _accountsController;
  late final HostGrants? grants;
  late final RunningSessions? running;
  late final HostSpeechCatalog? hostSpeech;
  late final HostVoiceCatalog? hostVoices;
  final HostFiles? hostFiles;
  final HostFolders? hostFolders;
  late final ControlTerminals? terminals;
  late final PanelTabKinds? panelTabs;
  late final HostSections hostSections;
  late final UsageController? usageHeld = control == null
      ? null
      : UsageController(sources: [ControlUsageSource('This host', control!)], accounts: _accountsController?.call());
  late final SecretsController? secretsHeld;
  late final MailAccountsController? mailHeld;
  late final AgentMoves? moves;
  late final PortableAgents? portable;
  late final ProgramStores? programsHeld;
  late final StoreExtensions? extensionsHeld;
  late final ExtensionSettings? extensionSettingsHeld;
  late final ShellSettings? shellSettingsHeld;
  late final ProviderSwitches? providerSwitchesHeld;
  late final CatalogSources? catalogSourcesHeld;

  bool may(String grant) => grants?.may(grant) ?? false;

  /// Asks the host for the look families it lists.
  Future<void> loadLooks() async {
    final c = control;
    if (c == null) return;
    try {
      final client = await c.ready(wait: false);
      vendorLooks.adopt(await client.call('looks/list', const {}));
    } on Object catch (error) {
      vendorLooks.failed(error is ControlFailed || error is StateError ? hostWords(error) : 'no link');
    }
  }

  /// Asks the host which agents it has loaded.
  Future<void> loadProviders() async {
    final c = control;
    if (c == null) return;
    try {
      final client = await c.ready(wait: false);
      providerCatalog.adopt(await client.call('providers/list', const {}));
    } on Object catch (error) {
      providerCatalog.failed(error is ControlFailed || error is StateError ? hostWords(error) : 'no link');
    }
  }

  /// Asks the host which items a profile may set.
  Future<void> loadProfileItems({bool wait = false}) async {
    final c = control;
    if (c == null) return;
    try {
      final client = await c.ready(wait: wait);
      unawaited(hostSections.load());
      unawaited(panelTabs?.load());
      profileItems.adopt(await client.call('profile_items/list', const {}));
      if (profileItems.asksForClientCertificate) {
        try {
          profileCertificates.adopt(await client.call('network/client_certificates', const {}));
        } on Object {
          profileCertificates.failed();
        }
      }
    } on Object catch (error) {
      profileItems.failed(error is ControlFailed || error is StateError ? hostWords(error) : 'no link');
    }
  }

  void dispose() {
    running?.dispose();
    hostFolders?.dispose();
    unawaited(control?.close());
  }
}
