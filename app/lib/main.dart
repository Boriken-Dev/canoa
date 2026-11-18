import 'attention/notifier.dart';
import 'dart:async';
import 'dart:convert' show utf8;
import 'served/folder_dialog.dart';
import 'served/served_control.dart' show servedControl;
import 'workspace/remote_host.dart';
import 'served/served_app.dart';
import 'desktop/control_client.dart' show ControlFailed;
import 'config/canoa_config.dart';
import 'config/connection.dart' show Connection, ConnectionKind, ConnectionTransport;
import 'config/canoa_config_file.dart';
import 'desktop/desktop_shell.dart';
import 'desktop/host_control.dart';
import 'net/system_http.dart';
import 'desktop/host_supervisor.dart';
import 'desktop/host_tool.dart';
import 'sources/control_choices.dart';
import 'sources/source_choices.dart';
import 'providers/provider_catalog.dart';
import 'sources/source_icons.dart' show sourceProvider;
import 'extension/composer_extension.dart';
import 'extension/local_host.dart';
import 'extension/local_host_banner.dart';
import 'extension/local_host_control.dart';
import 'client/client_mode.dart';
import 'client/full_app.dart' as full_app;
import 'client/pane_link.dart';
import 'client/pane_pairing.dart';
import 'client/related_rail.dart';
import 'client/related_sessions.dart';
import 'client/relation.dart';
import 'client/relation_source.dart';
import 'extension/tab_bridge.dart' show TabBridge;
import 'extension/tab_shares.dart' show TabShares;
import 'extension_context.dart';
import 'acp/agent_commands.dart' show AgentCommand;
import 'acp/browser_connect.dart';
import 'acp/connect.dart' show chatEndpoints;
import 'acp/pipe_channel.dart' show isPipeUrl;
import 'acp/provider_connect.dart';
import 'acp/client.dart';
import 'acp/local_agent.dart';
import 'acp/secret_identity.dart';
import 'desktop/child_flag.dart';
import 'package:coqui_acp/page.dart';
import 'package:desktop_drop/desktop_drop.dart';
import 'connect_dialog.dart';
import 'package:file_selector/file_selector.dart';
import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';

import 'accounts/account_models.dart' show AccountEntry, accountMeta, machineLogin;
import 'accounts/accounts_controller.dart';
import 'accounts/accounts_host.dart';
import 'desktop/host_grants.dart';
import 'mail/mail_accounts.dart';
import 'chat/agent_events.dart' show Choice;
import 'chat/cache_lifetime.dart';
import 'chat/modes.dart' show modeEntries, modeOptionIn;
import 'chat/session_options.dart' show OptionRole, optionIn;
import 'chat/delegate_dialog.dart';
import 'chat/delegation_setup_dialog.dart';
import 'chat/git_changes.dart' show GitReader;
import 'chat/inspector.dart';
import 'chat/timing_log.dart';
import 'chat/inspector_surfaces.dart';
import 'chat/run_in_terminal.dart';
import 'surfaces/files_surface.dart';
import 'surfaces/terminal_controller.dart' show ControlTerminals;
import 'surfaces/terminal_remote.dart';
import 'surfaces/terminal_section.dart';
import 'surfaces/terminal_surface.dart';
import 'served/host_files.dart' show HostFiles, WorkspaceFiles;
import 'served/host_config.dart' show HostConfigChanged, HostConfigStore;
import 'served/host_state.dart';
import 'package:coqui_acp_chat/coqui_acp_chat.dart' show SessionUsage;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' as chat;
import 'chat/child_thread.dart';
import 'chat/clipboard.dart';
import 'chat/composer.dart';
import 'chat/dictation.dart';
import 'chat/reply_voice.dart';
import 'chat/delegation_row.dart';
import 'chat/delegation_view.dart';
import 'chat/new_session_view.dart';
import 'chat/palette.dart';
import 'chat/source_trouble.dart';
import 'chat/agent_events.dart' show AgentCommandInfo;
import 'chat/session_views.dart';
import 'package:flutter/services.dart' show KeyUpEvent, LogicalKeyboardKey;
import 'chat/update_log.dart';
import 'auth/peer_proof.dart' show PeerMethod;
import 'history/agent_switch.dart';
import 'history/session_import.dart';
import 'history/session_move.dart';
import 'ledger/host_ledger_backend.dart';
import 'ledger/ledger_backend.dart';
import 'ledger/running_sessions.dart';
import 'ledger/session_ledger.dart';
import 'chat/presence.dart';
import 'permissions/host_paths.dart';
import 'permissions/permission_rules.dart';
import 'popout/platform.dart' as popout_platform;
import 'popout/popout_app.dart';
import 'popout/popout_hub.dart';
import 'permissions/protected_paths.dart' show ProtectedPaths;
import 'permissions/rule_log.dart';
import 'profile/overlays.dart' show NoOverlays;
import 'profile/profile_home.dart' show ProfileHome;
import 'served/control_overlays.dart' show ControlOverlayManager;
import 'served/control_profile_home.dart' show ControlProfileHome;
import 'store/control_data.dart';
import 'store/overlay_needs.dart';
import 'store/programs_view.dart' show installRuntime;
import 'profile/profile_home_platform.dart';
import 'acp/usage_wire.dart' show ProfileApprover, noApproval;
import 'profile/approval_dialog.dart';
import 'profile/effective_setup.dart';
import 'profile/report.dart';
import 'settings/extension_settings.dart';
import 'settings/shell_settings.dart';
import 'settings/settings_page.dart';
import 'rail/session_groups.dart';
import 'desktop/hosting_panel.dart' show HostingPanel;
import 'desktop/knowledge_panel.dart' show KnowledgeSearchPanel;
import 'shell/hosts/host_status.dart' show ConnectionProbe, probeConnection;
import 'shell/hosts/hosts_controller.dart';
import 'shell/hosts/hosts_section.dart';
import 'shell/layout_backend.dart';
import 'shell/layout_controls.dart';
import 'shell/layout_state.dart';
import 'shell/surface.dart';
import 'shell/work_panel.dart';
import 'shell/sections.dart';
import 'usage/spend_model.dart' show SpendRow;
import 'usage/usage_model.dart';
import 'usage/usage_section.dart';
import 'shell/shell.dart';
import 'rail/session_rail.dart';
import 'session_store.dart';
import 'sources/readiness.dart';
import 'sources/source_checks.dart';
import 'sources/sources.dart';
import 'client/start_picker.dart';
import 'loadout/loadouts_index.dart';
import 'loadout/loadouts_section.dart';
import 'store/store_index_view.dart';
import 'store/store_section.dart';
import 'store/store_selection.dart';
import 'store/programs.dart';
import 'store/store_extension.dart' show ExtensionStoreType;
import 'store/store_extensions.dart';
import 'store/store_index.dart';
import 'welcome/welcome_view.dart';
import 'drafts/draft_backend.dart';
import 'drafts/draft_store.dart';
import 'theme/follow_provider.dart';
import 'theme/look.dart';
import 'theme/vendor_looks.dart';
import 'auth/browser_peer.dart' show verifyBrowserPeer;
import 'workspace/documents.dart' show homeFolder;
import 'workspace/host_folders.dart';
import 'workspace/open_in.dart';
import 'workspace/open_in_host.dart' show HostedOpening;
import 'served/web_mode.dart' show WebMode;
import 'workspace/open_in_ui.dart';
import 'workspace/scratch.dart';
import 'workspace/site.dart' show siteFqdn;
import 'workspace/site_meta.dart';
import 'worktree/close_dialog.dart';
import 'worktree/host.dart';
import 'worktree/worktree.dart';
import 'selectable.dart';
import 'voice/browser_voices.dart';
import 'voice/host_speech.dart';
import 'voice/host_speech_output.dart';
import 'voice/outputs.dart';
import 'voice/pcm_player.dart';
import 'voice/record_capture.dart';
import 'voice/reply_reader.dart';
import 'voice/system_speech.dart';

/// The folder [file] is in, else null.
String? _parentFolder(String? file) {
  if (file == null) return null;
  final text = file.replaceAll('\\', '/');
  final cut = text.lastIndexOf('/');
  return cut <= 0 ? null : text.substring(0, cut);
}

/// Whether this browser's extension is paired with a Canoa on this computer that answers now: then the session list is that
/// Canoa's. Asked first, with a bound, so a worker that is slow to answer costs a start the wait and no more.
Future<bool> _localHostKeepsSessions(LocalHostBridge? bridge) async {
  if (bridge == null) return false;
  try {
    final view = await bridge.status().timeout(const Duration(seconds: 8));
    return view.state == LocalHostState.running && view.paired;
  } on Object {
    return false;
  }
}

/// canoa.cfg on the Canoa on this computer, read through the extension's worker; null when the host is not reached or will not give
/// it whole, which leaves the panel with no loadouts to choose among.
Future<CanoaConfig?> _localHostConfig(LocalHostBridge bridge) async {
  final control = localHostControl(bridge);
  try {
    return await HostConfigStore(HostFiles(control)).load().timeout(const Duration(seconds: 8));
  } on Object catch (error) {
    debugPrint('Canoa: canoa.cfg on this computer cannot be read: $error');
    return null;
  } finally {
    await control.close();
  }
}

/// This desktop's own host. Let the browser extension reach this app too: the app registers itself as the extension's native
/// messaging host, so the extension's Start button starts the app.
HostSupervisor _desktopSupervisor(CanoaConfig config, List<String> args) {
  final tool = findHostTool();
  return HostSupervisor(
    tool,
    off: !config.localHost || args.contains('--no-local-host'),
    browserLaunch: browserLaunch(tool),
  );
}

Future<void> main(List<String> args) async {
  // The executable's own entry takes this before Flutter starts; reaching here means a runner built without it, which must
  // not answer a host's own command by opening another window.
  if (args.firstOrNull == canoaChildFlag) throw StateError('this build has no entry for $canoaChildFlag');
  // A window the app opened for one conversation runs this same program, and only shows that conversation.
  final popped = popout_platform.popoutLaunch(args);
  if (popped != null) return runPopout(popped);
  WidgetsFlutterBinding.ensureInitialized();
  // A page a `canoa` served uses that host for what a desktop does on its own computer; the probe says at once whether this
  // browser's key is accepted there, and a page that cannot reach its host's control channel keeps its sessions in the browser.
  final served = kIsWeb ? await ServedApp.discover() : null;
  await served?.probe();
  if (served != null) {
    // A page's Open in is the host's, by its mode; it only asks the host what it can start when the host is reached.
    workspaceOpener = served.opener;
    if (served.reachable) await served.openIn.load();
  }
  final hostCfg = served != null && served.reachable ? await served.loadConfig() : null;
  // The chat alone shows no Store or Settings, and a browser key is not granted what the overlays list needs.
  if (hostCfg != null && !ClientMode.fromUri(Uri.base).chatOnly) await served?.refreshOverlays();
  var config = hostCfg ?? await loadConfig();
  final shell = createDesktopShell();
  final host = served?.supervisor ??
      (shell.isDesktop ? _desktopSupervisor(config, args) : null);
  // The host starts first: it keeps the session list, which it answers once it is up.
  final hosting = host?.apply(configPath: desktopConfigPath());
  // In the host's home on a desktop that runs one or on the host that served the page, else beside canoa.cfg, or in this
  // origin's IndexedDB on the web.
  final keptByHost = host != null && host.phase != HostPhase.off && host.phase != HostPhase.absent;
  // What the app asks of another host for itself (its list, a probe of a saved connection) goes out through the Canoa on this
  // computer, which follows the system's proxy, login and certificates; a page's requests are its browser's.
  if (served == null && keptByHost) useHostNetwork(HostControl(host));
  // The extension's panel uses the Canoa on this computer by default, once its key is paired with it.
  final localHost = served == null && !keptByHost ? platformLocalHostBridge() : null;
  // The chat alone in the extension's panel chooses among the loadouts and profiles of the person's canoa.cfg on this computer.
  if (localHost != null && ClientMode.fromUri(Uri.base).chatOnly && await _localHostKeepsSessions(localHost)) {
    config = await _localHostConfig(localHost) ?? config;
  }
  final LedgerBackend ledgerBackend;
  if (served != null && served.reachable) {
    ledgerBackend = served.ledger;
  } else if (localHost != null && await _localHostKeepsSessions(localHost)) {
    ledgerBackend = HostLedgerBackend.over(localHostControl(localHost));
  } else if (served == null && keptByHost) {
    ledgerBackend = HostLedgerBackend(host);
  } else {
    ledgerBackend = platformLedgerBackend(configPath: desktopConfigPath());
  }
  final ledger = SessionLedger(ledgerBackend, settings: config.sessions);
  await ledger.load();
  final configFile = desktopConfigPath();
  // The rules, their log, drafts and the layout are the host's when the app reaches one that keeps them (the one that served a
  // page when its key may, so they follow the person to another browser; a desktop's own once it runs); else beside canoa.cfg, or
  // in this browser.
  final ownState = served == null && keptByHost ? HostState(HostControl(host)) : null;
  final hostState = served != null && served.reachable
      ? (await served.state.available() ? served.state : null)
      : (ownState != null && await ownState.available() ? ownState : null);
  if (ownState != null) {
    // Open in is the host's on this computer too: it asks the host what it can start and has it start it.
    final openIn = HostedOpening(mode: WebMode.local, opening: HostFiles(HostControl(host!)), hostPeer: null, page: Uri(), onThisComputer: true);
    workspaceOpener = WorkspaceOpener.hosted(openIn);
    await openIn.load();
  }
  final rules = PermissionRules(
    hostState == null ? platformLedgerBackend(configPath: configFile, name: 'permissions') : HostDocumentBackend(hostState, 'permissions.json'),
    log: hostState == null ? platformRuleLog(configPath: configFile) : HostRuleLog(hostState),
    guards: PermissionGuards(
      home: homeFolder(),
      config: _parentFolder(configFile),
      protected: config.protectedPaths == null ? null : ProtectedPaths(config.protectedPaths!),
    ),
  );
  await rules.load();
  final drafts = DraftKeeper(hostState == null ? platformDraftBackend(configPath: desktopConfigPath()) : HostDraftBackend(hostState));
  final layout = ShellLayout(backend: hostState == null ? platformLayoutBackend(configPath: desktopConfigPath()) : HostLayoutBackend(hostState));
  await layout.load();
  final settingsSignal = ValueNotifier<int>(0);
  final hostingSignal = ValueNotifier<int>(0);
  await shell.initialize(
    config,
    onSettings: () => settingsSignal.value++,
    onHosting: host == null || served != null ? null : () => hostingSignal.value++,
  );
  runApp(CanoaApp(
    config: config, shell: shell, settingsSignal: settingsSignal,
    host: host, hostingSignal: hostingSignal, ledger: ledger, rules: rules, notifier: platformNotifier(), drafts: drafts, served: served, layout: layout,
    localHost: localHost,
  ));
  await hosting;
}

/// Shared Flutter application for standalone desktop/web and extension views.
/// The extension's privileged browser permissions remain in its JS service worker.
class CanoaApp extends StatefulWidget {
  const CanoaApp({
    super.key, this.store, this.config = const CanoaConfig(),
    this.shell, this.settingsSignal, this.host, this.hostingSignal,
    this.configPath, this.writeConfig, this.readConfig, this.ledger, this.rules, this.notifier, this.drafts,
    this.isGitRepository, this.folderExists, this.home, this.hostFolders, this.listFolder, this.resolveInside, this.pickFolder, this.pickFiles, this.statFile, this.clipboard,
    this.worktrees, this.askChoices, this.renderers = chat.ChatRenderers.none, this.popoutWindows,
    this.tabBridge, this.localHost, this.browserProvider, this.gitReader, this.accountsHost, this.mailHost, this.agentMoves, this.served, this.layout, this.surfaces, this.probe, this.connectEndpoint, this.openRemote, this.terminalOnConnection, this.speechCapture, this.voiceOutputs, this.pcmPlayer,
    this.mode, this.relations, this.openPage, this.paneLink,
  });

  /// Which app this page shows: the whole app, or the chat alone with the sessions of what is in front of the client first. The
  /// address says by default (`mode=chat`); the extension's side panel and an Office task pane ask for the chat alone.
  final ClientMode? mode;

  /// What is in front of the client, which the sessions related to it are listed for. By default the one the address names
  /// (`relation=`), else the page in the extension's front tab; a client that learns it later gives a [ManualRelation].
  final RelationSource? relations;

  /// Opens a page of its own, for "Open the full app"; the browser's by default.
  @visibleForTesting
  final bool Function(Uri address)? openPage;

  /// The page that frames the chat-only app (an Office task pane), which tells it what is in front of the client as that changes;
  /// by default the framing page of this one, when it has one.
  @visibleForTesting
  final PaneLink? paneLink;

  /// Which section is shown and how each section's index is sized, kept across runs; the app keeps its own when none is given.
  final ShellLayout? layout;

  /// How the Hosts section looks at a saved connection; the app's own reads the host's list or connects.
  @visibleForTesting
  final ConnectionProbe? probe;

  /// What a saved Canoa host offers of a shell, for the Terminal section; the app's own asks the host's control channel.
  @visibleForTesting
  final Future<TerminalAccess> Function(Connection connection)? terminalOnConnection;

  /// How a chat reaches a host's endpoint; the browser's own proof unless a test stands in.
  final ConnectEndpoint? connectEndpoint;

  /// How a saved Canoa host is opened for a new session aimed at it: its list read and its control channel dialled; the app's own
  /// unless a test stands in.
  @visibleForTesting
  final Future<RemoteHost> Function(Connection connection)? openRemote;

  /// The views the work panel can show; the app's own (details and changes) when none are given.
  @visibleForTesting
  final List<WorkSurface>? surfaces;

  /// The host that served this page, when one did: where its sources, session list, folders, uploads, canoa.cfg and overlays are,
  /// over the control channel. Null on a desktop, whose host runs in its own process, and for a page nothing served.
  final ServedApp? served;

  /// Where the audio of a dictation through the host's providers comes from: the microphone by default.
  @visibleForTesting
  final AudioCapture? speechCapture;

  /// The voices replies are read aloud with: the browser's and the host's by default, and a test's own.
  @visibleForTesting
  final VoiceOutputs? voiceOutputs;

  /// What plays the audio a voice provider hands back: this platform's own by default, and a test's own.
  @visibleForTesting
  final PcmPlayer? pcmPlayer;

  /// How a conversation is shown in a window of its own: the platform's by default, and a test's own.
  final PopoutWindows? popoutWindows;

  /// What the app asks of the host about accounts; by default the host this app runs, over its control channel.
  @visibleForTesting
  final AccountsHost? accountsHost;

  /// The host's mail accounts, where a test supplies its own; the app asks the host it reaches otherwise.
  @visibleForTesting
  final MailHost? mailHost;

  /// What the app asks of the host to change a running conversation's agent; by default the host this app runs, over its control
  /// channel.
  @visibleForTesting
  final AgentMoves? agentMoves;

  /// The extension's tabs and the browser's provider of agents, by default the platform's; a test gives its own.
  @visibleForTesting
  final TabBridge? tabBridge;

  /// The extension's bridge to the Canoa on this computer, by default the platform's; a test gives its own.
  @visibleForTesting
  final LocalHostBridge? localHost;
  @visibleForTesting
  final Future<AcpPage?>? browserProvider;

  /// What the conversation draws beyond the package's own: Canoa's content kinds (a delegation card, a chart), a card for a kind of
  /// tool, the lines a burst or a turn folds into. Registered here, never in the transcript.
  final chat.ChatRenderers renderers;

  /// Where sessions not yet started are kept between runs, for a store this app makes.
  final DraftKeeper? drafts;

  /// The folder, file and clipboard services the composer uses, each the platform's by default.
  @visibleForTesting
  final bool Function(String folder)? isGitRepository;
  @visibleForTesting
  final bool Function(String folder)? folderExists;
  /// Where overlays and trust live, for a test that stands a host's in.
  @visibleForTesting
  final ProfileHome? home;
  /// The host's answers about folders, for a test that stands one in.
  @visibleForTesting
  final HostFolders? hostFolders;
  @visibleForTesting
  final Future<List<String>> Function(String folder)? listFolder;
  @visibleForTesting
  final Future<InsideFile?> Function(String folder, String relative)? resolveInside;
  @visibleForTesting
  final Future<String?> Function()? pickFolder;
  @visibleForTesting
  final Future<List<XFile>> Function()? pickFiles;
  @visibleForTesting
  final Future<int?> Function(String path)? statFile;
  @visibleForTesting
  final chat.ClipboardReader? clipboard;

  /// Makes worktrees for agents on this computer; by default through this app's host.
  @visibleForTesting
  final Worktrees? Function()? worktrees;

  /// Asks for a source's models and efforts before its agent runs; by default the host this app carries is asked.
  @visibleForTesting
  final ChoicesLoader? askChoices;

  /// Reads the repository in a session's folder; by default through the host the session's agent runs on.
  @visibleForTesting
  final GitReader? Function(ChatSession session)? gitReader;

  final SessionStore? store;

  /// How a session that needs the person reaches them outside the window; none, nothing does.
  final Notifier? notifier;

  /// The list of sessions kept across runs, for a store this app makes; a given [store] brings its own.
  final SessionLedger? ledger;

  /// What the person told Canoa to allow without asking, kept across runs, for a store this app makes.
  final PermissionRules? rules;
  final CanoaConfig config;
  final DesktopShell? shell;
  final ValueNotifier<int>? settingsSignal;

  /// This app's own host, on a desktop.
  final HostSupervisor? host;
  final ValueNotifier<int>? hostingSignal;

  /// Where canoa.cfg is and how it is written; by default the platform's place.
  @visibleForTesting
  final String? configPath;
  @visibleForTesting
  final Future<void> Function(CanoaConfig)? writeConfig;

  /// Reads canoa.cfg again after the host changed it; by default the person's own file on this computer.
  final Future<CanoaConfig> Function()? readConfig;

  @override
  State<CanoaApp> createState() => _CanoaAppState();
}

class _CanoaAppState extends State<CanoaApp> {
  late final SessionStore store;
  late final bool ownsStore;
  late CanoaConfig config;
  // The tray asks between frames, and a frame may not come by itself while the window is hidden.
  void _requestSettings() => scheduleMicrotask(() {
    if (mounted) unawaited(_showSettings());
  });
  void _requestHosting() => scheduleMicrotask(() {
    if (mounted) unawaited(_showHosting());
  });

  String? get _configPath => widget.configPath ?? desktopConfigPath();
  Future<void> _write(CanoaConfig changed) => (widget.writeConfig ?? _writeConfig)(changed);

  /// The store canoa.cfg is written through while a host answers: the host that served this page, or this app's own. Null for an
  /// app whose host does not run, which then writes the file itself, as a host that cannot start for what the file holds needs.
  HostConfigStore? get _configStore => widget.served?.configStore ?? (_hostUp ? _desktopConfig : null);

  /// This app's own host's store, made with the configuration the app started with, so it knows what the file held.
  HostConfigStore? _desktopConfig;

  Future<void> _writeConfig(CanoaConfig changed) async {
    final store = _configStore;
    if (store == null) {
      await saveConfig(changed);
      _desktopConfig?.adopt(changed);
      return;
    }
    await _writeToHost(store, changed);
  }

  /// Writes the tables of canoa.cfg that changed to the host, which refuses a table or a file it cannot take in its own words. A
  /// table the host reads only at start applies after the host restarts, which a served page offers.
  Future<void> _writeToHost(HostConfigStore store, CanoaConfig changed) async {
    if (!store.loaded) {
      _messenger.currentState?.showSnackBar(const SnackBar(
          content: Text('The host canoa.cfg was not read whole, so that change is kept in this page and not written to the host.')));
      return;
    }
    try {
      if (await store.save(changed) && widget.served != null) _offerRestart();
    } on HostConfigChanged catch (error) {
      if (mounted) setState(() => config = error.config);
      _settingsKey.currentState?.controller.adopt(error.config);
      _messenger.currentState?.showSnackBar(SnackBar(content: Text(error.message)));
    } on ControlFailed catch (error) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('The host did not save that: ${error.message}')));
    } on Object catch (error) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('The host could not be reached to save that: $error')));
    }
  }

  void _offerRestart() => _messenger.currentState?.showSnackBar(SnackBar(
    duration: const Duration(seconds: 20),
    content: const Text('The host reads that setting when it starts.'),
    action: SnackBarAction(label: 'Restart the host', onPressed: () => unawaited(_restartHost())),
  ));

  /// Has the host that served this page end and start again, which a host run with `--supervised` does, and waits for it: the
  /// sessions open on it end, and the control channel is dialled again by the next request.
  Future<bool> _restartHost() async {
    final served = widget.served;
    if (served == null) return false;
    final messenger = _messenger.currentState;
    try {
      if (_openOnHost() > 0 && !await _confirmRestart()) return false;
      await served.files.restart().onError<Object>((error, stack) {
        // The link ends with the host, which may be before its answer arrives; only a refusal is a failure.
        if (error is ControlFailed) throw error;
      });
      await served.files.awaitHost();
      messenger?.showSnackBar(const SnackBar(content: Text('The host restarted.')));
      return true;
    } on ControlFailed catch (error) {
      messenger?.showSnackBar(SnackBar(content: Text('The host would not restart: ${error.message}')));
    } on Object catch (error) {
      messenger?.showSnackBar(SnackBar(content: Text('The host did not come back: $error')));
    }
    return false;
  }

  /// The store extensions of the host's canoa.cfg, edited in Settings where this key may change the host's own settings.
  late final ExtensionSettings? _extensionSettingsHeld = _hostFiles == null
      ? null
      : ExtensionSettings(
          files: _hostFiles,
          types: () async {
            final held = _extensionsHeld;
            if (held == null) return const {};
            await held.refresh();
            return held.addedBy;
          },
          restart: _restartForExtensions,
          saved: _extensionsSaved,
        );

  ExtensionSettings? get _extensionSettings => _grants?.may(HostGrants.hosting) ?? false ? _extensionSettingsHeld : null;

  /// The shells the host found and which of them its terminals and agents use, chosen in Settings where this key may change the
  /// host's own settings.
  late final ShellSettings? _shellSettingsHeld = _hostFiles == null
      ? null
      : ShellSettings(files: _hostFiles, restart: _restartForExtensions, saved: _extensionsSaved);

  ShellSettings? get _shellSettings => _grants?.may(HostGrants.hosting) ?? false ? _shellSettingsHeld : null;

  /// Has the host start again so that it loads the extensions canoa.cfg now names; true when it did.
  Future<bool> _restartForExtensions() async {
    if (widget.served != null) return _restartHost();
    final host = widget.host, path = _configPath;
    if (host == null || path == null) return false;
    if (_openOnHost() > 0 && !await _confirmRestart()) return false;
    await host.apply(configPath: path);
    return true;
  }

  /// canoa.cfg was changed through the host: the app reads its copy again, so a later save of another table writes nothing of it back.
  Future<void> _extensionsSaved() async {
    try {
      final changed = widget.served != null ? await widget.served!.loadConfig() : await _readConfig();
      if (changed == null || !mounted) return;
      setState(() => config = changed);
      _settingsKey.currentState?.controller.adopt(changed);
    } on Object {
      // The change is written; what the app holds of it catches up the next time canoa.cfg is read.
    }
  }

  final _settingsKey = GlobalKey<SettingsPageState>();

  /// The speech services this build has, held for the app's life so a test of the microphone survives a rebuild.
  final _speech = platformSpeechBackends();
  final _loadoutKey = GlobalKey<StoreSectionState>();

  /// The loadout chosen in the Loadouts index and the one being edited, which a visit to the Store leaves as they were.
  late final _loadoutsSelection = LoadoutsSelection(layout: _layout);

  /// The section Settings opens at when it is first shown.
  String? _settingsFirst;

  late final ShellLayout _layout = widget.layout ?? ShellLayout();
  late final WorkSurfaces _surfaces = WorkSurfaces(widget.surfaces ?? [...builtInSurfaces(), FilesSurface(_filesOf), TerminalSurface(_terminalOf)]);

  /// Hosting is part of Local's overview, in the Hosts section.
  Future<void> _showHosting() async {
    _hosts.select('local');
    _showSection(HostsController.section);
  }

  /// Saves canoa.cfg once canoa finds nothing wrong with it, then serves it.
  /// Only [host] comes from the dialog: the rest is what the app has now, which Settings may have changed meanwhile.
  Future<String?> _saveHosting(CanoaConfig fromDialog) async {
    if (widget.served case final served?) return _saveHostingOnHost(served, fromDialog);
    final host = widget.host;
    final path = _configPath;
    if (host == null || path == null) return 'canoa.cfg has no place on this computer';
    // The person's file holds [host] as the dialog left it, so a value it took out falls back to a lower layer's.
    final changed = config.withHost(fromDialog.host, source: config.source.withOwn('host', fromDialog.source.own['host']));
    if (changed.hostEntries.isNotEmpty) {
      final problem = await withFileBeside(path, changed.encode(), (file) => host.check(hostCheckSpec(path, file)));
      if (problem != null) return problem;
    }
    await _write(changed);
    if (mounted) setState(() => config = changed);
    _settingsKey.currentState?.controller.adopt(changed);
    await host.apply(configPath: path);
    return null;
  }

  /// The page's Hosting save: the host reads what `[host]` would be and says in its own words what is wrong, then writes it.
  /// It serves the new table when it next starts, which is offered, since ending it ends the sessions open on it.
  Future<String?> _saveHostingOnHost(ServedApp served, CanoaConfig fromDialog) async {
    if (!served.configLoaded || !served.configStore.includesHost) {
      return "This page holds no copy of the host's settings it may write.";
    }
    final changed = config.withHost(fromDialog.host, source: config.source.withOwn('host', fromDialog.source.own['host']));
    try {
      final problem = await served.configStore.checkHost(changed);
      if (problem != null) return problem;
      final restart = await served.configStore.save(changed);
      if (mounted) setState(() => config = changed);
      _settingsKey.currentState?.controller.adopt(changed);
      if (restart) _offerRestart();
      return null;
    } on HostConfigChanged catch (error) {
      if (mounted) setState(() => config = error.config);
      _settingsKey.currentState?.controller.adopt(error.config);
      return error.message;
    } on ControlFailed catch (error) {
      return error.message;
    } on Object catch (error) {
      return 'The host could not be reached to save that: $error';
    }
  }

  Widget _hostingColumn(BuildContext context, HostSupervisor supervisor, {Future<int> Function()? freePort}) => Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
    HostingPanel(
      key: const Key('hosts-hosting-panel'),
      supervisor: supervisor,
      config: config,
      onSave: _saveHosting,
      freePort: freePort ?? freeLoopbackPort,
    ),
    const SizedBox(height: 24),
    Text('Knowledge search', style: Theme.of(context).textTheme.titleMedium),
    const SizedBox(height: 8),
    KnowledgeSearchPanel(key: const Key('hosts-knowledge-panel'), config: config, onSave: _saveHosting),
  ]);

  /// Shows Settings, at [section] when one is named.
  Future<void> _showSettings({String? section}) async {
    if (_mode.chatOnly) return _openFullApp();
    await _refreshOverlays();
    if (!mounted) return;
    if (section != null) {
      // The page is built from what the app holds when it is first shown, so it is built again with the section to open at.
      setState(() => _settingsFirst = section);
      _settingsKey.currentState?.show(section);
    }
    _layout.select('settings');
  }

  /// The Hosts section: this computer, the host that served this page, and the connections saved.
  late final HostsController _hosts = HostsController(
    layout: _layout,
    probe: widget.probe ?? probeConnection,
    save: (changed) async {
      await _write(changed);
      if (mounted) setState(() => config = changed);
      _forgetRemotes();
      _settingsKey.currentState?.controller.adopt(changed);
    },
    sessions: () => store.sessions,
    nearby: _control == null ? null : () async => nearbyOf(await _control.ready(wait: false)),
    local: widget.host == null || widget.served != null
        ? null
        : () => HostSummary.ofSupervisor(widget.host!, grants: _grants?.held),
    servedName: widget.served == null ? null : (Uri.base.authority.isEmpty ? 'Serving host' : Uri.base.authority),
    servedSummary: widget.served == null
        ? null
        : () => HostSummary(
            state: 'Serving this page',
            peer: widget.served!.host.peer,
            method: widget.served!.host.method.id,
            agents: [for (final entry in widget.served!.host.entries) if (entry.kind == 'agent') entry.title ?? entry.name],
            capabilities: _grants?.held == null ? const [] : HostSummary.capabilitiesOf(_grants!.held!),
          ),
    // A page the host served writes through the host, which takes the table from a key that holds config.write.
    mayWrite: () => widget.served == null || (_grants?.may(HostGrants.configWrite) ?? false),
    start: _connect,
    browserProvider: kIsWeb ? _provider : null,
    useBrowserAgents: _useBrowserAgents,
    hasShell: (item) => _terminalTargets().any((target) => target.id == item.id),
    openShell: (item) {
      _showSection('terminal');
      unawaited(_terminalSection.open(item.id, null));
    },
    onHostGone: (id) => unawaited(_terminalSection.closeHost(id)),
  );

  /// The Hosts section's index and main view, built from what the app holds now.
  Widget _hostsIndex() {
    _hosts.adopt(config);
    return HostsIndex(controller: _hosts);
  }

  Widget? _usageIndex() => _usage == null ? null : UsageIndex(controller: _usage!);

  Widget? _usagePage() => _usage == null
      ? null
      : UsagePage(
          controller: _usage!,
          warnPermille: config.warnPermille,
          onWarnChanged: (value) => unawaited(
            _applySettings(config.copyWith(warnPermille: value)).then((changed) => _settingsKey.currentState?.controller.adopt(changed)),
          ),
          onManageAccounts: _accounts == null ? null : () => unawaited(_showSettings(section: 'accounts')),
        );

  Widget _hostsPage() {
    _hosts.adopt(config);
    return HostsOverview(
      controller: _hosts,
      localDetail: widget.host == null || widget.served != null
          ? null
          : (context) => _hostingColumn(context, widget.host!),
      servedDetail: widget.served == null || !(_grants?.may(HostGrants.hosting) ?? false)
          ? null
          : (context) => _hostingColumn(context, widget.served!.supervisor, freePort: () async => 0),
    );
  }

  /// Settings as the section shows it.
  Widget _settingsPage() => SettingsPage(
      key: _settingsKey,
      config: config,
      apply: _applySettings,
      section: _settingsFirst,
      ledger: store.ledger,
      rules: store.rules,
      hostWorkspacePreview: _viaHost
          ? (template) => _hostFolders!.preview(template, source: _lastSource?.label ?? '', handler: _lastSource == null ? '' : handlerOf(_lastSource!))
          : null,
      onAllowNotifications: kIsWeb ? widget.notifier?.requestPermission : null,
      presets: [
        for (final source in _hostedSources)
          if (source.local && !config.agents.any((agent) => agent.label == source.label)) source,
      ],
      hosted: [for (final source in _hostedSources) if (!source.local) source.hostEntry!],
      checkSource: _sourceChecks.check,
      programs: _programs,
      profileAgents: _profileAgents,
      home: _home,
      onPin: (name, digest) async {
        final changed = config.withOverlayPin(name, digest);
        await _write(changed);
        if (mounted) setState(() => config = changed);
        _settingsKey.currentState?.controller.adopt(changed);
        return changed;
      },
      onLoadout: _showLoadout,
      accounts: _accounts,
      mail: _mail,
      extensions: _extensionSettings,
      shells: _shellSettings,
      usageSection: _usage != null,
      voiceBackends: _speech,
      hostSpeech: _hostSpeech,
      voiceOutputs: _voiceOutputs,
      reader: _reader,
      version: const String.fromEnvironment('CANOA_VERSION'),
    );

  /// Applies and writes what Settings changed, as it is changed, and gives back what the app now holds.
  /// [host] stays as the app has it: Hosting writes it on its own.
  Future<CanoaConfig> _applySettings(CanoaConfig fromPage) async {
    var changed = fromPage.withHost(config.host);
    // The host serves a source from the moment it starts, so adding or removing one restarts it, which ends the sessions open
    // on any of its sources. The profile tables are read again at each session, so changing them restarts nothing.
    var restart = widget.host != null && widget.served == null && !_sameAgents(changed.agents, config.agents);
    if (restart && _openOnHost() > 0 && !await _confirmRestart()) {
      changed = changed.copyWith(agents: config.agents);
      restart = false;
    }
    if (!mounted) return changed;
    setState(() => config = changed);
    // Voice mode turned on in a pop-out window is already on, and follows that window's conversation: only a change is applied.
    if (_reader.voiceMode != changed.voice.speak.readReplies) _reader.setVoiceMode(changed.voice.speak.readReplies);
    store.embedLimit = changed.embedLimit;
    store.queueDelivery = changed.sessions.queueDelivery;
    if (_checked) unawaited(_checkSources());
    store.ledger?.configure(changed.sessions);
    store.defaultMode = changed.defaultMode;
    store.bypassOnceOnly = changed.allowOnceOnly;
    await widget.shell?.update(changed);
    try {
      await _write(changed);
      if (restart && _configPath != null) await widget.host!.apply(configPath: _configPath);
    } on UnsupportedError {
      // canoa.cfg in web/PWA deployments is read-only.
    }
    return changed;
  }

  static bool _sameAgents(List<AgentCommand> a, List<AgentCommand> b) =>
      a.length == b.length &&
      [for (var i = 0; i < a.length; i++) a[i].label == b[i].label && a[i].program == b[i].program && _sameWords(a[i].args, b[i].args)]
          .every((same) => same);

  static bool _sameWords(List<String> a, List<String> b) =>
      a.length == b.length && [for (var i = 0; i < a.length; i++) a[i] == b[i]].every((same) => same);

  /// How many sessions are open on this computer's own host, which restarting it ends.
  int _openOnHost() => store.sessions
      .where((session) =>
          session.place == (widget.served == null ? SessionPlace.local : SessionPlace.host) &&
          session.status != SessionStatus.draft &&
          session.status != SessionStatus.disconnected)
      .length;

  Future<bool> _confirmRestart() async {
    final above = _navigator.currentContext;
    if (above == null || !above.mounted) return false;
    final count = _openOnHost();
    final ok = await showSelectableDialog<bool>(
      context: above,
      builder: (context) => AlertDialog(
        key: const Key('restart-host'),
        title: const Text('Restart the host?'),
        content: Text(widget.served == null
            ? 'A source added or removed takes effect when Canoa\'s host restarts, and that ends the '
                '${count == 1 ? 'session' : '$count sessions'} open on this computer now. Their conversations stay in the list '
                'to open again.'
            : 'Restarting the host ends the ${count == 1 ? 'session' : '$count sessions'} open on it now. Their conversations '
                'stay in the list to open again.'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: Text(widget.served == null ? 'Keep the sources as they were' : 'Not now')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Restart')),
        ],
      ),
    );
    return ok ?? false;
  }

  /// The agents the app offers a profile for: the host's own sources, the commands in canoa.cfg and any agent with a profile.
  List<String> get _profileAgents => {
    for (final source in _hostedSources) if (source.local) source.label,
    for (final agent in config.agents) agent.label,
    ...config.agentProfiles.keys,
  }.toList();

  /// Where overlays install and catalogs are fetched: the host that served this page, else beside canoa.cfg.
  ProfileHome? get _home => widget.home ?? widget.served?.home ?? _desktopHome();

  ProfileHome? _desktopHomeHeld;
  String? _desktopHomeFor;

  /// Beside canoa.cfg, with the overlay and catalog work done by this app's host, and the reading of layers and trust by it while it
  /// is up and here while it is not.
  ProfileHome? _desktopHome() {
    final path = _configPath;
    if (_desktopHomeFor == path && _desktopHomeHeld != null) return _desktopHomeHeld;
    final control = _control;
    final local = platformProfileHome(configPath: path, overlays: control == null ? const NoOverlays() : ControlOverlayManager(control));
    _desktopHomeFor = path;
    if (control == null || local == null) return _desktopHomeHeld = local;
    return _desktopHomeHeld = ControlProfileHome(control, overlays: local.overlays, os: local.os, local: local, hostUp: () => _hostUp);
  }

  /// Reads which overlays the host has installed, which the store shows as installed: the host that served this page, or this app's own.
  Future<void> _refreshOverlays() async {
    if (widget.served case final served?) return served.refreshOverlays();
    if (!_hostUp) return;
    if (_home?.overlays case final ControlOverlayManager overlays) {
      try {
        await overlays.refresh();
      } on Object catch (error) {
        debugPrint("Canoa: the host's overlays cannot be read: $error");
      }
    }
  }

  /// Whether the host answers: the one that served this page, or this app's own once it runs.
  bool get _hostUp =>
      widget.served != null || (widget.host != null && (widget.host!.phase == HostPhase.running || widget.host!.phase == HostPhase.idle));

  /// Shows the Store: agents, overlays, skills and MCP servers, installed and to discover.
  Future<void> _showLoadout() async {
    if (_mode.chatOnly) return _openFullApp();
    // What the host has installed changes without this page, and the store shows it from what was last read.
    await _refreshOverlays();
    if (!mounted) return;
    _layout.select('store');
    if (_loadoutKey.currentState case final page?) unawaited(page.controller.adopt(config));
  }

  /// Shows Loadouts: what new sessions start with.
  void _showLoadouts() => _showSection('loadouts');

  /// A new session that starts with the loadout [name], shown.
  void _useLoadout(String name) {
    _newSession();
    final draft = store.selected;
    if (draft != null && draft.isDraft) store.updateDraft(draft, loadout: name);
    _layout.select(ShellLayout.conversations);
  }

  /// The Loadouts section: the list and the editor of canoa.cfg's loadouts.
  Widget _loadoutsPage() => LoadoutsSection(
    config: config,
    home: _home,
    onSave: _saveLoadout,
    onStore: () => unawaited(_showLoadout()),
    onUse: _useLoadout,
    agents: _profileAgents,
    selection: _loadoutsSelection,
  );

  /// The Store as the section shows it.
  late final StoreSelection _storeSelection = StoreSelection(layout: _layout);

  Widget _loadoutPage() => StoreSection(
    key: _loadoutKey,
    embedded: true,
    selection: _storeSelection,
    contentsStorage: _control == null ? null : ControlContentsStorage(_control),
    contentsReader: _control == null ? null : ControlContentsReader(_control, os: _home?.os ?? 'linux'),
    config: config,
    home: _home,
    onSave: _saveLoadout,
    agents: _profileAgents,
    programs: _programs,
    extensionTypes: _storeTypes(),
    runtimes: _programs == null ? null : _extensionsHeld?.runtimes,
    // The host reads registries and looks programs up, for a page and the desktop alike.
    fetchRegistry: _control == null ? null : controlRegistryFetch(_control),
    findProgram: _control == null ? null : controlFindProgram(_control),
    overlayNeeds: _control == null ? null : controlOverlayNeeds(_control),
    index: ControlStoreIndex.over(_control)?.catalogs,
  );

  /// The store types after the built-in ones: Runtimes and what extensions add, where the host grants installing programs.
  List<ExtensionStoreType> _storeTypes() {
    final held = _programs == null ? null : _extensionsHeld;
    if (held == null) return const [];
    unawaited(held.ensure());
    return held.types;
  }

  /// Saves what Loadout changed. The host reads the profile again when each session starts, so nothing is restarted and no open
  /// session ends: the change reaches the next one.
  Future<CanoaConfig?> _saveLoadout(CanoaConfig changed) async {
    await _write(changed);
    if (mounted) setState(() => config = changed);
    return changed;
  }

  /// What the conversation shown offers the keyboard and the palette.
  final _conversation = _ConversationHandle();

  /// How the rail groups the sessions, which the keyboard's order follows.
  final ValueNotifier<SessionGrouping> _grouping = ValueNotifier(SessionGrouping.status);

  /// The conversations shown in a window of their own.
  late final PopoutHub _popout = PopoutHub(store, widget.popoutWindows ?? popout_platform.createPopoutWindows(), reader: _reader, onVoiceMode: _voiceModeChanged);

  /// This state sits above the app's Navigator, so its dialogs open in the Navigator's context.
  final _navigator = GlobalKey<NavigatorState>();
  final _messenger = GlobalKey<ScaffoldMessengerState>();
  Timer? _contextTimer;
  StreamSubscription<(ChatSession, AttentionKind)>? _attention;
  final List<VerifiedAgent> _ownedClients = [];

  /// Tells the person about [session] outside the window, when its kind is on and they are not looking at it there.
  Future<void> _notify(ChatSession session, AttentionKind kind) async {
    final notifier = widget.notifier;
    final wanted = switch (kind) {
      AttentionKind.permission => config.notifications.permission,
      AttentionKind.question => config.notifications.question,
      AttentionKind.finished => config.notifications.finished,
    };
    if (notifier == null || !wanted) return;
    if (session.id == store.selectedId && await notifier.attended()) return;
    final text = attentionText(session, kind);
    await notifier.show(title: text.title, body: text.body, sessionId: session.id);
  }

  /// The person chose a notification: its conversation, in the window.
  void _openFromNotification(String sessionId) {
    if (!mounted) return;
    store.select(sessionId);
    unawaited(widget.shell?.open());
  }

  void _showWaiting() => unawaited(widget.shell?.showWaiting(store.waiting));
  bool _claimingContext = false;

  /// The tabs the extension's panel shares with a session; null outside the extension.
  TabsComposerExtension? _tabs;
  LocalHostBridge? _localHost;

  late final ClientMode _mode = widget.mode ?? ClientMode.fromUri(Uri.base);

  /// What the next session started for what is in front of the chat-only app starts with, as the person chose it.
  final _start = StartChoice();

  /// The kind of client this is when it is a chat-only one (one of [clientKinds]): the one its address names, else what its
  /// relation shows, else a browser's when the page in front of a tab is what it follows; null for the whole app.
  String? get _clientKind => !_mode.chatOnly ? null : _mode.kind ?? (_relations is FrontTabRelation ? 'browser' : null);

  /// What `initialize` carries for a session started through the browser's provider: the loadout and the profile the person chose
  /// for it and the kind of client this is, which the host resolves against canoa.cfg's loadouts and `[defaults.clients]` once,
  /// when the session starts.
  Map<String, Object?> _providerMeta() => {
    if (_start.loadout case final name? when config.loadouts.containsKey(name)) loadoutMeta: name,
    if (_start.preset case final name? when config.presets.containsKey(name)) presetMeta: name,
    clientMeta: ?_clientKind,
  };

  /// What is in front of the client: the one the app was given, else the one its address names, else the page in front in the
  /// extension's window; null where nothing is.
  RelationSource? _relations;

  /// Whether [_relations] is the app's own, which it then ends.
  bool _ownsRelations = false;

  /// The page that frames the chat-only app and what it says of what is in front; null where none frames it.
  PaneLink? _pane;
  StreamSubscription<Relation?>? _paneRelations;

  /// Whether the chat-only app, served by a host that has a control channel, was turned away by it: its key is not accepted there,
  /// so the person pairs it by PIN here, where there is no Hosts section.
  bool get _needsPairing {
    final served = widget.served;
    return _mode.chatOnly && served != null && served.portal == null && served.host.control != null && !served.reachable;
  }

  /// Opens the whole app in a page of its own, from the chat-only app's link, and says so when this client cannot.
  void _openFullApp() {
    final opened = (widget.openPage ?? full_app.openPage)(ClientMode.fullAppOf(Uri.base));
    if (!opened) {
      _messenger.currentState?.showSnackBar(const SnackBar(content: Text('This page cannot open the whole app in a page of its own.')));
    }
  }

  /// Shows the section [id]; the chat-only app has no other than Conversations, so it opens the whole app there instead.
  void _showSection(String id) {
    if (_mode.chatOnly) {
      _openFullApp();
    } else {
      _layout.select(id);
    }
  }

  Future<AcpPage?> get _provider => widget.browserProvider ?? findAcp(timeout: const Duration(milliseconds: 300));

  Future<void> _claimContext() async {
    if (_claimingContext) return;
    _claimingContext = true;
    try {
      final context = await claimExtensionPageContext();
      if (context != null && mounted) store.addPageContext(context);
    } finally {
      _claimingContext = false;
    }
  }

  /// Each agent the person grants becomes a session. The provider decides the working directory it was granted.
  Future<void> _useBrowserAgents(AcpPage acp, {required String cwd, required String title}) async {
    final agents = await acp.request(purpose: 'Chat with it in Canoa');
    for (final agent in agents) {
      await _openProviderSession(acp, agent, cwd: cwd, title: title);
    }
  }

  Future<void> _openProviderSession(
    AcpPage acp, AcpPageAgent agent, {required String cwd, required String title, String? site, Relation? relation}) async {
    final meta = _providerMeta();
    final client = await connectProviderAgent(acp, agent, decider: store.askPermission, elicitor: store.askQuestion, initializeMeta: meta);
    try {
      // A site's session works in a folder the agent's host makes and names in its answer; the page names none.
      final opened = await client.openSession(cwd: cwd, setup: site == null ? AcpSessionSetup.none : siteSetup(site));
      final made = site == null ? null : madeFolder(opened.raw);
      if (site != null && made == null) throw StateError('The host did not say which workspace it made for this session');
      if (!mounted) throw StateError('Canoa UI closed');
      final local = store.addProviderSession(
        title: title.isEmpty ? agent.label : title,
        agent: agent.label,
        peerId: client.peerId,
        remoteSessionId: opened.id,
        cwd: made?.folder ?? cwd,
        site: site,
        relation: relation,
        workspaceId: made?.id,
        loadout: config.loadoutOfStart(
          loadout: meta[loadoutMeta] as String?,
          preset: meta[presetMeta] as String?,
          client: meta[clientMeta] as String?,
        ),
        agentNames: title.isEmpty,
      );
      store.applyOpened(local, opened);
      store.attachVerifiedClient(local.id, client);
      _ownedClients.add(client);
      await store.applyHostProfile(local, expected: _hasProfiles);
      // The browser's link hides the host's own report from the page, so the loadout is said here, from what canoa.cfg names.
      if (local.profile == null && local.loadout != null) local.chat.note('Loadout ${local.loadout}');
      if (config.lostClientLoadout(meta[clientMeta] as String?) case final lost? when meta[loadoutMeta] == null && meta[presetMeta] == null) {
        local.chat.note('canoa.cfg gives ${meta[clientMeta]} clients the loadout $lost, which it no longer holds: the default applies');
      }
    } catch (_) {
      await client.close();
      rethrow;
    }
  }

  /// A new session for [relation], what is in front of the client: each agent the person grants works in a folder of its own that
  /// its host makes in its Documents (under the page's site for a webpage, under the app's own for anything else), and the ledger
  /// notes the relation, the site and the folder. The relation stays here: the agent is told none of it.
  Future<void> _newRelatedSession(Relation relation) async {
    final acp = await _provider;
    if (acp == null) throw StateError('This browser has no provider of agents');
    final site = relation.kind == Relation.originKind ? siteFqdn(relation.key) : servedSite;
    if (site == null) throw StateError('${relation.display} is not a site a session can be kept for');
    final agents = await acp.request(purpose: 'Chat with it in Canoa about ${relation.display}');
    for (final agent in agents) {
      await _openProviderSession(acp, agent, cwd: siteStandInFolder, title: '', site: site, relation: relation);
    }
    // The next session starts from the defaults again, as a new draft does in the whole app.
    _start.clear();
  }

  /// Opens a session related to something again, in its own folder, through the browser's provider: the agent it ran on must
  /// still be granted to this browser, or the person is asked again.
  Future<void> _resumeRelated(LedgerEntry entry) async {
    if (entry.place != SessionPlace.browser) return _reopen(entry);
    final acp = await _provider;
    if (acp == null) throw StateError('This browser has no provider of agents');
    var agent = (await acp.agents()).where((each) => each.peer == entry.peerId).firstOrNull;
    agent ??= (await acp.request(purpose: 'Resume ${entry.title ?? 'a conversation'}')).where((each) => each.peer == entry.peerId).firstOrNull;
    if (agent == null) throw StateError('${entry.entry} is not granted to this browser now');
    final client = await connectProviderAgent(acp, agent, decider: store.askPermission, elicitor: store.askQuestion);
    try {
      await store.reopen(entry, client, expected: _hasProfiles);
      _ownedClients.add(client);
    } catch (_) {
      await client.close();
      rethrow;
    }
  }

  /// Whether canoa.cfg sets any profile, so an agent given none of it says so.
  bool get _hasProfiles => config.overlays.isNotEmpty || !config.profile.isEmpty || config.agentProfiles.isNotEmpty;

  late final HostControl? _control = widget.served?.control ?? (widget.host == null ? null : HostControl(widget.host!));

  /// The sessions the host keeps running for several clients, which Earlier conversations lists to join.
  late final RunningSessions? _running = _control == null
      ? null
      : RunningSessions(() async {
          final control = _control;
          final answer = await (await control.ready(wait: false)).call('sessions/running');
          if (answer is! Map || answer['sessions'] is! List) return const [];
          return [for (final each in answer['sessions'] as List) ?RunningSession.parse(each)];
        });

  /// The speech providers of the host this app reaches, which run what a page or a platform without a speech service of its own records.
  late final HostSpeechCatalog? _hostSpeech = _control == null
      ? null
      : HostSpeechCatalog(
          _control,
          capture: widget.speechCapture ?? const RecordCapture(),
          grants: _grants,
          programs: ProgramStores(control: () => _control.ready(wait: false), open: _control.open, kind: 'runtimes'),
        );

  /// Reads what the host's speech providers are, once, before a dictation looks for its service: a page that has just opened has
  /// not asked, and a host that was down is asked again.
  Future<void> _speechReady() async {
    final catalog = _hostSpeech, grants = _grants;
    if (catalog == null || (catalog.loaded && catalog.problem == null)) return;
    if (_speech.any((each) => each.id == config.voice.backend)) return;
    if (grants != null && !grants.known) await grants.refresh();
    if (grants == null || grants.may(HostGrants.voice)) await catalog.refresh();
  }

  /// Why no speech service is there to dictate with, in words, when the host is the reason.
  String? _noSpeechWhy() {
    final catalog = _hostSpeech, grants = _grants;
    if (catalog == null) return null;
    if (grants != null && grants.known && !grants.may(HostGrants.voice)) {
      return 'This host has not given this key the voice grant, so it cannot run a speech provider for it. Its canoa.cfg names the keys that may.';
    }
    if (catalog.problem != null) return "This host's speech providers are not available: ${catalog.problem}";
    if (catalog.loaded && catalog.providers.every((each) => each.problem != null)) {
      return 'No speech service is set up yet. Install one in Settings, under Voice: it runs on your host.';
    }
    return null;
  }

  /// The voice providers of the host this app reaches, which read replies aloud for a page and for a platform with no voices of its
  /// own. They run over the same control channel as the speech-to-text ones.
  late final HostVoiceCatalog? _hostVoices = _hostSpeech == null ? null : HostVoiceCatalog(_hostSpeech.call);

  /// Every voice this app or page can read aloud with.
  late final VoiceOutputs _voiceOutputs = widget.voiceOutputs ?? VoiceOutputs(host: _hostVoices, browser: browserSpeechOutputs(), load: _voicesReady);

  /// Reads the host's voice providers once, before the first reply is read: a page that has just opened has not asked, and a host
  /// that was down is asked again.
  Future<void> _voicesReady() async {
    final catalog = _hostVoices, grants = _grants;
    if (catalog == null) return;
    if (grants != null && !grants.known) await grants.refresh();
    if (grants == null || grants.may(HostGrants.voice)) await catalog.refresh();
  }

  /// The voice replies are read with now, from what Settings holds; null when none is there, which [_noVoiceWhy] says.
  Future<ReadingVoice?> _readingVoice() async {
    await _voiceOutputs.ready();
    final speak = config.voice.speak;
    final output = _voiceOutputs.resolve(speak.output);
    return output == null ? null : (output: output, choices: choicesFor(output, speak));
  }

  String? _noVoiceWhy() {
    final named = config.voice.speak.output, grants = _grants;
    if (named != null && _voiceOutputs.all.isNotEmpty) {
      return 'The voice $named is not available here. Choose another in Settings, under Voice.';
    }
    if (grants != null && grants.known && !grants.may(HostGrants.voice) && _voiceOutputs.browser.isEmpty) {
      return 'This host has not given this key the voice grant, so it cannot read replies aloud for it. Its canoa.cfg names the keys that may.';
    }
    if (_voiceOutputs.problem case final problem?) return "This host's voice providers are not available: $problem";
    return null;
  }

  /// Reads replies aloud: a reply's Play button, and voice mode, through the voice Settings holds.
  late final ReplyReader _reader = ReplyReader(
    voice: _readingVoice,
    player: widget.pcmPlayer ?? platformPcmPlayer(),
    whyNone: _noVoiceWhy,
    offered: () => _voiceOutputs.offered,
  )..setVoiceMode(config.voice.speak.readReplies);

  /// Voice mode was turned on or off in the composer: the choice is kept in canoa.cfg.
  void _voiceModeChanged(bool on) {
    if (config.voice.speak.readReplies == on) return;
    final changed = config.copyWith(voice: config.voice.copyWith(speak: config.voice.speak.copyWith(readReplies: on)));
    setState(() => config = changed);
    _settingsKey.currentState?.controller.adopt(changed);
    unawaited(_write(changed));
  }

  /// Dictation into the composer, with the service and choices Settings holds at the moment it starts.
  late final Dictation _dictation = Dictation(
    ready: _speechReady,
    whyNone: _noSpeechWhy,
    backend: () => resolveSpeech(config.voice.backend, _speech, _hostSpeech?.backends ?? const []),
    voice: () => config.voice,
    // Where the audio goes is told once per service, and kept.
    acknowledge: (id) {
      final changed = config.copyWith(voice: config.voice.copyWith(acknowledged: {...config.voice.acknowledged, id}));
      setState(() => config = changed);
      _settingsKey.currentState?.controller.adopt(changed);
      unawaited(_write(changed));
    },
    // What the service noted goes with the message as a block the agent reads.
    onFrontMatter: (notes) {
      final session = store.selected;
      return session == null ? 'There is no conversation to send it with.' : store.addDictationNotes(session, notes);
    },
  );

  late final HostFiles? _hostFiles = widget.served?.files ?? (_control == null ? null : HostFiles(_control));

  /// The host's answers about folders: whether one is there and in a repository, an `@`'s names, a chat's scratch folder.
  late final HostFolders? _hostFolders = (widget.hostFolders ?? (_control == null ? null : HostFolders(_control)))?..addListener(_foldersChanged);

  void _foldersChanged() {
    if (mounted) setState(() {});
  }

  /// Whether folders are asked of the host (a served page always, a desktop once its host runs) and not read from this disk.
  bool get _viaHost => _hostFolders != null && _hostUp;

  /// The files of the host a session runs on, when this app has a channel to it: its own host, or the one that served this page.
  WorkspaceFiles? _filesOf(ChatSession session) => _runsOnThisHost(session) ? _hostFiles : null;

  /// Whether [session] runs on the host this app reaches: the one that served this page, or this computer's own.
  bool _runsOnThisHost(ChatSession session) {
    final served = widget.served;
    return served == null
        ? session.place == SessionPlace.local || session.place == SessionPlace.host
        : session.peerId == null || session.peerId == served.host.peer;
  }

  late final ControlTerminals? _terminals = _control == null ? null : ControlTerminals(_control);

  /// What this app's own host offers of a shell: the Terminal section's Local row (a page's own host on a served page).
  Future<TerminalAccess> _localTerminal() async {
    final grants = _grants, terminals = _terminals;
    if (terminals == null || grants == null) return const TerminalRefused('This app has no host of its own to run a shell.');
    if (!grants.known) await grants.refresh();
    if (!grants.known) return const TerminalRefused('The host did not answer, so its grants are not known. Ask again once it runs.');
    if (grants.may(HostGrants.terminal)) return TerminalOnHost(terminals);
    return TerminalRefused(widget.served == null
        ? "This computer's host has not given this app a terminal."
        : "This host has not given this page's key a terminal. Its canoa.cfg names the keys that may have one.");
  }

  /// The hosts the Terminal section can open a shell on: the app's own, then each saved Canoa host.
  List<TerminalTarget> _terminalTargets() => [
    if (_terminals != null)
      TerminalTarget(
        id: widget.served != null ? 'served' : 'local',
        name: widget.served != null ? (Uri.base.authority.isEmpty ? 'Serving host' : Uri.base.authority) : 'Local',
        kind: widget.served != null ? 'Serves this page' : 'This computer',
        access: _localTerminal,
      ),
    for (final connection in config.connections)
      if (connection.kind == ConnectionKind.canoa)
        TerminalTarget(
          id: 'connection:${connection.name}',
          name: connection.name,
          kind: connection.kind.label,
          access: () => (widget.terminalOnConnection ?? terminalOnConnection)(connection),
        ),
  ];

  /// The folders the conversations on [targetId]'s host use.
  List<String> _terminalFolders(String targetId) => {
    for (final session in store.sessions)
      if (session.cwd case final folder? when folder.isNotEmpty && _terminalTargetOf(session) == targetId) folder,
  }.toList();

  /// The Terminal section's target for the host [session] runs on; null for a host it does not list.
  String? _terminalTargetOf(ChatSession session) {
    if (_filesOf(session) != null) return widget.served != null ? 'served' : 'local';
    if (session.place != SessionPlace.peer) return null;
    final name = config.connections.where((each) => each.peer == session.peerId).firstOrNull?.name;
    return name == null ? null : 'connection:$name';
  }

  /// Where a new shell from the Terminal section starts: the selected conversation's workspace on its own host.
  ({String target, String folder})? _terminalSelected() {
    final session = store.selected;
    if (session == null) return null;
    final target = _terminalTargetOf(session), folder = FilesSurface.workspaceOf(session);
    return target == null || folder == null || folder.isEmpty ? null : (target: target, folder: folder);
  }

  late final TerminalSectionController _terminalSection = TerminalSectionController(
    targets: _terminalTargets,
    workspaces: _terminalFolders,
    selected: _terminalSelected,
  );

  /// What the host [session] runs on offers of a shell: the terminal of the host this app reaches when it grants one; a direct agent
  /// endpoint says it offers none; nothing for any other host, which the Terminal section reaches.
  TerminalAccess? _terminalOf(ChatSession session) {
    if (_runsOnThisHost(session)) {
      final terminals = _terminals;
      return terminals != null && (_grants?.may(HostGrants.terminal) ?? false) ? TerminalOnHost(terminals) : null;
    }
    final direct = config.connections.where((each) => each.kind == ConnectionKind.acp && each.peer.toLowerCase() == session.peerId?.toLowerCase());
    return direct.isEmpty ? null : const TerminalRefused('This is a direct agent endpoint: it offers a session and no shell.');
  }
  /// Whether [session]'s folder is on a computer this app can ask about links and repositories: the host it is a client of.
  bool _pathsOnHost(ChatSession session) => _viaHost && _filesOf(session) != null;

  Worktrees? _localWorktrees() => _control == null ? null : controlWorktrees(_control);

  /// Reads the repository in [session]'s folder through the host its agent runs on; null where none can.
  GitReader? _gitReaderFor(ChatSession session) {
    if (widget.gitReader != null) return widget.gitReader!(session);
    if (session.place == SessionPlace.local) return _control == null ? null : HostGit(controlWorktreeCall(_control));
    final worktrees = _worktrees(session.place, endpoint: session.endpoint?.toString(), fallback: session.websocketFallback?.toString(),
        peerId: session.peerId, method: session.method);
    return worktrees is HostWorktrees ? HostGit(worktrees.call) : null;
  }

  /// Makes, reads and removes worktrees where a session's agent runs: by this app's host for one it runs, through the host for
  /// one it reached at [endpoint]; null where neither can.
  Worktrees? _worktrees(SessionPlace place, {String? endpoint, String? fallback, String? peerId, String? method}) {
    if (place == SessionPlace.local) return _localWorktrees();
    if (place != SessionPlace.host && place != SessionPlace.peer) return null;
    final uri = Uri.tryParse(endpoint ?? '');
    final proof = PeerMethod.values.where((each) => each.id == method).firstOrNull;
    if (uri == null || proof == null || peerId == null) return null;
    if (isPipeUrl(uri.toString())) {
      return HostWorktrees((name, params) => hostWorktreeRequestOverPipe(
        uri.toString(), expectedPeerId: peerId, method: name, params: params,
        verifyPeer: (channel, expected) => verifyBrowserPeer(channel, expected, proof),
        timeout: Duration(seconds: config.connectTimeout),
      ));
    }
    final overWebTransport = uri.scheme == 'https';
    final endpoints = chatEndpoints(
      webSocket: overWebTransport ? (fallback ?? '') : uri.toString(),
      webTransport: overWebTransport ? uri.toString() : '',
      preferWebTransport: config.preferWebTransport,
    );
    return HostWorktrees((name, params) => hostWorktreeRequest(
      endpoints, expectedPeerId: peerId, method: name, params: params,
      verifyPeer: (channel, expected) => verifyBrowserPeer(channel, expected, proof),
      timeout: Duration(seconds: config.connectTimeout),
    ));
  }

  /// Takes back a worktree made for a session that did not start, which nothing has changed yet.
  static Future<void> _unmake(Worktrees worktrees, SessionWorktree worktree) async {
    try {
      await worktrees.remove(worktree);
    } on Object catch (error) {
      debugPrint('Canoa: the worktree made for a session that did not start stays at ${worktree.path}: $error');
    }
  }

  /// Ends [session], and when it worked in a worktree of its own asks, once its agent has ended, whether to keep it.
  Future<void> _closeSession(ChatSession session) async {
    final worktree = session.worktree;
    final worktrees = worktree == null
        ? null
        : _worktrees(session.place, endpoint: session.endpoint?.toString(), fallback: session.websocketFallback?.toString(),
            peerId: session.peerId, method: session.method);
    await store.close(session);
    if (worktree == null || worktrees == null) return;
    WorktreeChoice? choice;
    WorktreeState? shown;
    do {
      WorktreeState? state;
      String? problem;
      try {
        state = await worktrees.status(worktree);
      } on Object catch (error) {
        problem = '$error';
      }
      if (state != null && !state.exists) {
        _messenger.currentState?.showSnackBar(SnackBar(content: Text('Its worktree ${worktree.path} was already gone.')));
        return;
      }
      // Changes made after the person was told of them are told again before anything is deleted.
      if (choice == WorktreeChoice.discard && state != null && shown != null && state.listing == shown.listing) break;
      final above = _navigator.currentContext;
      if (above == null || !above.mounted) return;
      choice = await showSelectableDialog<WorktreeChoice>(
        context: above,
        builder: (_) => WorktreeCloseDialog(worktree: worktree, state: state, problem: problem),
      );
      shown = state;
    } while (choice == WorktreeChoice.discard);
    if (choice == null || choice == WorktreeChoice.keep) return;
    try {
      final removed = await worktrees.remove(worktree, discard: choice == WorktreeChoice.discard);
      _messenger.currentState?.showSnackBar(SnackBar(content: Text(removed.branchKept
          ? 'Removed the worktree. Branch ${removed.branch} stays, with ${removed.commits} commit(s) of its own.'
          : 'Removed the worktree and its branch ${removed.branch}.')));
    } on Object catch (error) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('The worktree stays at ${worktree.path}: $error')));
    }
  }

  /// The worktree [entry] worked in, found again, or an error saying it is gone.
  Future<void> _findWorktree(LedgerEntry entry) async {
    final worktree = entry.worktree;
    if (worktree == null) return;
    final worktrees = _worktrees(entry.place, endpoint: entry.endpoint, fallback: entry.fallback, peerId: entry.peerId,
        method: entry.method);
    if (worktrees == null) throw StateError('This session worked in a worktree, which this build cannot reach');
    final state = await worktrees.status(worktree);
    if (!state.exists) {
      throw StateError('The worktree this session worked in is gone: ${worktree.path}, branch ${worktree.branch}');
    }
  }

  /// The agent entries the canoa this app runs is serving now, as it lists them.
  /// The other sources of this computer a session that ran on one can be moved to, by name and as shown.
  Map<String, String> _moveTargets(ChatSession session) => store.canMove(session) && _control != null && widget.served == null
      ? {
          for (final source in _hostedSources)
            if (source.local && source.label != session.agent) source.label: source.displayName,
        }
      : const {};

  /// Asks which source to move [session] to, has the host carry its history over, and opens the result beside it.
  Future<void> _moveTo(BuildContext context, ChatSession session) async {
    final control = _control;
    final to = control == null ? null : await pickTarget(context, targets: _moveTargets(session));
    if (to == null || control == null || !mounted || !context.mounted) return;
    final messenger = ScaffoldMessenger.of(context);
    try {
      await moveSession(
        store: store,
        control: await control.ready(),
        session: session,
        to: to,
        connect: (entry) => _linkAgent(entry, _cacheMeta((lifetime: CacheLifetime.chat, chosen: false))),
        keep: _ownedClients.add,
      );
    } on Object catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('That session could not be moved: ${reasonOf(error)}')));
    }
  }

  /// Whether the agents this app runs for itself can be changed inside a conversation: only where it runs them, not through a page
  /// another host serves.
  bool get _canSwitchAgents => widget.agentMoves != null || (_control != null && widget.served == null);

  List<AgentSource> get _hostedSources => hostedSources(widget.host?.listening ?? const []);

  /// The host's own sources whose earlier sessions it can bring in from the programs' folders, as the host says of each agent.
  List<AgentSource> get _importable => [
    for (final source in _hostedSources)
      if (source.local && providerCatalog.byId(source.harness)?.imports == true) source,
  ];

  /// The offer on the welcome page, when this app has a host that can do it. Nothing is read until the person asks.
  HistoryImport? _historyImport() {
    final control = _control;
    final sources = _importable;
    if (control == null || sources.isEmpty) return null;
    return HistoryImport(
      from: [for (final source in sources) source.displayName],
      list: () async => listImportable(await control.ready(), [
        for (final source in sources) (entry: source.label, harness: source.harness!, label: source.displayName),
      ]),
      bring: (chosen) async => importChosen(await control.ready(), chosen),
    );
  }

  bool _canListImported(ChatSession session) =>
      _control != null && session.place == SessionPlace.local && _importable.any((source) => source.label == session.agent);

  Future<List<AcpSessionInfo>> _importedFor(ChatSession session) async => importedSessions(await _control!.ready(), session.agent);

  /// Shows what an imported session says, read back from the host's copy.
  Future<void> _showImported(ChatSession from, AcpSessionInfo info) async {
    final said = historyMessages(await sessionHistory(await _control!.ready(), entry: from.agent, session: info.id));
    store.showImported(title: info.title ?? 'Imported session', agent: from.agent, said: said);
  }

  /// The sources Settings has: the host's own that it does not hide, canoa.cfg's `[agents.NAME]` commands, and the other agent
  /// entries of the canoa this app runs.
  List<AgentSource> get _sources => [
    for (final source in _hostedSources)
      if (source.local && !config.hiddenSources.contains(source.label)) source,
    for (final source in _hostedSources)
      if (!source.local) source,
  ];

  /// What each source needs and whether it is here, as last checked.
  Map<AgentSource, SourceCheck> get _checks => _sourceChecks.results;

  Future<SourceCheck> _check(AgentSource source) async {
    final listening = widget.host?.listening.where((listening) => listening.name == source.hostEntry).firstOrNull;
    if (listening == null) {
      return SourceCheck(problem: 'canoa is not serving ${source.label} now: open Hosting to see why.');
    }
    // The Store's extension for the source's program says whether it is here, which version and whether a newer one exists;
    // only a source no extension serves keeps the host's own list of what it is missing.
    if (_programs case final programs?) {
      final reported = await storeReadiness(programs, source);
      if (reported != null) return reported;
    }
    return SourceCheck(missing: listening.missing);
  }

  /// What the host this app reaches gives its key. The app shows the program Store and the accounts when it says so, and the
  /// host's own link holds every grant: the host is the one place that decides.
  late final HostGrants? _grants = _control == null ? null : (HostGrants(() => _control.ready(wait: false))..addListener(_grantsChanged));

  StreamSubscription<void>? _reconnects;

  /// The link to the host was made again after it ended (the host restarted, or the connection came back): what the page read of
  /// it before is read once more, since an account made or a grant changed meanwhile would otherwise stay unseen until a reload.
  Future<void> _hostCameBack() async {
    unawaited(_loadLooks());
    unawaited(_loadProviders());
    await _grants?.refresh();
    if (!mounted) return;
    if (_grants?.may(HostGrants.accounts) ?? false) unawaited(_accountsHeld?.refresh());
    if (_grants?.may(HostGrants.usage) ?? false) unawaited(_usageHeld?.refresh());
    if (_grants?.may(HostGrants.voice) ?? false) {
      unawaited(_hostSpeech?.refresh());
      unawaited(_hostVoices?.refresh());
    }
  }

  /// A grant arrived or changed: what it opens is listed, and a source's readiness may now name its program.
  void _grantsChanged() {
    if (!mounted) return;
    setState(() {});
    if (_grants?.may(HostGrants.accounts) ?? false) unawaited(_accountsHeld?.refresh());
    if (_grants?.may(HostGrants.usage) ?? false) unawaited(_usageHeld?.refresh());
    if (_grants?.may(HostGrants.voice) ?? false) {
      unawaited(_hostSpeech?.refresh());
      unawaited(_hostVoices?.refresh());
    }
    if (_programs != null && _checked) unawaited(_checkSources());
  }

  /// The Usage section: the limits of the host this app reaches, which the section shows when the host grants `usage`.
  late final UsageController? _usageHeld = _control == null
      ? null
      : UsageController(sources: [ControlUsageSource('This host', _control)], accounts: _accountsHeld);

  UsageController? get _usage => _grants?.may(HostGrants.usage) ?? false ? _usageHeld : null;

  /// The accounts of the host this app reaches, for Settings and the composer's chip; none where the app reaches no host or the
  /// host does not grant them.
  AccountsController? get _accounts => widget.accountsHost != null || (_grants?.may(HostGrants.accounts) ?? false) ? _accountsHeld : null;

  late final AccountsController? _accountsHeld = _buildAccounts();

  /// The mail accounts of the host this app reaches, for Settings; none where the app reaches no host or the host does not grant
  /// setting them up.
  MailAccountsController? get _mail => widget.mailHost != null || (_grants?.may(HostGrants.mailSetup) ?? false) ? _mailHeld : null;

  late final MailAccountsController? _mailHeld = _buildMail();

  MailAccountsController? _buildMail() {
    final control = _control;
    final host = widget.mailHost ?? (control == null ? null : ControlMailHost(control));
    return host == null ? null : MailAccountsController(host: host);
  }

  /// How a running conversation changes agent: the host's own export and import, over its control channel.
  late final AgentMoves? _moves = widget.agentMoves ?? _controlMoves();
  AgentMoves? _controlMoves() {
    final control = _control;
    return control == null ? null : ControlAgentMoves(() => control.ready());
  }
  late final PortableAgents? _portable = _moves == null ? null : PortableAgents(_moves);

  AccountsController? _buildAccounts() {
    final control = _control;
    final host = widget.accountsHost ?? (control == null ? null : ControlAccountsHost(control));
    if (host == null) return null;
    return AccountsController(
      host: host,
      config: () => config,
      change: _changeAccounts,
      holder: _accountHolder,
      usage: (id) => store.accountUsage[id],
      // A page a host served is not on the host's machine as far as its browser's loopback goes.
      hostIsHere: widget.served == null,
    );
  }

  /// `initialize`'s `_meta` for the account a session runs on: its name, never a credential. Absent runs the entry's first.
  Map<String, Object?> _accountMeta(String? account) =>
      account == null ? const {} : {accountMeta: account == machineLogin ? null : account};

  /// The open session that runs on [account], which a login that rotates its credential allows only one of.
  String? _accountHolder(String account) {
    for (final session in store.sessions) {
      if (!session.isDraft && session.status != SessionStatus.disconnected && session.account == account) return session.title;
    }
    return null;
  }

  /// Writes what changes which entries offer which accounts and has the host serve it. The host wrote the account's own table in
  /// the person's file, so it is read again first: saving the copy this app held would take it out.
  Future<void> _changeAccounts(CanoaConfig Function(CanoaConfig config) edit) async {
    final path = _configPath;
    final fresh = await _readConfig();
    // What entries offer is the host's to say, and the host alone writes canoa.cfg for it: the edit is made over the entries the host
    // lists, and each entry it changes is asked of the host, which makes the entry too when there is none. This app reads the file
    // again afterwards and never writes it from here, so a write of this app's copy cannot undo what the host just wrote.
    final entries = _accountsHeld?.hostEntries ?? const <AccountEntry>[];
    final view = fresh.withHostEntries(entries);
    final edited = edit(view);
    final offers = <String, List<String>>{
      for (final entry in edited.hostEntries)
        if (!_sameList(edited.hostAccountsOf(entry.name), view.hostAccountsOf(entry.name))) entry.name: edited.hostAccountsOf(entry.name),
    };
    final accountsHost = widget.accountsHost ?? (_control == null ? null : ControlAccountsHost(_control));
    for (final MapEntry(:key, :value) in offers.entries) {
      await accountsHost?.offer(key, value, harness: edited.hostHarnessOf(key));
    }
    // The host serves what an entry offers from the moment it starts, so a change of the offers restarts it, which ends the
    // sessions open on it: the person is asked first, and says which. Declined, it is saved and applies when the host next starts.
    final canRestart = widget.served != null || (widget.host != null && path != null);
    var restart = canRestart && offers.isNotEmpty;
    if (restart) {
      final ending = _endedByRestart();
      if (ending.isNotEmpty && !await _confirmAccountsRestart(ending)) restart = false;
    }
    final changed = await _readConfig();
    if (mounted) setState(() => config = changed);
    _settingsKey.currentState?.controller.adopt(changed);
    await _accountsHeld?.refresh();
    _accounts?.changed();
    if (!restart) return;
    if (widget.served != null) {
      await _startHostAgain();
    } else {
      await widget.host!.apply(configPath: path);
    }
    await _accountsHeld?.refresh();
  }

  /// canoa.cfg as it is now: the person's own file on this computer, or the copy this page holds where the host that served it keeps
  /// the file.
  Future<CanoaConfig> _readConfig() async {
    if (widget.readConfig case final read?) return read();
    return widget.writeConfig == null && widget.served == null ? loadConfig() : config;
  }

  static bool _sameList(List<String> a, List<String> b) => a.length == b.length && [for (var i = 0; i < a.length; i++) a[i] == b[i]].every((same) => same);

  /// Has the host that served this page end and start again, and waits for it to answer: the offers just written apply from its start.
  Future<void> _startHostAgain() async {
    final served = widget.served;
    if (served == null) return;
    try {
      await served.files.restart().onError<Object>((error, stack) {
        // The link ends with the host, which may be before its answer arrives; only a refusal is a failure.
        if (error is ControlFailed) throw error;
      });
      await served.files.awaitHost();
    } on ControlFailed catch (error) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('The host would not restart, so that applies when it next starts: ${error.message}')));
    } on Object catch (error) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('The host did not come back: $error')));
    }
  }

  /// The titles of the sessions open on this computer's own host or on an entry it serves, which restarting the host ends.
  List<String> _endedByRestart() => [
    for (final session in store.sessions)
      if ((session.place == SessionPlace.local || session.place == SessionPlace.host) &&
          session.status != SessionStatus.draft &&
          session.status != SessionStatus.disconnected &&
          session.status != SessionStatus.preview)
        session.title,
  ];

  Future<bool> _confirmAccountsRestart(List<String> ending) async {
    final above = _navigator.currentContext;
    if (above == null || !above.mounted) return false;
    final ok = await showSelectableDialog<bool>(
      context: above,
      builder: (context) => AlertDialog(
        key: const Key('restart-accounts'),
        title: const Text('Restart the host?'),
        content: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text('What a source offers takes effect when the host restarts, and that ends '
              '${ending.length == 1 ? 'this session' : 'these ${ending.length} sessions'}. '
              'The conversations stay in the list to open again.'),
          const SizedBox(height: 8),
          for (final title in ending.take(8)) Text('• $title', key: const Key('restart-accounts-session')),
          if (ending.length > 8) Text('and ${ending.length - 8} more'),
          const SizedBox(height: 8),
          const Text('Or leave it: it is saved, and applies the next time the host starts. Until then the running host still offers '
              'what it did, so an account removed meanwhile fails to start a session.'),
        ]),
        actions: [
          TextButton(key: const Key('restart-accounts-later'), onPressed: () => Navigator.pop(context, false), child: const Text('Leave it for now')),
          FilledButton(key: const Key('restart-accounts-now'), onPressed: () => Navigator.pop(context, true), child: const Text('Restart now')),
        ],
      ),
    );
    return ok ?? false;
  }

  /// The programs the host's Store keeps. Nothing is asked of the host until a page that shows them, or a source's readiness,
  /// needs it; an install runs only when the person confirms its plan.
  late final ProgramStores? _programsHeld = _control == null
      ? null
      : (ProgramStores(control: () => _control.ready(wait: false), open: _control.open, afterChange: _hostReadsPrograms)
          ..addListener(_programsChanged));

  /// Runtimes and the types extensions add, which the host lists and installs from; asked only once the Store is shown.
  late final StoreExtensions? _extensionsHeld = _control == null
      ? null
      : (StoreExtensions(
          control: () => _control.ready(wait: false),
          open: _control.open,
          mayInstall: () => _grants?.may(HostGrants.hosting) ?? false,
          restart: _restartForExtensions,
          saved: _extensionsSaved,
        )..addListener(_extensionsChanged));

  void _extensionsChanged() {
    if (mounted) setState(() {});
  }

  /// The host reads which program a source runs when it starts: after one was installed, updated or removed it starts again, so the
  /// source runs the new one. The person is asked first when sessions are open on it.
  Future<void> _hostReadsPrograms() async {
    if (!mounted || (widget.served == null && (widget.host == null || _configPath == null))) return;
    final ending = _endedByRestart();
    if (ending.isNotEmpty && !await _confirmAccountsRestart(ending)) return;
    if (widget.served != null) {
      await _startHostAgain();
    } else {
      await widget.host!.apply(configPath: _configPath);
    }
  }

  /// The programs the host's Store keeps, when the host grants installing them.
  ProgramStores? get _programs => _grants?.may(HostGrants.programs) ?? false ? _programsHeld : null;

  String? _programsSeen;

  /// A program found, installed, updated or removed changes whether its sources can run.
  void _programsChanged() {
    final now = _programs?.signature;
    if (now == _programsSeen) return;
    _programsSeen = now;
    if (mounted && _checked) unawaited(_checkSources());
  }

  /// [_askChoices] as the entry's account [account], which lists what that account may use; null for a source the host does not
  /// run from canoa.cfg.
  Future<SourceChoices?> _askChoicesAs(AgentSource source, String account) async {
    if (source.host != null) return _askChoicesOn(source, account: account);
    final control = _control;
    return control == null ? null : choicesOverControl(control, source, account: account);
  }

  /// What the host [source] belongs to says of it, over that host's own control channel; null for a host that has none.
  Future<SourceChoices?> _askChoicesOn(AgentSource source, {String? account}) async {
    final remote = _remoteReady[source.host];
    if (remote == null || !remote.hasControl) return null;
    return choicesOverControl(remote.control, source, account: account);
  }

  /// What the host says of [source]: the models and efforts its provider lists with no session, which the host asks (a preset by
  /// its harness, a hosted entry by its name in canoa.cfg, a program entry through a short session of its own).
  Future<SourceChoices?> _askChoices(AgentSource source) async {
    if (widget.askChoices case final ask?) return ask(source);
    if (source.host != null) return _askChoicesOn(source);
    final control = _control;
    return control == null ? null : choicesOverControl(control, source);
  }

  /// Whether the sources have been looked for: only once a new session or Settings needs them, since each runs a lookup.
  bool _checked = false;

  Future<void> _checkSources() async {
    _checked = true;
    // A change of the Store's programs can come while a page is built, and a source-less host has nothing to wait for.
    await null;
    await _sourceChecks.refresh(_sources);
  }

  /// Looks sources up side by side and shows each as its answer arrives.
  late final SourceChecks _sourceChecks = SourceChecks(_check, onChange: () {
    if (mounted) setState(() {});
  });

  /// The source and folder chosen last, which a new session starts with.
  AgentSource? _lastSource;
  /// The folder chosen last on each host: this app's own under null, a saved connection's under its name.
  final Map<String?, String> _lastFolders = {};
  String? get _lastFolder => _lastFolders[null];

  /// Whether the drafts the last run kept have been put back, so that the window does not make a new one first.
  bool _restored = false;
  bool _landing = false;

  /// The window with no session lands on the page of choices for a new one, once there is a source to start it from: the draft
  /// is made here and is no session, with no row, until its first message is sent.
  void _land() {
    if (!_restored || _landing || !mounted || store.selected != null || _sources.isEmpty) return;
    _landing = true;
    scheduleMicrotask(() {
      _landing = false;
      if (mounted && store.selected == null && _sources.isNotEmpty) _newSession();
    });
  }

  /// Whether no agent can start here: every source has been looked at and none is ready (or there is none).
  bool get _noAgent => _sources.every((source) => _checks[source] != null && !_checks[source]!.ready);

  void _noteLast() {
    _land();
    final session = store.selected;
    if ((session == null || session.isDraft) && !_checked) unawaited(_checkSources());
    if (session == null || session.source == null) return;
    _lastSource = session.source;
    if (session.isDraft && session.cwd != null) _lastFolders[session.host] = session.cwd!;
    if (session.isDraft) _hint(session);
  }

  /// Folders sessions worked in on [host], the last chosen first: this app's own host for null, else the saved connection of that
  /// name. A worktree's stands for its repository. One that is gone, or written for another system, is left out.
  List<String> _recentFoldersOn(String? host) {
    final connection = _connection(host);
    final remote = host == null ? null : _remoteReady[host];
    bool onHost(SessionPlace place, String? peer) =>
        connection == null ? place != SessionPlace.peer : place == SessionPlace.peer && peer?.toLowerCase() == connection.peer.toLowerCase();
    final written = <String>{
      ?_lastFolders[host],
      for (final session in store.sessions)
        if (session.source != null && session.cwd != null && session.kind == SessionKind.workspace &&
            (connection == null ? session.host == null && session.place != SessionPlace.peer : onHost(session.place, session.peerId)))
          session.worktree?.repo ?? session.cwd!,
      for (final entry in store.ledger?.entries ?? const <LedgerEntry>[])
        if (entry.kind == SessionKind.workspace &&
            (connection == null ? (entry.place == SessionPlace.local || entry.place == SessionPlace.host) : onHost(entry.place, entry.peerId)))
          ?(entry.worktree?.repo ?? entry.cwd),
    };
    // One entry per workspace, however the host wrote its path (a share's `\\?\UNC\...` or `//...` form, or another case).
    final folders = distinctWorkspaces(written);
    if (connection != null) {
      // A saved host is asked about its own folders, and a path of another system's is never offered for it.
      return [
        for (final folder in folders)
          if (pathFitsSystem(remote?.os, folder) && (remote?.folders.exists(folder) ?? true)) folder,
      ].take(8).toList();
    }
    return folders.where(widget.folderExists ?? (_viaHost ? _hostFolders!.exists : (_) => true)).take(8).toList();
  }

  List<String> get _recentFolders => _recentFoldersOn(null);

  /// A new session with nothing started, from the source and folder chosen last; one already waiting untouched is shown
  /// instead of another.
  void _newSession() {
    if (!canRunLocalAgents && widget.host == null) {
      store.createPreviewSession();
      return;
    }
    // The page of choices is one at a time: a draft already waiting, touched or not, is shown instead of another.
    for (final session in store.sessions) {
      if (session.status == SessionStatus.draft && session.messages.isEmpty) {
        store.select(session.id);
        return;
      }
    }
    if (!_checked) unawaited(_checkSources());
    final sources = _sources;
    final source = sources.contains(_lastSource)
        ? _lastSource
        : sources.where((source) => _checks[source]?.ready ?? false).firstOrNull ?? sources.firstOrNull;
    // A chat unless a workspace is chosen, where this build can make a chat's folder.
    final kind = _kit.scratchFolder == null ? SessionKind.workspace : SessionKind.chat;
    final draft = store.createDraft(source: source, kind: kind);
    _hint(draft);
  }

  /// The draft and source each draft's profile defaults were last worked out for.
  final Map<String, AgentSource?> _hinted = {};

  /// Puts the profile's model, effort and mode as [draft]'s defaults, where it has them.
  void _hint(ChatSession draft) {
    final source = draft.source;
    if (_hinted.containsKey(draft.id) && _hinted[draft.id] == source) return;
    _hinted[draft.id] = source;
    if (source == null) return;
    // A hosted agent is given none of the profile, so its defaults are only the person's last choices.
    store.defaultChoices(draft);
  }

  // ---- Where a new session starts: this app's own host, or a saved connection ----

  /// The saved connections' hosts opened for a new session, by connection name; one is dialled the first time a draft is aimed at it.
  final Map<String, Future<RemoteHost>> _remotes = {};
  final Map<String, RemoteHost> _remoteReady = {};

  Connection? _connection(String? name) => name == null ? null : config.connections.where((each) => each.name == name).firstOrNull;

  /// The host saved as [name], its list read and its control channel dialled. A failure is the reason in words and is not kept, so the
  /// next ask tries again.
  Future<RemoteHost> _remoteOf(String name) {
    final connection = _connection(name);
    if (connection == null) return Future.error(StateError('$name is no longer a saved connection'));
    if (connection.kind == ConnectionKind.acp) return Future.error(StateError('$name is a direct agent: it has no host to ask'));
    return _remotes.putIfAbsent(name, () async {
      try {
        final opened = await (widget.openRemote ?? _openRemote)(connection);
        if (!mounted) {
          opened.dispose();
          throw StateError('Canoa UI closed');
        }
        _remoteReady[name] = opened;
        return opened;
      } on Object {
        unawaited(_remotes.remove(name)?.then<void>((_) {}, onError: (Object _) {}));
        rethrow;
      }
    });
  }

  Future<RemoteHost> _openRemote(Connection connection) async {
    final served = await fetchServedHost(connection);
    return RemoteHost(connection, served, servedControl(served));
  }

  /// Puts away a host whose saved connection changed or went, so that the next draft aimed at it dials what is saved now.
  void _forgetRemotes() {
    for (final name in _remotes.keys.toList()) {
      final now = _connection(name);
      final held = _remoteReady[name];
      if (now != null && held != null && held.connection.address == now.address && held.connection.peer == now.peer) continue;
      _remotes.remove(name);
      _remoteReady.remove(name)?.dispose();
      _remoteAccounts.remove(name)?.dispose();
    }
  }

  /// The accounts of each saved connection's host a draft was aimed at, asked of that host's own control channel; what the host
  /// lists is the host's, never this app's canoa.cfg.
  final Map<String, AccountsController> _remoteAccounts = {};

  /// The accounts controller of the host saved as [name], made and asked once its control channel is known; null for this app's own
  /// host and for a host with no control channel.
  AccountsController? _accountsOn(String? name) => name == null ? null : _remoteAccounts[name];

  /// Reads [remote]'s accounts again: each time a draft is aimed at it, since they change under it.
  void _askAccountsOf(RemoteHost remote) {
    if (!remote.hasControl) return;
    final name = remote.connection.name;
    final held = _remoteAccounts[name] ??= AccountsController(
      host: ControlAccountsHost(remote.control),
      config: () => const CanoaConfig(),
      change: (_) async => throw StateError('$name keeps which accounts its agents offer in its own canoa.cfg'),
      hostIsHere: false,
    );
    unawaited(held.refresh());
  }

  /// The account a draft starts on when it chose none: the first its source's entry offers, then the first it can use, on that host.
  String? _defaultAccountOf(AgentSource source) {
    final entry = source.hostEntry;
    if (entry == null) return null;
    final accounts = source.host == null ? _accounts : _accountsOn(source.host);
    return accounts?.offeredBy(entry).where((each) => each.usable).firstOrNull?.name;
  }

  /// The choices the host chip offers: this computer (or the host that served this page) first, then each saved connection.
  List<HostChoice> get _hostChoices => widget.host == null
      ? const []
      : [
          HostChoice(
            name: null,
            label: widget.served == null ? 'Local' : (Uri.base.authority.isEmpty ? 'Serving host' : Uri.base.authority),
            detail: widget.served == null ? 'This computer' : 'Serves this page',
          ),
          for (final connection in config.connections)
            HostChoice(name: connection.name, label: connection.name, kind: connection.kind, detail: connection.kind.label),
        ];

  /// The sources a draft aimed at [host] can start from: what that host serves, read when it was chosen.
  List<AgentSource> _sourcesOn(String host) {
    final connection = _connection(host);
    if (connection == null) return const [];
    if (connection.kind == ConnectionKind.acp) return [AgentSource.hosted(connection.name, host: connection.name)];
    final remote = _remoteReady[host];
    if (remote == null) return const [];
    return [
      for (final entry in remote.agents)
        AgentSource.hosted(entry.name, harness: entry.harness, title: entry.title, host: host),
    ];
  }

  /// Aims [draft] at the saved connection [name], or at this app's own host for null: what it offers, its agents and recent
  /// folders, is read again from there.
  void _chooseHost(ChatSession draft, String? name) {
    store.updateDraft(draft, host: name, localHost: name == null);
    if (name == null) {
      final sources = _sources;
      final back = sources.contains(_lastSource)
          ? _lastSource
          : sources.where((source) => _checks[source]?.ready ?? false).firstOrNull ?? sources.firstOrNull;
      if (back != null) store.updateDraft(draft, source: back);
      return;
    }
    unawaited(_enterRemote(draft, name));
  }

  Future<void> _enterRemote(ChatSession draft, String name) async {
    final connection = _connection(name);
    if (connection == null) return;
    try {
      if (connection.kind != ConnectionKind.acp) await _remoteOf(name);
    } on Object catch (error) {
      if (mounted && draft.host == name) {
        _messenger.currentState?.showSnackBar(SnackBar(content: Text('Could not read $name: ${reasonOf(error)}')));
      }
      return;
    }
    if (!mounted || draft.host != name || !draft.isDraft) return;
    final remote = _remoteReady[name];
    if (remote != null) _askAccountsOf(remote);
    final offered = _sourcesOn(name);
    final kept = offered.where((source) => source == draft.source).firstOrNull;
    final chosen = kept ?? offered.firstOrNull;
    if (chosen != null && chosen != draft.source) store.updateDraft(draft, source: chosen);
    setState(() {});
  }

  Future<void> _addHost() async {
    final above = _navigator.currentContext;
    if (above == null || !above.mounted) return;
    _hosts.adopt(config);
    await showAddConnectionDialog(above, onAdd: _hosts.add, nearby: _hosts.nearby);
  }

  /// Asks which folder [session] works in, through the folder listing of the host it starts on.
  Future<String?> _browseFolder(ChatSession session) async {
    final above = _navigator.currentContext;
    if (above == null || !above.mounted) return null;
    final name = session.host;
    if (name == null) {
      if (widget.pickFolder != null) return widget.pickFolder!();
      final files = widget.served?.files ?? (_viaHost ? _hostFiles : null);
      final folders = _hostFolders;
      if (files == null || folders == null) return canRunLocalAgents ? getDirectoryPath() : null;
      return pickHostFolder(
        above,
        (path) => files.listFolders(path: path),
        start: _lastFolders[null],
        recent: _recentFoldersOn(null),
        check: (path) => workspaceProblem(host: 'this computer', os: null, folders: folders, files: files, path: path),
      );
    }
    final connection = _connection(name);
    if (connection == null) return null;
    if (connection.kind == ConnectionKind.acp) return _typePath(above, connection);
    final RemoteHost remote;
    try {
      remote = await _remoteOf(name);
    } on Object catch (error) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('Could not read $name: ${reasonOf(error)}')));
      return null;
    }
    if (!above.mounted) return null;
    if (!remote.hasControl) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('$name listens for no control channel, so its workspaces cannot be browsed.')));
      return null;
    }
    return pickHostFolder(
      above,
      (path) async {
        if (path != null && !pathFitsSystem(remote.os, path)) throw StateError('$path is not a path on $name, which runs ${remote.os}.');
        return remote.files.listFolders(path: path);
      },
      start: _lastFolders[name],
      recent: _recentFoldersOn(name),
      check: remote.problemWith,
      host: name,
    );
  }

  /// A direct agent has no folder listing: the path is taken as typed.
  Future<String?> _typePath(BuildContext above, Connection connection) {
    final typed = TextEditingController(text: _lastFolders[connection.name] ?? '');
    return showSelectableDialog<String>(
      context: above,
      builder: (context) => AlertDialog(
        title: Text('Workspace on ${connection.name}'),
        content: TextField(
          key: const Key('typed-workspace'),
          controller: typed,
          autofocus: true,
          decoration: const InputDecoration(hintText: 'An absolute path on that machine'),
          onSubmitted: (value) => Navigator.pop(context, value.trim().isEmpty ? null : value.trim()),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
          FilledButton(
            key: const Key('typed-workspace-use'),
            onPressed: () => Navigator.pop(context, typed.text.trim().isEmpty ? null : typed.text.trim()),
            child: const Text('Use'),
          ),
        ],
      ),
    ).whenComplete(typed.dispose);
  }

  /// Starts a draft aimed at a saved connection, on its first message: the workspace is checked by that host (a direct agent's is
  /// taken as typed), a chat's folder is made by that host, a worktree is made there, and the agent is reached at its own address.
  Future<void> _startOnConnection(ChatSession draft, String name) async {
    final source = draft.source;
    final connection = _connection(name);
    if (source == null) throw StateError('Choose its agent first');
    if (connection == null) throw StateError('$name is no longer a saved connection');
    final direct = connection.kind == ConnectionKind.acp;
    draft.account ??= _defaultAccountOf(source);
    final remote = direct ? null : await _remoteOf(name);
    final String url;
    if (remote == null) {
      url = connection.address;
    } else {
      url = remote.agents.where((entry) => entry.name == source.label).firstOrNull?.url ??
          (throw StateError('$name does not serve ${source.label} now'));
    }
    final noFolder = draft.cwd == null || draft.cwd!.isEmpty;
    if (draft.kind != SessionKind.chat && noFolder && remote != null) {
      draft
        ..kind = SessionKind.chat
        ..wantsWorktree = false;
    }
    final scratch = draft.kind == SessionKind.chat;
    if (scratch && remote == null) throw StateError('$name takes a workspace you type: choose one first');
    if (!scratch && noFolder) throw StateError('Choose its workspace first');
    var folder = draft.cwd ?? '';
    final request = scratch ? _scratchRequest(draft) : null;
    var madeFolder = false;
    if (scratch) {
      if (request == null) throw StateError('A chat cannot be started here; choose a workspace instead');
      try {
        final made = await remote!.folders.makeScratch(request);
        folder = made.path;
        madeFolder = made.made;
      } on Object catch (error) {
        throw StateError("$name cannot make the chat's workspace: ${reasonOf(error)}");
      }
    } else if (remote != null) {
      final problem = await remote.problemWith(folder);
      if (problem != null) throw StateError(problem);
    }
    final worktree = !scratch && draft.wantsWorktree && remote != null && await remote.folders.isRepository(folder);
    final method = PeerMethod.parse(connection.method);
    final endpoint = Uri.parse(url);
    final pinned = connection.peer.toLowerCase();
    final place = widget.host?.peerId?.toLowerCase() == pinned ? SessionPlace.host : SessionPlace.peer;
    final starting = _Starting();
    _inFlight.add(starting);
    Worktrees? worktrees;
    SessionWorktree? made;
    VerifiedAgent? client;
    try {
      worktrees = worktree ? _worktrees(place, endpoint: url, peerId: pinned, method: method.id) : null;
      if (worktree && worktrees == null) throw StateError('Canoa cannot ask $name for a worktree');
      made = worktrees == null ? null : await worktrees.create(folder, worktreeBranch(source.label));
      starting
        ..worktrees = worktrees
        ..made = made;
      if (_quitting) throw StateError('Canoa is quitting');
      final where = made?.path ?? folder;
      final overTransport = direct && connection.transport == ConnectionTransport.webtransport;
      client = await (widget.connectEndpoint ?? connectBrowserAcp)(
        chatEndpoints(
          webSocket: overTransport ? '' : url,
          webTransport: overTransport ? url : '',
          preferWebTransport: config.preferWebTransport,
        ),
        pinnedPeerId: pinned,
        method: method,
        timeout: Duration(seconds: config.connectTimeout),
        decider: store.askPermission,
        elicitor: store.askQuestion,
        secrets: store,
        // The chosen account and loadout travel as they do to this computer's own entries; the host reads them by name.
        initializeMeta: _startMeta(draft),
      );
      starting.client = client;
      if (_quitting) throw StateError('Canoa is quitting');
      final opened = await client.openSession(cwd: where);
      if (!mounted || _quitting) throw StateError('Canoa UI closed');
      store.bindDraft(draft, client, opened, cwd: where, worktree: made, place: place, endpoint: endpoint, method: method.id);
      _ownedClients.add(client);
    } catch (_) {
      await client?.close();
      if (made != null) await _unmake(worktrees!, made);
      if (madeFolder && request != null) await remote!.folders.removeScratch(request);
      rethrow;
    } finally {
      _inFlight.remove(starting);
    }
    await store.applyHostProfile(draft, expected: _hasProfiles);
  }

  /// Starts [draft]'s agent on its first message: the worktree is made, the profile prepared and shown where it has something
  /// to approve, the agent started and its session opened, and the draft bound to it. Anything made is taken back when it
  /// fails, and the draft stays as it was.
  Future<void> _startDraft(ChatSession draft) async {
    final source = draft.source;
    if (source == null) throw StateError('Choose where its agent comes from first');
    if (draft.host case final name?) return _startOnConnection(draft, name);
    await _stopForTrouble(source);
    final check = _checks[source] ?? await timed<SourceCheck>('lookup', () => _sourceChecks.check(source));
    if (!check.ready) {
      throw StateError(check.describe(source.label)!);
    }
    // A chat works in its scratch folder, which is made now and not before; a workspace session needs the folder chosen.
    // A session with no workspace chosen is a chat, where this build can make a chat's folder.
    final noFolder = draft.cwd == null || draft.cwd!.isEmpty;
    if (draft.kind != SessionKind.chat && noFolder && _kit.scratchFolder != null) {
      draft
        ..kind = SessionKind.chat
        ..wantsWorktree = false;
    }
    final scratch = draft.kind == SessionKind.chat;
    if (!scratch && noFolder) throw StateError('Choose its workspace first');
    // A served page's host makes the chat's folder itself, from the stand-in the kit names.
    final chosen = scratch ? _kit.scratchFolder?.call(draft) : draft.cwd;
    if (chosen == null) throw StateError('A chat cannot be started here; choose a workspace instead');
    var folder = chosen;
    var madeFolder = false;
    var hostMade = false;
    // The served page's own chat: the host makes and names the folder in its answer to `session/new`.
    final hostMakes = scratch && widget.served != null;
    // A desktop whose host runs asks it to make the folder the template names, as the host does for a served page.
    final scratchRequest = scratch && !hostMakes && _viaHost ? _scratchRequest(draft) : null;
    if (scratch && !hostMakes) {
      try {
        final request = scratchRequest;
        if (request == null) throw StateError('Its workspace is made by the host, which is not here');
        final made = await _hostFolders!.makeScratch(request);
        folder = made.path;
        hostMade = true;
        madeFolder = made.made;
      } on Object catch (error) {
        throw StateError('Cannot make its workspace $folder: ${reasonOf(error)}');
      }
    }
    // Worktree shows, and applies, only for a folder in a git repository.
    final worktree = !scratch &&
        draft.wantsWorktree &&
        (widget.isGitRepository != null
            ? widget.isGitRepository!(folder)
            : _viaHost && await _hostFolders!.isRepository(folder));
    final starting = _Starting();
    _inFlight.add(starting);
    try {
      await _startHosted(draft, source.hostEntry!, folder, worktree, starting, hostMakes: hostMakes);
    } catch (_) {
      if (madeFolder && hostMade && scratchRequest != null) await _hostFolders!.removeScratch(scratchRequest);
      rethrow;
    } finally {
      _inFlight.remove(starting);
    }
  }

  /// A source the host says cannot start, the program missing or nobody signed in, is not started: the action that puts it right
  /// opens instead, and the draft stays as it is. The host is asked once more first, since it may have been put right since.
  Future<void> _stopForTrouble(AgentSource source) async {
    var trouble = sourceTrouble(source, store.choicesOf(source), accounts: _accounts);
    if (trouble == null || !trouble.blocksStart) return;
    await store.askChoicesAgain(source);
    trouble = sourceTrouble(source, store.choicesOf(source), accounts: _accounts);
    if (trouble == null || !trouble.blocksStart) return;
    final context = _navigator.currentContext;
    if (mounted && context != null && context.mounted && canAct(trouble, _kit)) unawaited(actOnTrouble(context, store, _kit, trouble));
    throw StateError('${trouble.message}.');
  }

  /// Starts under way, whose worktree and agent quitting takes back.
  final Set<_Starting> _inFlight = {};
  bool _quitting = false;

  /// Quitting during a start: what it made so far goes, and it goes no further.
  Future<void> _endStarts() async {
    _quitting = true;
    for (final starting in [..._inFlight]) {
      await starting.client?.close();
      if (starting.made case final made?) await _unmake(starting.worktrees!, made);
    }
  }

  /// [chosen] in canoa.cfg's order, without any that is no longer installed.
  List<String> _overlaysNamed(Set<String> chosen) => [for (final name in config.overlays.keys) if (chosen.contains(name)) name];

  /// What `initialize` carries to a host to say how long this session's prompt cache should live.
  Map<String, Object?> _cacheMeta(({CacheLifetime lifetime, bool chosen}) asked) =>
      {cacheLifetimeMeta: cacheLifetimeRequest(asked.lifetime, chosen: asked.chosen)};

  /// What `initialize` carries to an entry of the host that served this page: the cache lifetime and, only when the person chose
  /// overlays for this session, which they are. Sent unasked, a name from canoa.cfg's own `[overlays]` would narrow an operator's
  /// profile of the entry by names that mean nothing there.
  Map<String, Object?> _startMeta(ChatSession draft) {
    final cache = {..._cacheMeta(store.cacheLifetimeOf(draft)), ..._accountMeta(draft.account)};
    final asked = draft.overlays;
    final withLoadout = {...cache, ..._loadoutMeta(draft), ..._presetMeta(draft), clientMeta: ?_clientKind};
    if (widget.served == null || asked == null || config.overlays.isEmpty) return withLoadout;
    final chosen = asked.where(config.overlays.containsKey).toSet();
    return {...withLoadout, overlaysMeta: _overlaysNamed(chosen)};
  }

  /// The profile [draft] was given, when canoa.cfg still holds it.
  Map<String, Object?> _presetMeta(ChatSession draft) {
    final name = draft.preset;
    return name != null && config.presets.containsKey(name) ? {presetMeta: name} : const {};
  }

  /// Writes the profile [name] into canoa.cfg as [preset], in place when it is there; false when the file could not be written.
  Future<bool> _savePreset(String name, Preset preset) async {
    final changed = config.withPresets({...config.presets, name: preset});
    try {
      await _write(changed);
    } on Object {
      return false;
    }
    if (!mounted) return true;
    setState(() => config = changed);
    _settingsKey.currentState?.controller.adopt(changed);
    return true;
  }

  /// Shows what a new session starts with, layer by layer: the profile and what was changed from it make the session's own layer.
  void _showSetup(ChatSession session) {
    final above = _navigator.currentContext;
    if (above == null) return;
    final chosen = config.presets[session.preset];
    final options = store.optionsOf(session);
    List<String> offered(OptionRole role) => [
      for (final choice in optionIn(options, role)?.choices ?? const <Choice>[]) choice.value,
    ];
    unawaited(showSelectableDialog<void>(
      context: above,
      builder: (_) => EffectiveSetupDialog(
        setup: EffectiveSetup(
          config: config,
          agent: session.source?.label ?? session.agent,
          session: store.presetOf(session, context: chosen?.context).sessionLayer,
          home: _home,
          workingDirectory: session.cwd,
          profile: chosen == null ? null : session.preset,
          offered: {
            'model': offered(OptionRole.model),
            'effort': offered(OptionRole.effort),
            'mode': [for (final entry in modeEntries(modeOptionIn(options))) entry.value],
          },
          unsupported: session.presetUnsupported,
          overlayNeeds: _control == null ? null : controlOverlayNeeds(_control),
          onInstallRuntime: _programs == null
              ? null
              : (providers) async {
                  final runtimes = _extensionsHeld?.runtimes;
                  if (runtimes == null) return;
                  for (final provider in providers) {
                    if (!above.mounted || !await installRuntime(above, runtimes, provider)) break;
                  }
                },
        ),
      ),
    ));
  }

  /// The loadout [draft] was given, when canoa.cfg still holds it; a session that names none starts with `[defaults] loadout`.
  Map<String, Object?> _loadoutMeta(ChatSession draft) {
    final name = draft.loadout;
    return name != null && config.loadouts.containsKey(name) ? {loadoutMeta: name} : const {};
  }

  Future<void> _startHosted(ChatSession draft, String name, String folder, bool worktree, _Starting starting,
      {bool hostMakes = false}) async {
    final host = widget.host;
    final listening = host?.listening.where((listening) => listening.name == name).firstOrNull;
    if (host == null || listening == null) throw StateError('canoa is not serving $name now: open Hosting to see why');
    // Only a desktop holds its host in this process; a served page reaches every entry, account or not, over its host's endpoint.
    if (widget.served == null && (listening.local || draft.account != null || (_accounts?.offeredBy(name).isNotEmpty ?? false))) {
      draft.account ??= _accounts?.offeredBy(name).where((each) => each.usable).firstOrNull?.name;
      return _startViaLink(draft, name, folder, worktree, starting);
    }
    draft.account ??= _accounts?.offeredBy(name).where((each) => each.usable).firstOrNull?.name;
    final method = PeerMethod.values.where((method) => method.id == (listening.method ?? host.peerMethod)).firstOrNull;
    if (method == null) throw StateError('canoa did not say which method it proves by');
    final endpoint = Uri.parse(listening.url);
    final worktrees = worktree
        ? _worktrees(SessionPlace.host, endpoint: listening.url, peerId: listening.peer, method: method.id)
        : null;
    if (worktree && worktrees == null) throw StateError('Canoa cannot ask canoa for a worktree');
    final made = worktrees == null ? null : await worktrees.create(folder, worktreeBranch(name));
    starting
      ..worktrees = worktrees
      ..made = made;
    if (_quitting) throw StateError('Canoa is quitting');
    final where = made?.path ?? folder;
    VerifiedAgent? client;
    try {
      client = isPipeUrl(listening.url)
          ? await connectBrowserAcpOverPipe(
              listening.url, pinnedPeerId: listening.peer, method: method,
              timeout: Duration(seconds: config.connectTimeout),
              decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
              initializeMeta: _startMeta(draft),
            )
          : await (widget.connectEndpoint ?? connectBrowserAcp)(
              chatEndpoints(webSocket: listening.url, webTransport: '', preferWebTransport: config.preferWebTransport),
              pinnedPeerId: listening.peer, method: method,
              timeout: Duration(seconds: config.connectTimeout),
              decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
              initializeMeta: _startMeta(draft),
            );
      starting.client = client;
      if (_quitting) throw StateError('Canoa is quitting');
      final opened = await client.openSession(cwd: where, setup: hostMakes ? siteSetup(servedSite) : AcpSessionSetup.none);
      final placed = hostMakes ? madeFolder(opened.raw) : null;
      if (hostMakes && placed == null) throw StateError('The host did not say which workspace it made for this chat');
      if (!mounted || _quitting) throw StateError('Canoa UI closed');
      if (placed != null) {
        draft
          ..site = servedSite
          ..workspaceId = placed.id;
      }
      store.bindDraft(draft, client, opened, cwd: placed?.folder ?? where, worktree: made, place: SessionPlace.host, endpoint: endpoint,
          method: method.id);
      _ownedClients.add(client);
    } catch (_) {
      await client?.close();
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
    await store.applyHostProfile(draft, expected: _hasProfiles);
  }

  /// Answers the host's question, before a session of [agent] starts, of which MCP servers and which overlays' hooks it may
  /// start: the person's choice for this session, kept in canoa.cfg when they ask. Nothing is allowed when there is no window.
  ProfileApprover _approver(String agent) => (ask) async {
    final above = _navigator.currentContext;
    if (above == null || (ask.servers.isEmpty && ask.hooks.isEmpty)) return noApproval;
    final choice = await showSelectableDialog<ApprovalChoice>(
      context: above,
      builder: (_) => ProfileApprovalDialog(ask: ask, agent: agent),
    );
    if (choice == null) return noApproval;
    if (choice.remember && (choice.allow.isNotEmpty || choice.hooks.isNotEmpty) && mounted) {
      final kept = <String, Object>{
        ...?config.agentMcp[agent],
        for (final server in ask.servers)
          if (choice.allow.contains(server.name)) server.name: server.digest,
      };
      final keptHooks = <String, String>{
        ...?config.agentHooks[agent],
        for (final hook in ask.hooks)
          if (choice.hooks.contains(hook.overlay)) hook.overlay: hook.digest,
      };
      try {
        final changed = config.withAgentMcp(agent, kept).withAgentHooks(agent, keptHooks);
        await _write(changed);
        if (mounted) setState(() => config = changed);
      } on Object catch (error) {
        debugPrint('Canoa could not keep the choices for $agent: $error');
      }
    }
    return (servers: choice.allow, hooks: choice.hooks);
  };

  /// The agent behind the host's own source [name], reached through the in-process link and ready to open a session.
  Future<VerifiedAgent> _linkAgent(String name, Map<String, Object?> initializeMeta) async {
    final host = widget.host;
    if (host == null) throw StateError('This build has no host of its own to start $name');
    final HostLink link;
    try {
      link = host.openLink(name);
    } on HostCommandFailed catch (error) {
      throw StateError('$error');
    }
    // The host in this process binds a secret prompt by the secret it shares with this app, which only this link has.
    LinkSecretIdentity? linkSecrets;
    final secret = link is HostLinkSecret ? (link as HostLinkSecret).takeLinkSecret() : null;
    if (secret != null) {
      try {
        // Only what is derived from the secret is kept; the bytes are zeroed.
        linkSecrets = await linkSecretIdentity(secret);
      } on Object {
        // No secret prompts on this link: the host is offered nothing.
      } finally {
        secret.fillRange(0, secret.length, 0);
      }
    }
    final client = VerifiedAgent.fromChild(
      label: name, messages: link.messages, send: link.send, close: link.close,
      decider: store.askPermission, elicitor: store.askQuestion, initializeMeta: initializeMeta,
      approver: _approver(name), secrets: store, linkSecrets: linkSecrets,
    );
    try {
      await client.initialize();
      return client;
    } on Object {
      await client.close();
      rethrow;
    }
  }

  /// Starts a session on one of the host's own sources, over the in-process link: no address and no proof, since the host
  /// is in this process.
  Future<void> _startViaLink(ChatSession draft, String name, String folder, bool worktree, _Starting starting) async {
    final worktrees = worktree ? (widget.worktrees ?? _localWorktrees)() : null;
    if (worktree && worktrees == null) throw StateError('This build cannot make a worktree');
    final made = worktrees == null ? null : await timed('worktree', () => worktrees.create(folder, worktreeBranch(name)));
    starting
      ..worktrees = worktrees
      ..made = made;
    if (_quitting) throw StateError('Canoa is quitting');
    final where = made?.path ?? folder;
    VerifiedAgent? client;
    try {
      // The overlays it starts with are Loadout's defaults unless the person chose; what it starts with is kept with it.
      final named = _loadoutMeta(draft)[loadoutMeta] as String?;
      final chosen = draft.overlays?.where(config.overlays.containsKey).toSet() ??
          (named == null ? config.defaultOverlays : config.resolveLoadout(named)!.overlays).toSet();
      if (config.overlays.isNotEmpty) draft.overlays = chosen;
      client = await timed<VerifiedAgent>('link_and_initialize', () => _linkAgent(name, {
        ..._cacheMeta(store.cacheLifetimeOf(draft)),
        ..._accountMeta(draft.account),
        ..._loadoutMeta(draft),
        ..._presetMeta(draft),
        if (config.overlays.isNotEmpty) overlaysMeta: _overlaysNamed(chosen),
      }));
      starting.client = client;
      if (_quitting) throw StateError('Canoa is quitting');
      final opened = await client.openSession(cwd: where);
      if (!mounted || _quitting) throw StateError('Canoa UI closed');
      store.bindDraft(draft, client, opened, cwd: where, worktree: made);
      _ownedClients.add(client);
    } catch (_) {
      await client?.close();
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
    await store.applyHostProfile(draft, expected: _hasProfiles);
  }

  /// Takes part in a session the host keeps running: this app's connection to that entry is made as for any session of the entry,
  /// and attaches to the running one, replayed what was said. The ledger notes it as it does any session, with where this app is
  /// in its log.
  Future<void> _attachRunning(RunningSession running) async {
    for (final session in store.sessions) {
      if (session.remoteSessionId == running.sessionId &&
          session.agent == running.entry &&
          session.status != SessionStatus.disconnected) {
        store.select(session.id);
        return;
      }
    }
    final SessionPlace place;
    final String peerId;
    String? endpoint, method;
    if (widget.served != null) {
      final listening = widget.host?.listening.where((each) => each.name == running.entry).firstOrNull;
      if (listening == null) throw StateError('canoa is not serving ${running.entry} now: open Hosting to see why');
      place = SessionPlace.host;
      peerId = listening.peer;
      endpoint = listening.url;
      method = listening.method ?? widget.host?.peerMethod;
    } else {
      place = SessionPlace.local;
      peerId = 'local:${running.entry}';
    }
    final ledger = store.ledger;
    final entry = ledger?.find(peerId, running.sessionId) ??
        ledger?.record(
          peerId: peerId, sessionId: running.sessionId, entry: running.entry, place: place, reopenBy: 'session/resume',
          cwd: running.cwd, title: running.title, endpoint: endpoint, method: method,
        ) ??
        LedgerEntry(
          peerId: peerId, sessionId: running.sessionId, entry: running.entry, place: place, created: DateTime.now(),
          lastUsed: DateTime.now(), harness: running.entry, reopenBy: 'session/resume', cwd: running.cwd, title: running.title,
          endpoint: endpoint, method: method,
        );
    await _reopen(entry, attach: true);
  }

  /// Opens a session from the list again: its agent is started or reached again, then asked to open it. With [attach] it is a
  /// session the host keeps running, joined where this app left it.
  Future<void> _reopen(LedgerEntry entry, {bool attach = false}) async {
    for (final session in store.sessions) {
      if (session.peerId == entry.peerId && session.remoteSessionId == entry.sessionId) {
        store.select(session.id);
        return;
      }
    }
    final why = entry.whyNotReopened;
    if (why != null) throw StateError(why);
    await _findWorktree(entry);
    final VerifiedAgent client = await _agentFor(
      place: entry.place,
      entryName: entry.entry,
      address: entry.endpoint,
      fallback: entry.fallback,
      methodId: entry.method,
      peerId: entry.peerId,
      cacheLifetime: entry.cacheLifetime,
      overlays: entry.overlays,
      account: entry.account,
    );
    try {
      // A host this app saved shows under the name it was saved with, not its address.
      await store.reopen(entry, client, expected: _hasProfiles, attach: attach,
          host: entry.place == SessionPlace.peer ? savedNameOfPeer(config.connections, entry.peerId) : null);
      _ownedClients.add(client);
    } catch (_) {
      await client.close();
      rethrow;
    }
  }

  /// Starts or reaches the agent a session of [entryName] runs on, for [account], over the way it was begun: this computer's own
  /// link, or the peer at [endpoint]. Opening a session again and moving a running one to another account both begin here.
  Future<VerifiedAgent> _agentFor({
    required SessionPlace place,
    required String entryName,
    String? address,
    String? fallback,
    String? methodId,
    required String? peerId,
    String? cacheLifetime,
    List<String>? overlays,
    String? account,
  }) async {
    final VerifiedAgent client;
    if (widget.served != null && place == SessionPlace.local) {
      // A page holds no host in its process: a session begun on the host's own computer is reached through its endpoint.
      final listening = widget.host?.listening.where((each) => each.name == entryName).firstOrNull;
      if (listening == null) throw StateError('canoa is not serving $entryName now: open Hosting to see why');
      return _agentFor(
        place: SessionPlace.host,
        entryName: entryName,
        address: listening.url,
        methodId: listening.method ?? widget.host?.peerMethod,
        peerId: listening.peer,
        cacheLifetime: cacheLifetime,
        overlays: overlays,
        account: account,
      );
    }
    switch (place) {
      case SessionPlace.local when _hostedSources.any((source) => source.local && source.label == entryName) ||
          account != null ||
          (_accounts?.offeredBy(entryName).isNotEmpty ?? false):
        client = await _linkAgent(entryName, {
          ..._cacheMeta((
            lifetime: CacheLifetime.named(cacheLifetime) ?? CacheLifetime.chat,
            chosen: cacheLifetime != null,
          )),
          if (overlays case final started?) overlaysMeta: _overlaysNamed(started.toSet()),
          ..._accountMeta(account),
        });
      case SessionPlace.local:
        throw StateError('$entryName is not one of the sources of this computer now');
      case SessionPlace.host || SessionPlace.peer:
        final endpoint = Uri.tryParse(address ?? '');
        final method = PeerMethod.values.where((each) => each.id == methodId).firstOrNull;
        if (endpoint == null || method == null) throw StateError('Canoa did not note how to reach that peer');
        final overWebTransport = endpoint.scheme == 'https';
        final cacheMeta = {
          ..._cacheMeta((
            lifetime: CacheLifetime.named(cacheLifetime) ?? CacheLifetime.chat,
            chosen: cacheLifetime != null,
          )),
          if (widget.served != null)
            if (overlays case final started?) overlaysMeta: _overlaysNamed(started.toSet()),
        };
        client = isPipeUrl(endpoint.toString())
            ? await connectBrowserAcpOverPipe(
                endpoint.toString(), pinnedPeerId: peerId!, method: method,
                timeout: Duration(seconds: config.connectTimeout),
                decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
                initializeMeta: {...cacheMeta, ..._accountMeta(account)},
              )
            : await (widget.connectEndpoint ?? connectBrowserAcp)(
                chatEndpoints(
                  webSocket: overWebTransport ? (fallback ?? '') : endpoint.toString(),
                  webTransport: overWebTransport ? endpoint.toString() : '',
                  preferWebTransport: config.preferWebTransport,
                ),
                pinnedPeerId: peerId!, method: method,
                timeout: Duration(seconds: config.connectTimeout),
                decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
                initializeMeta: {...cacheMeta, ..._accountMeta(account)},
              );
      case SessionPlace.browser:
        throw StateError('Reopen it from Earlier conversations');
    }
    return client;
  }

  /// The host's entries come and go as it starts and ends, and a hosted source can run only while it serves one.
  List<AgentSource>? _serving;
  void _hostChanged() {
    // The host was started anew: which accounts it has is asked again.
    if (widget.host?.listening.isNotEmpty ?? false) {
      if (!vendorLooks.answered) unawaited(_loadLooks());
      if (!providerCatalog.answered) unawaited(_loadProviders());
      unawaited(_grants?.refresh());
      unawaited(_accounts?.refresh());
      unawaited(_running?.refresh());
      if (!_mode.chatOnly) unawaited(_refreshOverlays());
    }
    final serving = _hostedSources;
    if (_serving != null && _serving!.length == serving.length && _serving!.every(serving.contains)) return;
    _serving = serving;
    store.resolveKeptSources((label) =>
        _sources.where((source) => source.label == label).firstOrNull);
    _land();
    if (_checked) {
      unawaited(_checkSources());
    } else if (mounted) {
      setState(() {});
    }
  }

  /// Ends every agent this app started, and the host with the agents it started.
  Future<void> _endAgents() async {
    await _endStarts();
    await store.ledger?.flush();
    await store.drafts?.flush();
    final ending = [for (final client in _ownedClients) client.close(), ?widget.host?.stop()];
    _ownedClients.clear();
    // Longer than the host has to end its agents.
    await Future.wait(ending).timeout(const Duration(seconds: 10), onTimeout: () => const []);
  }

  Future<void> _connect(ConnectTarget target) async {
    final endpoints = chatEndpoints(
      webSocket: target.webSocket,
      webTransport: target.webTransport,
      preferWebTransport: config.preferWebTransport,
    );
    final ws = endpoints.webSocket;
    final wt = target.webTransport.isEmpty ? null : Uri.parse(target.webTransport);
    final primary = wt ?? ws!;
    final pinnedPeerId = target.peerId.toLowerCase();
    final place = widget.host?.peerId?.toLowerCase() == pinnedPeerId ? SessionPlace.host : SessionPlace.peer;
    // The host makes it inside its working roots, before any agent starts, and says where.
    final worktrees = target.worktree
        ? _worktrees(place, endpoint: primary.toString(), fallback: wt == null ? null : ws?.toString(),
            peerId: pinnedPeerId, method: target.method.id)
        : null;
    if (target.worktree && worktrees == null) throw StateError('Canoa cannot ask this agent\'s host for a worktree');
    final made = worktrees == null
        ? null
        : await worktrees.create(target.cwd, worktreeBranch(target.title.isEmpty ? 'remote agent' : target.title));
    final cwd = made?.path ?? target.cwd;
    final VerifiedAgent client;
    try {
      client = await connectBrowserAcp(
        endpoints, pinnedPeerId: pinnedPeerId, method: target.method,
        timeout: Duration(seconds: config.connectTimeout),
        decider: store.askPermission, elicitor: store.askQuestion, secrets: store,
      );
    } catch (_) {
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
    try {
      final opened = await client.openSession(cwd: cwd);
      final remoteId = opened.id;
      if (!mounted) throw StateError('Canoa UI closed');
      final local = store.addRemoteSession(
        title: target.title.isEmpty ? 'Remote agent' : target.title,
        agent: pinnedPeerId.substring(0, 22),
        peerId: pinnedPeerId,
        remoteSessionId: remoteId,
        endpoint: primary,
        websocketFallback: wt == null ? null : ws,
        place: place,
        method: target.method.id,
        cwd: cwd,
        worktree: made,
        agentNames: target.title.isEmpty,
      );
      store.applyOpened(local, opened);
      store.attachVerifiedClient(local.id, client);
      _ownedClients.add(client);
      await store.applyHostProfile(local, expected: _hasProfiles);
    } catch (_) {
      await client.close();
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
  }

  @override
  void initState() {
    super.initState();
    ownsStore = widget.store == null;
    config = widget.config;
    if (_control case final control? when widget.served == null) _desktopConfig = HostConfigStore.ofDesktop(HostFiles(control), config);
    widget.settingsSignal?.addListener(_requestSettings);
    widget.hostingSignal?.addListener(_requestHosting);
    store = widget.store ??
        SessionStore(recorder: updateRecorderFromEnvironment(), ledger: widget.ledger, rules: widget.rules, defaultMode: config.defaultMode, bypassOnceOnly: config.allowOnceOnly,
            drafts: widget.drafts, embedLimit: config.embedLimit);
    store.queueDelivery = config.sessions.queueDelivery;
    store.starter = _startDraft;
    store.choicesLoader = _askChoices;
    store.accountChoicesLoader = _askChoicesAs;
    store.defaultAccountOf = _defaultAccountOf;
    store.gitReader = _gitReaderFor;
    store.pathsOnHost ??= _pathsOnHost;
    if (_control case final control?) widget.rules?.useHost(HostPaths(control), up: () => _hostUp);
    store.addListener(_noteLast);
    widget.host?.addListener(_hostChanged);
    _reconnects = _control?.reconnected.listen((_) => unawaited(_hostCameBack()));
    scheduleMicrotask(() {
      unawaited(_loadLooks());
      unawaited(_loadProviders());
      unawaited(_grants?.refresh());
      unawaited(_accounts?.refresh());
      _running?.keepFresh();
    });
    if (store.secretIdentity == null) unawaited(_loadSecretIdentity());
    unawaited(store.restoreDrafts((label) =>
        _sources.where((source) => source.label == label).firstOrNull).whenComplete(() {
      _restored = true;
      _land();
    }));
    // Closing a window to the tray keeps its agents. Quitting does not, and `dispose` does not run when the window is destroyed.
    widget.shell?.beforeQuit = _endAgents;
    _conversation.close = _closeSession;
    _conversation
      ..moveTargets = _moveTargets
      ..move = _moveTo
      ..canListImported = _canListImported
      ..imported = _importedFor
      ..showImported = _showImported;
    _conversation.popout = _popout;
    _attention = store.attention.listen((needs) => unawaited(_notify(needs.$1, needs.$2)));
    unawaited(widget.notifier?.initialize(_openFromNotification));
    store.addListener(_showWaiting);
    final kept = _layout.of(ShellLayout.conversations).item;
    if (kept != null && store.selectedId != kept && store.sessions.any((session) => session.id == kept)) store.select(kept);
    _keepSelection();
    store.addListener(_keepSelection);
    // The welcome page, which a window with no session shows, lists each source with what it needs.
    scheduleMicrotask(_noteLast);
    if (widget.served?.problem ?? widget.served?.configProblem case final problem?) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        _messenger.currentState?.showSnackBar(SnackBar(duration: const Duration(seconds: 30), content: Text(problem)));
      });
    }
    unawaited(_claimContext());
    _localHost = widget.localHost ?? platformLocalHostBridge();
    final bridge = widget.tabBridge;
    _tabs = bridge == null ? TabsComposerExtension.platform(store) : TabsComposerExtension(TabShares(bridge), store: store);
    final front = _tabs?.tabs;
    _relations = widget.relations ??
        (_mode.relation != null
            ? ManualRelation(_mode.relation)
            : front != null
                ? FrontTabRelation(front)
                // The chat-only app always has a source, which says nothing is in front until a client tells it.
                : _mode.chatOnly ? ManualRelation() : null);
    _ownsRelations = widget.relations == null && _relations != null;
    final manual = _relations;
    if (_mode.chatOnly && _ownsRelations && manual is ManualRelation) {
      // A pane that learns what is open later (a pinned Outlook pane moving between messages) says so, and the list follows with no
      // reload; it is told the app is listening only once the first message can be heard.
      _pane = widget.paneLink ?? PaneLink.ofPage();
      _paneRelations = _pane?.relations.listen((relation) => manual.current = relation);
      _pane?.ready();
    }
    _contextTimer = Timer.periodic(
      const Duration(seconds: 2), (_) => unawaited(_claimContext()),
    );
  }

  /// Writes down which conversation is shown, for the next run.
  void _keepSelection() => _layout.setItem(ShellLayout.conversations, store.selectedId);

  /// The identity secret prompts are bound with, once this page's own key can be used: until then a host is offered nothing.
  Future<void> _loadSecretIdentity() async {
    try {
      final identity = await browserSecretIdentity();
      if (mounted && store.secretIdentity == null) store.secretIdentity = identity;
    } catch (_) {
      // No key, no secret prompts: the connection itself reports a key that cannot be used.
    }
  }

  @override
  void didUpdateWidget(CanoaApp oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.config != oldWidget.config) config = widget.config;
    _forgetRemotes();
  }

  @override
  void dispose() {
    _running?.dispose();
    store.removeListener(_keepSelection);
    if (widget.layout == null) _layout.dispose();
    _surfaces.dispose();
    _terminalSection.dispose();
    _grouping.dispose();
    _loadoutsSelection.dispose();
    _start.dispose();
    _popout.dispose();
    unawaited(_paneRelations?.cancel());
    _pane?.dispose();
    if (_ownsRelations) (_relations as ChangeNotifier).dispose();
    _tabs?.dispose();
    _contextTimer?.cancel();
    unawaited(_attention?.cancel());
    store.removeListener(_showWaiting);
    store.removeListener(_noteLast);
    if (identical(store.starter, _startDraft)) store.starter = null;
    if (identical(store.choicesLoader, _askChoices)) store.choicesLoader = null;
    if (identical(store.accountChoicesLoader, _askChoicesAs)) store.accountChoicesLoader = null;
    if (identical(store.gitReader, _gitReaderFor)) store.gitReader = null;
    if (identical(store.pathsOnHost, _pathsOnHost)) store.pathsOnHost = null;
    widget.host?.removeListener(_hostChanged);
    unawaited(_reconnects?.cancel());
    _hostFolders?.dispose();
    _dictation.dispose();
    _reader.stop();
    _reader.dispose();
    if (widget.voiceOutputs == null) _voiceOutputs.dispose();
    for (final remote in _remoteReady.values) {
      remote.dispose();
    }
    _remoteReady.clear();
    for (final accounts in _remoteAccounts.values) {
      accounts.dispose();
    }
    _remoteAccounts.clear();
    unawaited(_control?.close());
    widget.settingsSignal?.removeListener(_requestSettings);
    widget.hostingSignal?.removeListener(_requestHosting);
    widget.shell?.beforeQuit = null;
    if (ownsStore) store.dispose();
    for (final client in _ownedClients) {
      unawaited(client.close());
    }
    super.dispose();
  }

  /// The names an `@` offers in [folder], which the host lists; none while it does not answer.
  Future<List<String>> _mentionsOf(String folder) async {
    try {
      return await _hostFolders!.mentions(folder);
    } on Object {
      return const [];
    }
  }

  /// The file or folder [relative] names inside [folder], from the host; null while it does not answer.
  Future<InsideFile?> _resolveIn(String folder, String relative) async {
    try {
      return await _hostFolders!.resolve(folder, relative);
    } on Object {
      return null;
    }
  }

  /// What the host makes [session]'s scratch folder from, null for a session made no id here.
  ScratchRequest? _scratchRequest(ChatSession session) {
    final id = session.workspaceId;
    if (id == null) return null;
    final source = session.source;
    return (session: id, source: source?.label ?? '', handler: source == null ? '' : handlerOf(source), created: session.created);
  }

  /// Asks which folder of the host's computer a new session works in, from the places the host lets this page's key browse.
  Future<String?> _pickHostFolder() async {
    final above = _navigator.currentContext;
    final served = widget.served;
    if (above == null || served == null) return null;
    return pickHostFolder(above, (path) => served.files.listFolders(path: path), start: _lastFolder);
  }

  /// Whether [session]'s agent runs on the host that served this page: a draft starts on one of its entries, and a started session
  /// is on it when it was reached at its key.
  bool _onServedHost(ChatSession session) => session.isDraft
      ? session.source != null
      : session.place == SessionPlace.host && session.peerId == widget.served?.host.peer;

  /// Keeps a file the person attached on the host's computer, where the agent reads it.
  Future<({String path, int size})> _upload(String name, List<int> bytes) async {
    final kept = await widget.served!.files.upload(name, bytes);
    return (path: kept.path, size: kept.size);
  }

  /// What the composer offers, from this app's sources, folders, files and clipboard.
  ComposerKit get _kit => ComposerKit(
    sources: _sources,
    checks: _checks,
    recentFolders: _recentFolders,
    inGitRepository: widget.isGitRepository ?? (_viaHost ? _hostFolders!.inRepository : (_) => false),
    foldersChanged: _remoteReady.isEmpty ? _hostFolders : Listenable.merge([?_hostFolders, for (final remote in _remoteReady.values) remote.folders]),
    foldersReadable: (session) =>
        session.host == null && (widget.listFolder != null ? session.place == SessionPlace.local : _viaHost && _filesOf(session) != null),
    pickFolder: widget.pickFolder ?? (widget.served != null ? _pickHostFolder : canRunLocalAgents ? () => getDirectoryPath() : null),
    // A served page has no file system: the host makes a chat's folder when the session opens.
    hosts: _hostChoices,
    chooseHost: _chooseHost,
    addHost: _addHost,
    manageHosts: () => _showSection(HostsController.section),
    browseFolder: _browseFolder,
    nativeFolder: widget.pickFolder == null && widget.served == null && canRunLocalAgents && _viaHost ? () => getDirectoryPath() : null,
    sourcesOf: (session) => session.host == null ? _sources : _sourcesOn(session.host!),
    recentFoldersOf: (session) => _recentFoldersOn(session.host),
    repositoryOf: (session) {
      final name = session.host, cwd = session.cwd;
      if (name == null || cwd == null) return null;
      return _remoteReady[name]?.folders.inRepository(cwd) ?? false;
    },
    pathUnchecked: (session) => _connection(session.host)?.kind == ConnectionKind.acp,
    // The host makes a chat's folder when its session opens; what the composer holds is a stand-in for it.
    scratchFolder: widget.served != null || _viaHost ? (_) => siteStandInFolder : null,
    pickFiles: widget.pickFiles ?? () => openFiles(),
    statFile: widget.statFile ?? (canRunLocalAgents ? localFileSize : null),
    uploadFile: widget.served == null ? null : _upload,
    hostFiles: widget.served == null ? null : _onServedHost,
    listFolder: widget.listFolder ?? (_viaHost ? _mentionsOf : null),
    resolveInside: widget.resolveInside ?? (_viaHost ? _resolveIn : null),
    clipboard: widget.clipboard ?? const SystemClipboard(),
    overlays: config.overlays.keys.toList(),
    defaultOverlays: config.defaultOverlays,
    loadouts: {for (final name in config.loadouts.keys) name: config.resolveLoadout(name)!},
    defaultLoadout: config.defaultLoadoutFor(_clientKind),
    onLoadout: _showLoadouts,
    presets: config.presets,
    savePreset: _savePreset,
    onSetup: _showSetup,
    onProfiles: () => unawaited(_showSettings(section: 'profiles')),
    pasteAsFile: config.pasteAsFile,
    enterSteers: config.sessions.whileWorking == WhileWorking.steer,
    warnPermille: config.warnPermille,
    onSourceSettings: () => unawaited(_showSettings(section: 'sources')),
    onSettings: () => unawaited(_showSettings()),
    dictation: _dictation,
    reader: _reader,
    onVoiceMode: _voiceModeChanged,
    accounts: _accounts,
    accountsOf: (session) => _accountsOn(session.host),
    onAccountSettings: () => unawaited(_showSettings(section: 'accounts')),
    switchAccount: (session, account) => store.switchAccount(
      session,
      account,
      move: _accounts == null
          ? null
          : (moving, from, to) => _accounts!.host.moveSession(
              entry: moving.source?.hostEntry ?? moving.agent,
              session: moving.remoteSessionId!,
              cwd: moving.cwd!,
              from: from,
              to: to,
            ),
      start: (moving) async {
        final client = await _agentFor(
          place: moving.place,
          entryName: moving.agent,
          address: moving.endpoint?.toString(),
          fallback: moving.websocketFallback?.toString(),
          methodId: moving.method,
          peerId: moving.peerId,
          cacheLifetime: moving.cacheLifetime?.id,
          overlays: moving.overlays?.toList(),
          account: moving.account,
        );
        _ownedClients.add(client);
        return client;
      },
      expected: _hasProfiles,
    ),
    switchAgent: _moves == null || !_canSwitchAgents
        ? null
        : (session, target) => store.switchAgent(
            session,
            target,
            move: (moving, to) => _moves.move(entry: moving.agent, session: moving.remoteSessionId!, to: to, cwd: moving.cwd),
            start: (moving, source) async {
              final client = await _agentFor(
                place: SessionPlace.local,
                entryName: source.label,
                peerId: null,
                cacheLifetime: moving.cacheLifetime?.id,
                overlays: moving.overlays?.toList(),
              );
              _ownedClients.add(client);
              return client;
            },
          ),
    portable: _portable,
    programs: _programs,
    extension: _tabs,
    onDelegate: (session) {
      final above = _navigator.currentContext;
      if (above != null) unawaited(showDelegateDialog(above, store, session));
    },
    onProfile: (session) {
      final above = _navigator.currentContext;
      final report = session.profile;
      if (above == null || report == null) return;
      unawaited(showSelectableDialog<void>(context: above, builder: (_) => ProfileReportDialog(report: report)));
    },
  );

  /// The loadout and profile the chat-only app's next session starts with, chosen beside its New session button; none in the whole
  /// app, whose composer chooses them on each new session.
  Widget? _startOptions() => _mode.chatOnly ? StartPicker(choice: _start, config: config, client: _clientKind) : null;

  /// Where the client has something in front of it, the sessions related to it above the welcome page.
  Widget _withRelated(Widget welcome) {
    final listed = _relatedAbove(welcome);
    final served = widget.served;
    if (!_needsPairing || served == null) return listed;
    return Column(children: [
      Flexible(
        child: SingleChildScrollView(
          child: PanePairing(hostPeer: served.host.peer, origin: '${served.page.scheme}://${served.page.authority}'),
        ),
      ),
      Expanded(child: listed),
    ]);
  }

  Widget _relatedAbove(Widget welcome) {
    final relations = _relations, ledger = store.ledger;
    if (relations == null || ledger == null) return welcome;
    return Column(children: [
      // At most half the panel: a place with many sessions scrolls inside it.
      Flexible(
        child: SingleChildScrollView(
          child: RelatedSessions(
            source: relations, ledger: ledger, store: store,
            onNew: _newRelatedSession, onResume: _resumeRelated,
            startOptions: _startOptions(),
          ),
        ),
      ),
      Expanded(child: welcome),
    ]);
  }

  /// The family the shown conversation's agent picks when Appearance follows the provider, else the chosen one.
  LookId _familyShown() {
    final session = store.selected;
    return familyFor(
      chosen: config.look,
      follow: config.followProvider,
      inConversations: _layout.section == ShellLayout.conversations,
      provider: session == null ? null : sourceProvider(session.source) ?? session.agent,
    );
  }

  final Map<(LookId, Brightness, String, int), ThemeData> _themes = {};

  ThemeData _theme(LookId family, Brightness brightness) => _themes.putIfAbsent(
    (family, brightness, config.density.name, vendorLooks.revision),
    () => themeFor(family, brightness, density: config.density),
  );

  /// Asks the host for the families it lists, which the Claude and ChatGPT looks draw from; until it answers they draw as Canoa's.
  Future<void> _loadLooks() async {
    final control = _control;
    if (control == null) return;
    try {
      final client = await control.ready(wait: false);
      vendorLooks.adopt(await client.call('looks/list', const {}));
    } on Object catch (error) {
      vendorLooks.failed(error is ControlFailed ? error.message : error is StateError ? error.message : 'no link');
    }
  }

  /// Asks the host which agents it has loaded: the only place the app learns which agents there are, what each is called, and what its
  /// mark looks like. Until it answers, none is known.
  Future<void> _loadProviders() async {
    final control = _control;
    if (control == null) return;
    try {
      final client = await control.ready(wait: false);
      providerCatalog.adopt(await client.call('providers/list', const {}));
    } on Object catch (error) {
      providerCatalog.failed(error is ControlFailed ? error.message : error is StateError ? error.message : 'no link');
    }
  }

  @override
  Widget build(BuildContext context) {
    _popout
      ..look = config.look.name
      ..density = config.density.name;
    return ListenableBuilder(
      listenable: Listenable.merge([vendorLooks, providerCatalog]),
      builder: (_, _) => config.followProvider
          ? ListenableBuilder(listenable: Listenable.merge([store, _layout]), builder: (_, _) => _app(_familyShown()))
          : _app(config.look),
    );
  }

  Widget _app(LookId family) {
    return MaterialApp(
      navigatorKey: _navigator,
      scaffoldMessengerKey: _messenger,
      title: 'Canoa',
      debugShowCheckedModeBanner: false,
      theme: _theme(family, Brightness.light),
      darkTheme: _theme(family, Brightness.dark),
      themeMode: config.themeMode,
      home: AnimatedBuilder(
        animation: Listenable.merge([store, _popout]),
        builder: (context, _) => _CanoaHome(
          store: store, onNewSession: _newSession, onSettings: _showSettings,
          onHosts: () => _showSection(HostsController.section), onLoadout: _showLoadout, onLoadouts: _showLoadouts, onReopen: _reopen, running: _running, onAttach: _attachRunning,
          account: widget.served?.portal?.user, onSignOut: widget.served?.portal?.signOut,
          layout: _layout, surfaces: _surfaces, settings: _settingsPage(), loadout: _loadoutPage(), loadouts: _loadoutsPage(),
          hostsIndex: _hostsIndex(), hostsPage: _hostsPage(), storeIndex: StoreIndex(selection: _storeSelection),
          loadoutsIndex: LoadoutsIndex(selection: _loadoutsSelection, config: config),
          usageIndex: _usageIndex(), usagePage: _usagePage(), sessionSpend: _usage?.sessionSpend,
          terminalIndex: TerminalIndex(controller: _terminalSection), terminalMain: TerminalMain(controller: _terminalSection),
          conversation: _conversation,
          localHost: _localHost,
          chatOnly: _mode.chatOnly,
          relations: _relations,
          onNewRelated: _newRelatedSession,
          onResumeRelated: _resumeRelated,
          onOpenFullApp: _openFullApp,
          startOptions: _startOptions(),
          grouping: _grouping,
          kit: _kit,
          renderers: widget.renderers,
          noAgent: _noAgent,
          welcome: _withRelated(WelcomeView(
            sources: _sources, checks: _checks, programs: _programs, historyImport: _historyImport(),
            onStore: () => unawaited(_showLoadout()),
            onSettings: () => unawaited(_showSettings(section: 'sources')),
          )),
        ),
      ),
    );
  }
}

class _CanoaHome extends StatelessWidget {
  const _CanoaHome({
    required this.store, required this.onNewSession, required this.onSettings,
    this.onHosts, this.onLoadout, this.onLoadouts, this.account, this.onSignOut, this.onReopen, this.running, this.onAttach, required this.conversation, this.kit = const ComposerKit(),
    this.renderers = chat.ChatRenderers.none, required this.welcome, this.noAgent = false, this.localHost, required this.grouping,
    required this.layout, required this.surfaces, required this.settings, required this.loadout, required this.loadouts,
    required this.hostsIndex, required this.hostsPage, required this.storeIndex, required this.loadoutsIndex, this.usageIndex, this.usagePage, this.sessionSpend,
    required this.terminalIndex, required this.terminalMain,
    this.chatOnly = false, this.relations, required this.onNewRelated, required this.onResumeRelated, this.onOpenFullApp, this.startOptions,
  });

  /// The extension's bridge to the Canoa on this computer; null where this is not the extension.
  final LocalHostBridge? localHost;

  final ValueNotifier<SessionGrouping> grouping;
  final ShellLayout layout;
  final WorkSurfaces surfaces;

  /// The main views of the Settings, Store and Loadouts sections.
  final Widget settings, loadout, loadouts;

  /// The Hosts section's index and main view.
  final Widget hostsIndex, hostsPage, storeIndex, loadoutsIndex;

  /// The Usage section's index and main view; none where the host does not grant usage.
  final Widget? usageIndex, usagePage;

  /// The Terminal section's index and main view.
  final Widget terminalIndex, terminalMain;

  /// Whether this is the chat-only app: the conversation and the sessions related to what is in front of the client, with none of
  /// the sections, panels or surfaces of the whole app, and a way to open that.
  final bool chatOnly;

  /// What is in front of the client, and what the chat-only index does with it.
  final RelationSource? relations;
  final Future<void> Function(Relation relation) onNewRelated;
  final Future<void> Function(LedgerEntry entry) onResumeRelated;
  final VoidCallback? onOpenFullApp;

  /// The loadout and profile the next session started for what is in front starts with, chosen beside its button.
  final Widget? startOptions;
  final Future<SpendRow?> Function(String sessionId)? sessionSpend;

  final SessionStore store;
  final Widget welcome;

  /// Whether no agent can start here, which the page of choices for a new session gives way to the welcome page for.
  final bool noAgent;

  /// Opens a new session's empty view.
  final VoidCallback onNewSession;
  final ComposerKit kit;
  final chat.ChatRenderers renderers;
  final VoidCallback onSettings;
  final VoidCallback? onHosts;
  final VoidCallback? onLoadout, onLoadouts;
  final String? account;
  final Future<void> Function()? onSignOut;
  final Future<void> Function(LedgerEntry)? onReopen;

  /// The sessions the host keeps running, and what joins one.
  final RunningSessions? running;
  final Future<void> Function(RunningSession)? onAttach;
  final _ConversationHandle conversation;

  /// The session [step] places from the one chosen, going round the list.
  void _step(int step) {
    final sessions = listOrder(store.listed, grouping.value);
    if (sessions.isEmpty) return;
    final at = sessions.indexWhere((session) => session.id == store.selectedId);
    store.select(sessions[(at + step) % sessions.length].id);
  }

  /// The agent's first option of [kind] for the question shown, if any.
  void _answer(String kind) {
    final session = store.selected;
    if (session == null || session.pendingPermissions.isEmpty) return;
    final options = session.pendingPermissions.first.options;
    final chosen = options.where((option) => option.kind == kind).firstOrNull ??
        (kind == 'reject_once' ? options.where((option) => option.rejects).firstOrNull : null);
    if (chosen != null) store.choosePermission(session, chosen);
  }

  void _stop() {
    if (store.selected?.status == SessionStatus.working) unawaited(store.cancelActiveSession().catchError((Object _) {}));
  }

  /// The rail as the window shows it: beside the conversation, or in the drawer.
  Widget _rail(BuildContext context) => SessionRail(
    store: store, onNewSession: onNewSession,
    onReopen: onReopen, running: running, onAttach: onAttach, grouping: grouping,
    initialScroll: layout.of(ShellLayout.conversations).scroll,
    onScroll: (offset, {required end}) {
      layout.setScroll(ShellLayout.conversations, offset);
      if (end) unawaited(layout.flush());
    },
    onSearch: () => unawaited(_palette(context, sessionsOnly: true)),
    onClose: conversation.close ?? store.close,
  );

  Future<void> _palette(BuildContext context, {bool sessionsOnly = false}) {
    final session = store.selected;
    return showSelectableDialog<void>(
      context: context,
      builder: (_) => CommandPalette(items: sessionsOnly ? [
        for (final other in listOrder(store.listed, grouping.value))
          PaletteItem('Go to ${other.title}', () => store.select(other.id), detail: other.agent, icon: Icons.chat_bubble_outline),
      ] : [
        PaletteItem('New session', onNewSession, icon: Icons.add_comment_outlined),
        if (!chatOnly && session != null && conversation.showChanges != null && (changeTotals(session) != null))
          PaletteItem('Show the changes', conversation.showChanges!, icon: Icons.difference_outlined),
        if (session?.status == SessionStatus.working) PaletteItem('Stop the turn', _stop, icon: Icons.stop_circle_outlined),
        if (session != null && store.canDelegate(session))
          PaletteItem('Delegate task', () => unawaited(showDelegateDialog(context, store, session)),
              detail: 'Hand a task to another agent, model or host', icon: Icons.account_tree_outlined),
        if (session != null && store.canDelegate(session))
          PaletteItem('Delegation setup', () => unawaited(openDelegationSetup(context, store, session)),
              detail: 'Which agents this conversation may delegate to, and what it may attach', icon: Icons.tune),
        if (!chatOnly && session != null)
          if (workspaceOpener.workspaceOf(place: session.place, cwd: session.cwd, endpoint: session.endpoint, peerId: session.peerId).target case final target?)
            for (final action in workspaceOpener.actionsFor(WorkspaceContext.palette, target))
              PaletteItem(action.label, () => unawaited(runWorkspaceAction(context, action, target)),
                  detail: target.path, icon: action.icon),
        if (chatOnly)
          PaletteItem('Open the full app', onSettings, detail: 'Workspaces, terminal, Store and Settings', icon: Icons.open_in_new)
        else ...[
          PaletteItem('Settings', onSettings, icon: Icons.settings_outlined),
          if (onLoadouts != null) PaletteItem('Loadouts', onLoadouts!, icon: Icons.inventory_2_outlined),
          if (onLoadout != null) PaletteItem('Store', onLoadout!, icon: Icons.storefront_outlined),
          if (onHosts != null) PaletteItem('Hosts', onHosts!, detail: 'Local, saved connections and Hosting', icon: Icons.lan_outlined),
        ],
        PaletteItem('Keyboard shortcuts', () => unawaited(showShortcutHelp(context)), icon: Icons.keyboard_outlined),
        for (final other in store.listed)
          if (other.id != store.selectedId)
            PaletteItem('Go to ${other.title}', () => store.select(other.id), detail: other.agent, icon: Icons.chat_bubble_outline),
        for (final command in session?.commands ?? const <AgentCommandInfo>[])
          PaletteItem('/${command.name}', () => conversation.insert?.call('/${command.name} '),
              detail: command.description.isEmpty ? 'A command of the agent\'s' : command.description, icon: Icons.terminal),
        PaletteItem('Open in browser', () => unawaited(askToOpenInBrowser(context, workspaceOpener)),
            detail: 'A web address, in the system browser', icon: Icons.open_in_browser),
      ]),
    );
  }

  /// The keyboard's shortcuts, wherever focus is in the window.
  Map<ShortcutActivator, VoidCallback> _shortcuts(BuildContext context) => {
    chord(LogicalKeyboardKey.keyK): () => unawaited(_palette(context)),
    chord(LogicalKeyboardKey.keyN): onNewSession,
    // The chat-only app has no panels.
    if (!chatOnly) ...{
      chord(LogicalKeyboardKey.backquote): () {
        if (store.selected case final session?) toggleBottomPanel(surfaces: surfaces, layout: layout, session: session);
      },
      chord(LogicalKeyboardKey.keyB, alt: true): () {
        if (store.selected case final session? when surfaces.offers(session, SurfaceSlot.right)) layout.toggleRight();
      },
    },
    // The shell hears it too when focus is inside it; this is for focus above, where nothing in the shell is.
    chord(LogicalKeyboardKey.keyB): () {
      if (layout.section == ShellLayout.conversations) layout.toggleIndex(layout.section);
    },
    chord(LogicalKeyboardKey.comma): onSettings,
    for (final (index, key) in const [
      LogicalKeyboardKey.digit1, LogicalKeyboardKey.digit2, LogicalKeyboardKey.digit3,
      LogicalKeyboardKey.digit4, LogicalKeyboardKey.digit5, LogicalKeyboardKey.digit6,
      LogicalKeyboardKey.digit7, LogicalKeyboardKey.digit8, LogicalKeyboardKey.digit9,
    ].indexed)
      chord(key): () {
        final ordered = listOrder(store.listed, grouping.value);
        if (index < ordered.length) store.select(ordered[index].id);
      },
    chord(LogicalKeyboardKey.tab): () => _step(1),
    chord(LogicalKeyboardKey.tab, shift: true): () => _step(-1),
    chord(LogicalKeyboardKey.keyL): () => conversation.focusComposer?.call(),
    chord(LogicalKeyboardKey.space, shift: true): () => unawaited(kit.dictation?.press() ?? Future.value()),
    chord(LogicalKeyboardKey.period): _stop,
    chord(LogicalKeyboardKey.keyA, shift: true): () => _answer('allow_once'),
    chord(LogicalKeyboardKey.keyR, shift: true): () => _answer('reject_once'),
    chord(LogicalKeyboardKey.keyD, shift: true): () => conversation.showChanges?.call(),
    chord(LogicalKeyboardKey.slash): () => unawaited(showShortcutHelp(context)),
  };

  @override
  Widget build(BuildContext context) {
    return CallbackShortcuts(
      bindings: _shortcuts(context),
      child: Focus(
        autofocus: true,
        // Push to talk ends when any key of the shortcut comes up.
        onKeyEvent: (_, event) {
          if (event is KeyUpEvent && kit.dictation?.holding == true) unawaited(kit.dictation!.release());
          return KeyEventResult.ignored;
        },
        child: _layout(context),
      ),
    );
  }

  Widget _layout(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      final narrow = AppShell.isNarrow(constraints.maxWidth) ||
          Uri.base.queryParameters['view'] == 'panel';
      return Selectable(child: AppShell(
        layout: layout,
        narrow: narrow,
        account: account,
        onSignOut: onSignOut,
        indexWidth: context.look.railWidth,
        banner: localHost == null ? null : LocalHostBanner(bridge: localHost!),
        selector: !chatOnly,
        sections: [
          AppSection(
            id: ShellLayout.conversations,
            ownsLayoutControls: true,
            label: 'Conversations',
            icon: Icons.chat_bubble_outline,
            index: chatOnly ? _relatedRail() : _rail(context),
            main: _Conversation(
              key: ValueKey(store.selectedId ?? 'empty'),
              store: store,
              narrow: narrow,
              handle: conversation,
              layout: layout,
              surfaces: surfaces,
              kit: kit,
              renderers: renderers,
              welcome: welcome,
              noAgent: noAgent,
              sessionSpend: sessionSpend,
              chatOnly: chatOnly,
              onOpenFullApp: onOpenFullApp,
            ),
          ),
          if (!chatOnly) ...[
          AppSection(id: HostsController.section, label: 'Hosts', icon: Icons.lan_outlined, index: hostsIndex, main: hostsPage),
          AppSection(id: 'terminal', label: 'Terminal', icon: Icons.terminal, index: terminalIndex, main: terminalMain),
          if (usagePage != null) AppSection(id: 'usage', label: 'Usage', icon: Icons.speed, index: usageIndex, main: usagePage!),
          AppSection(id: 'loadouts', label: 'Loadouts', icon: Icons.inventory_2_outlined, index: loadoutsIndex, main: loadouts),
          AppSection(id: 'store', label: 'Store', icon: Icons.storefront_outlined, index: storeIndex, main: loadout, ownsLayoutControls: true),
          AppSection(id: 'settings', label: 'Settings', tooltip: 'Canoa settings', icon: Icons.settings_outlined, main: settings, atFoot: true),
          ],
        ],
      ));
    });
  }

  /// The chat-only app's index: the sessions of what is in front first.
  Widget _relatedRail() => RelatedRail(
    source: relations!,
    store: store,
    ledger: store.ledger,
    onNew: onNewRelated,
    onNewSession: onNewSession,
    onResume: onResumeRelated,
    onOpenFullApp: onOpenFullApp,
    running: running,
    onAttach: onAttach,
    startOptions: startOptions,
  );
}

/// What a start under way has made so far, for quitting to take back.
final class _Starting {
  Worktrees? worktrees;
  SessionWorktree? made;
  VerifiedAgent? client;
}

/// What the conversation shown offers the keyboard and the palette, set by it while it is shown.
final class _ConversationHandle {
  Object? owner;
  VoidCallback? focusComposer, showChanges;
  ValueChanged<String>? insert;

  /// Ends a conversation as the app does, asking after its worktree; set by the app.
  Future<void> Function(ChatSession)? close;

  /// The sources a session can move to, by name and as shown, and the move itself; set by the app.
  Map<String, String> Function(ChatSession)? moveTargets;
  Future<void> Function(BuildContext, ChatSession)? move;

  /// The sessions the host brought in for a session's source, for Earlier conversations, whether it has any to list, and the
  /// showing of one; set by the app.
  bool Function(ChatSession)? canListImported;
  Future<List<AcpSessionInfo>> Function(ChatSession)? imported;
  Future<void> Function(ChatSession, AcpSessionInfo)? showImported;
  /// The conversations shown in a window of their own; set by the app.
  PopoutHub? popout;
}

class _Conversation extends StatefulWidget {
  const _Conversation({
    super.key, required this.store, required this.narrow, required this.handle, required this.layout, required this.surfaces,
    this.kit = const ComposerKit(), this.renderers = chat.ChatRenderers.none, required this.welcome, this.noAgent = false, this.sessionSpend,
    this.chatOnly = false, this.onOpenFullApp,
  });

  /// Whether this is the chat-only app, whose conversation has no work panels, inspector, terminal or workspace to open.
  final bool chatOnly;

  /// Opens the whole app, which the chat-only header offers.
  final VoidCallback? onOpenFullApp;

  final Widget welcome;
  final bool noAgent;

  /// What the host recorded a session spending, for the inspector; null where the host does not grant usage.
  final Future<SpendRow?> Function(String sessionId)? sessionSpend;
  final _ConversationHandle handle;
  final ComposerKit kit;
  final chat.ChatRenderers renderers;

  final SessionStore store;
  final bool narrow;
  final ShellLayout layout;
  final WorkSurfaces surfaces;

  @override
  State<_Conversation> createState() => _ConversationState();
}

class _ConversationState extends State<_Conversation> {
  final TextEditingController _input = TextEditingController();

  /// The composer's focus.
  final FocusNode _composerFocus = FocusNode();

  /// The conversation, which keeps its end in view while the person has not scrolled away.
  final _transcript = GlobalKey<chat.ChatTranscriptState>();

  /// A draft's composer text goes with it, across sessions shown and runs of the app.
  void _keepText() {
    final session = widget.store.selected;
    if (session != null && session.isDraft) widget.store.keepDraftText(session, _input.text);
  }

  @override
  void initState() {
    super.initState();
    final session = widget.store.selected;
    // A draft's text, or a first message not yet taken by its agent.
    if (session != null && (session.isDraft || session.draftText.isNotEmpty)) _input.text = session.draftText;
    _input.addListener(_keepText);
    widget.store.addListener(_followReplies);
    widget.handle
      ..owner = this
      ..focusComposer = _composerFocus.requestFocus
      ..showChanges = widget.chatOnly ? null : () {
        final session = widget.store.selected;
        if (session != null && !session.isDraft) _showInspector(session);
      }
      ..insert = (text) {
        _input.value = TextEditingValue(text: text, selection: TextSelection.collapsed(offset: text.length));
        _composerFocus.requestFocus();
      };
  }

  /// Voice mode hears the conversation shown as it grows, and reads what is new.
  void _followReplies() {
    final reader = widget.kit.reader, session = widget.store.selected;
    // A child's conversation shown in its place follows itself (child_thread.dart), and the parent's is not read meanwhile.
    if (reader == null || !reader.voiceMode || session == null || session.isDraft || session.openChild != null) return;
    followReplies(reader, session.id, session.messages, working: session.status == SessionStatus.working);
  }

  /// This conversation's renderers: the delegation rows, the terminal's, replies with Play beside Copy, and the app's own.
  chat.ChatRenderers _renderersFor(ChatSession session) {
    final reader = widget.kit.reader;
    var renderers = delegationRenderers.merge(terminalRenderers(offered: () => _terminalSurface(session) != null, run: (command) => _runInTerminal(session, command)));
    if (reader != null) renderers = renderers.merge(replyRenderers(reader));
    return renderers.merge(widget.renderers);
  }

  /// Adds [text] to the message being written, on a line of its own.
  void _addToComposer(String text) {
    final had = _input.text;
    final joined = had.isEmpty || had.endsWith('\n') ? '$had$text' : '$had\n$text';
    _input.value = TextEditingValue(text: joined, selection: TextSelection.collapsed(offset: joined.length));
  }

  /// Shows the surface [id] in its panel, or hides the panel when that surface is the one it shows. On a narrow window the panel
  /// takes the whole window, with a way back.
  void _toggleSurface(ChatSession session, String id) {
    final surface = widget.surfaces.registry.where((each) => each.id == id).firstOrNull;
    if (surface == null) return;
    final mine = widget.surfaces.of(session.id);
    final open = mine.find(id);
    final layout = widget.layout;
    final slot = open?.slot ?? surface.placements.first;
    final panelOpen = slot == SurfaceSlot.right ? layout.rightOpen : layout.bottomOpen;
    if (open != null && panelOpen && layout.focus != PanelFocus.main && identical(mine.activeIn(slot), open)) {
      slot == SurfaceSlot.right ? layout.setRightOpen(false) : layout.setBottomOpen(false);
      return;
    }
    // What the disk holds is read when a surface is shown, so it is not older than the person's last look.
    widget.store.refreshGit(session);
    mine.openSurface(surface);
    slot == SurfaceSlot.right ? layout.showRight() : layout.showBottom();
  }

  /// The Terminal surface when the host [session] runs on gives a shell to open.
  TerminalSurface? _terminalSurface(ChatSession session) {
    if (widget.chatOnly) return null;
    final surface = widget.surfaces.registry.whereType<TerminalSurface>().firstOrNull;
    return surface != null && surface.accessFor(session) is TerminalOnHost ? surface : null;
  }

  /// Types [command] into the session's terminal, shown, without pressing Enter.
  void _runInTerminal(ChatSession session, String command) {
    final surface = _terminalSurface(session);
    if (surface == null) return;
    final opened = widget.surfaces.of(session.id).openSurface(surface);
    opened.slot == SurfaceSlot.bottom ? widget.layout.showBottom() : widget.layout.showRight();
    surface.workspaceFor(opened.controller, session)?.focused?.type(command);
  }

  /// The Files surface when the host [session] runs on gives its files to browse.
  FilesSurface? _filesSurface(ChatSession session) {
    if (widget.chatOnly) return null;
    final surface = widget.surfaces.registry.whereType<FilesSurface>().firstOrNull;
    return surface != null && surface.available(session) ? surface : null;
  }

  /// Shows [path], a file of the session's host, in the Files surface.
  void _openInFiles(ChatSession session, String path) {
    final surface = _filesSurface(session);
    if (surface == null) return;
    final opened = widget.surfaces.of(session.id).openSurface(surface, null, (controller) => controller.data[FilesSurface.revealKey] = FileReveal(path));
    opened.slot == SurfaceSlot.bottom ? widget.layout.showBottom() : widget.layout.showRight();
  }

  /// Takes [text] as a text file on the message being written, or says why it cannot.
  void _attachText(ChatSession session, String name, String text) {
    final messenger = ScaffoldMessenger.of(context);
    final problem = widget.store.addAttachment(session, name, utf8.encode(text), mimeType: 'text/plain');
    if (problem != null) messenger.showSnackBar(SnackBar(content: Text(problem)));
  }

  void _showInspector(ChatSession session) => _toggleSurface(session, DetailsSurface.surfaceId);

  @override
  void dispose() {
    // A conversation chosen next may have taken the handle already.
    if (identical(widget.handle.owner, this)) {
      widget.handle
        ..owner = null
        ..focusComposer = null
        ..showChanges = null
        ..insert = null;
    }
    _input.removeListener(_keepText);
    widget.store.removeListener(_followReplies);
    _composerFocus.dispose();
    _input.dispose();
    super.dispose();
  }

  Future<void> _showEarlier(ChatSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    final handle = widget.handle;
    final brought = <String>{};
    final picked = await showSelectableDialog<AcpSessionInfo>(
      context: context,
      builder: (context) => EarlierConversations(
        list: (cursor) => earlierPage(
          cursor,
          imported: (handle.canListImported?.call(session) ?? false) ? () => handle.imported!(session) : null,
          agent: widget.store.canListEarlier(session) ? (next) => widget.store.earlierSessions(session, cursor: next) : null,
          onImported: brought.addAll,
        ),
      ),
    );
    if (picked == null) return;
    try {
      if (brought.contains(picked.id)) {
        await handle.showImported!(session, picked);
      } else {
        await widget.store.openEarlier(session, picked);
      }
    } catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('That conversation could not be opened: $error')));
    }
  }

  /// Puts what was queued back in the composer, after what is written there.
  void _queuedToComposer(String text) {
    final current = _input.text;
    final joined = current.trim().isEmpty ? text : '$current\n\n$text';
    _input.value = TextEditingValue(text: joined, selection: TextSelection.collapsed(offset: joined.length));
    _composerFocus.requestFocus();
  }

  /// Sends the composer's message. While a turn runs it queues, or with [steer] stops the turn and goes first.
  void _send({bool steer = false}) {
    final text = _input.text;
    final session = widget.store.selected;
    final attached = session?.attachments.isNotEmpty ?? false;
    if (text.trim().isEmpty && !attached) return;
    // A draft's first message shows in the conversation while its agent starts; a start that fails puts it back here.
    final starting = session != null && session.isDraft;
    try {
      _input.clear();
      unawaited(widget.store.sendText(text, steer: steer).catchError((Object error) {
        if (!mounted) return;
        // Not sent, so it is there to change and send again.
        if (session != null && !identical(widget.store.selected, session)) {
          widget.store.keepDraftText(session, text);
        } else if (_input.text.isEmpty) {
          _input.text = text;
        }
        final started = session?.remoteSessionId != null;
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('${steer ? 'Not steered' : starting && !started ? 'Not started' : 'Message not accepted'}: ${reasonOf(error)}')),
        );
      }));
    } on StateError {
      ScaffoldMessenger.of(context).showSnackBar(const SnackBar(
        content: Text('Connect and verify an ACP peer before sending.'),
      ));
    }
    _transcript.currentState?.jumpToLatest();
  }

  /// Under the title: the agent. Its folder, branch and worktree are in the inspector.
  static String _subtitle(ChatSession? session) => session == null ? 'Multi-session agent chat' : session.agent;

  /// The state a session's header names.
  static String _state(ChatSession? session) => switch (session?.status) {
    null => 'Offline',
    SessionStatus.preview => 'Preview',
    SessionStatus.draft => 'New',
    SessionStatus.connecting when session!.isDraft => 'Starting',
    final status => status.name,
  };

  Future<void> _popOut(ChatSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    try {
      await widget.handle.popout!.popOut(session);
    } catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('Could not open a window: ${error is StateError ? error.message : error}')));
    }
  }

  /// The header's entry for opening [session]'s folder on this computer, or null where nothing can be started.
  (String, IconData, VoidCallback)? _workspaceEntry(ChatSession? session) {
    if (session == null) return null;
    final workspace = workspaceOpener.workspaceOf(place: session.place, cwd: session.cwd, endpoint: session.endpoint, peerId: session.peerId);
    if (workspace.target == null && workspace.reason == null) return null;
    return ('Open the workspace', Icons.folder_open, () => unawaited(showWorkspaceActions(
      context, opener: workspaceOpener, target: workspace.target, reason: workspace.reason,
    )));
  }

  /// What the header offers for [session], beside closing it: as buttons, or under one menu where two or more would crowd a phone.
  List<Widget> _headerActions(ChatSession? session, {bool tight = false}) {
    final actions = <(String, IconData, VoidCallback)>[
      if (session != null && (widget.store.canListEarlier(session) || (widget.handle.canListImported?.call(session) ?? false)))
        ('Earlier conversations', Icons.history, () => unawaited(_showEarlier(session))),
      if (session != null && !session.isDraft && !widget.chatOnly) ('Session details', Icons.view_sidebar_outlined, () => _showInspector(session)),
      if (session != null && !widget.chatOnly && (widget.handle.moveTargets?.call(session).isNotEmpty ?? false))
        ('Move to…', Icons.drive_file_move_outlined, () => unawaited(widget.handle.move!(context, session))),
      if (session != null && (!widget.narrow || Uri.base.queryParameters['view'] == 'panel') && !session.isDraft && (widget.handle.popout?.available ?? false))
        if (widget.handle.popout!.isPoppedOut(session))
          ('Bring back from its own window', Icons.close_fullscreen, () => unawaited(widget.handle.popout!.dockBack(session)))
        else
          ('Open in its own window', Icons.open_in_new, () => unawaited(_popOut(session))),
      if (!widget.chatOnly) ?_workspaceEntry(session),
      if (widget.chatOnly && widget.onOpenFullApp != null) ('Open the full app', Icons.open_in_new, widget.onOpenFullApp!),
      if (session?.profile case final report?)
        ('Profile', Icons.tune, () => unawaited(showSelectableDialog<void>(
          context: context,
          builder: (_) => ProfileReportDialog(report: report),
        ))),
    ];
    if (!(widget.narrow || tight) || actions.length < 2) {
      return [
        for (final (tooltip, icon, onPressed) in actions) IconButton(tooltip: tooltip, icon: Icon(icon), onPressed: onPressed),
      ];
    }
    return [
      PopupMenuButton<VoidCallback>(
        tooltip: 'More',
        onSelected: (chosen) => chosen(),
        itemBuilder: (context) => [
          for (final (label, icon, onPressed) in actions)
            PopupMenuItem(value: onPressed, child: ListTile(leading: Icon(icon), title: Text(label))),
        ],
      ),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final chosen = widget.store.selected;
    // With no agent to start one, a new session's page of choices gives way to the welcome page, which is no session's.
    final session = chosen != null && widget.noAgent && chosen.isDraft && !widget.store.isStarting(chosen) ? null : chosen;
    // A first message shown here once its agent has taken it, sent from another view of this session, goes.
    if (session?.firstSent case final sent? when _input.text == sent && !widget.store.isStarting(session!)) {
      session.firstSent = null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && _input.text == sent) _input.clear();
      });
    }

    final conversation = Column(children: [
      Material(
        color: context.look.bar,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          // A header with panels open beside it is narrow though the window is not: its actions go under one menu, as on a phone.
          child: LayoutBuilder(builder: (context, headerBox) => Row(children: [
            if (widget.narrow)
              Builder(builder: (inner) => IconButton(
                tooltip: 'Sessions',
                icon: const Icon(Icons.menu),
                onPressed: () => Scaffold.of(inner).openDrawer(),
              )),
            Expanded(child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(session?.title ?? 'Canoa', style: const TextStyle(
                  fontSize: 17, fontWeight: FontWeight.w600,
                ), maxLines: 1, overflow: TextOverflow.ellipsis),
                Text(_subtitle(session),
                  style: Theme.of(context).textTheme.bodySmall, maxLines: 1, overflow: TextOverflow.ellipsis),
              ],
            )),
            if (session != null && !session.isDraft && !widget.chatOnly)
              if (changeTotals(session) case (final added, final removed)) _ChangesChip(added: added, removed: removed, onPressed: () => _showInspector(session)),
            if (session != null && session.others.isNotEmpty) PresenceMark(others: session.others, narrow: widget.narrow),
            if (session != null && widget.store.inBypass(session)) _BypassMark(narrow: widget.narrow),
            ..._headerActions(session, tight: headerBox.maxWidth < 680),
            if (session != null)
              IconButton(
                tooltip: 'Close conversation',
                icon: const Icon(Icons.close),
                onPressed: () => unawaited((widget.handle.close ?? widget.store.close)(session)),
              ),
            // On a phone the state is an icon whose tooltip names it, which leaves the conversation's name room to be read.
            // The welcome page, with no session or no agent to start one, has no state to name.
            if (session == null)
              const SizedBox.shrink()
            else if (widget.narrow || headerBox.maxWidth < 680)
              Tooltip(
                message: _state(session),
                child: const Padding(padding: EdgeInsets.all(8), child: Icon(Icons.hub_outlined, size: 18)),
              )
            else
              Chip(
                avatar: const Icon(Icons.hub_outlined, size: 16),
                label: Text(_state(session)),
                visualDensity: VisualDensity.compact,
              ),
            if (!widget.narrow && !widget.chatOnly)
              LayoutControls(
                layout: widget.layout,
                section: ShellLayout.conversations,
                hasIndex: true,
                hasRight: session != null && widget.surfaces.offers(session, SurfaceSlot.right),
                hasBottom: session != null && widget.surfaces.offers(session, SurfaceSlot.bottom),
              ),
          ])),
        ),
      ),
      if (session == null)
        Expanded(child: widget.welcome)
      else if (session.isDraft && !widget.store.isStarting(session))
        Expanded(
          child: _dropping(session, NewSessionView(
            store: widget.store,
            session: session,
            kit: widget.kit,
            composer: _composerFor(session, centred: true),
          )),
        )
      else if (session.openChild != null && session.tasks[session.openChild!] != null)
        Expanded(
          child: ChildThreadView(
            store: widget.store, session: session, task: session.tasks[session.openChild!]!,
            renderers: widget.kit.reader == null
                ? delegationRenderers.merge(widget.renderers)
                : delegationRenderers.merge(replyRenderers(widget.kit.reader!)).merge(widget.renderers),
            reader: widget.kit.reader,
            narrow: widget.narrow,
          ),
        )
      else ...[
        Expanded(
          child: _dropping(session, chat.ChatTranscript(
            key: _transcript,
            entries: delegationEntries(session, onOpen: (id) => widget.store.openChild(session, id), now: widget.store.clock),
            terminals: session.terminals,
            plan: session.plan,
            working: session.status == SessionStatus.working || widget.store.isStarting(session),
            agentName: session.agent,
            turns: [for (final turn in session.chat.turns) chat.TurnSpan.of(turn)],
            renderers: _renderersFor(session),
            narrow: widget.narrow,
          )),
        ),
        if (session.status == SessionStatus.working && session.tasks.live.isNotEmpty)
          DelegationWaiting(
            children: [for (final task in session.tasks.live) ChildView.of(task)],
            onStop: () => unawaited(widget.store.stopTasks(session)),
          ),
        if (session.queue.isNotEmpty) SessionQueuePanel(store: widget.store, session: session, toComposer: _queuedToComposer),
        if (session.pendingPermissions.isNotEmpty) SessionPermissionCard(store: widget.store, session: session),
        if (session.pendingQuestions.isNotEmpty) SessionQuestionCard(store: widget.store, session: session),
        if (session.pendingSecrets.isNotEmpty) SessionSecretCard(store: widget.store, session: session),
        // The context's fraction is in the inspector.
        chat.UsageFooter(text: chat.describeUsage(SessionUsage(cost: session.usage.cost, currency: session.usage.currency), spend: session.spend)),
        Container(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 20),
          child: Center(
            heightFactor: 1,
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: context.chatLook.messageWidth + 100),
              child: Column(mainAxisSize: MainAxisSize.min, children: [
                _composerFor(session),
                if (!canRunLocalAgents || session.place != SessionPlace.local) ...[
                  const SizedBox(height: 8),
                  Text('Remote sessions require verified peer authentication',
                    style: TextStyle(fontSize: 11, color: context.chatLook.muted)),
                ],
              ]),
            ),
          ),
        ),
      ],
    ]);
    if (session == null || widget.chatOnly) return conversation;
    return PanelFrame(
      layout: widget.layout,
      session: session,
      surfaces: widget.surfaces.of(session.id),
      registry: widget.surfaces,
      narrow: widget.narrow,
      child: conversation,
      wrap: (panels) => SurfaceEnvironment(
        store: widget.store,
        sessionSpend: widget.sessionSpend,
        onComment: _addToComposer,
        onAttachText: widget.store.canAttach(session) ? (name, text) => _attachText(session, name, text) : null,
        onOpenInFiles: _filesSurface(session) == null ? null : (path) => _openInFiles(session, path),
        onOpenChild: () {
          if (widget.narrow) widget.layout.setRightOpen(false);
        },
        child: panels,
      ),
    );
  }

  /// The composer of [session]: in the middle of a new session's view when [centred], else docked under the conversation.
  Widget _composerFor(ChatSession session, {bool centred = false}) => SessionComposer(
    store: widget.store,
    session: session,
    controller: _input,
    focusNode: _composerFocus,
    kit: widget.kit,
    centred: centred,
    enabled: !widget.store.isStarting(session) && (session.status == SessionStatus.preview ||
        session.status == SessionStatus.draft ||
        session.status == SessionStatus.connected ||
        session.status == SessionStatus.working),
    onSend: _send,
    onSteer: () => _send(steer: true),
    working: session.status == SessionStatus.working,
    onStop: () {
      final messenger = ScaffoldMessenger.of(context);
      unawaited(widget.store.cancelActiveSession().catchError((Object error) {
        messenger.showSnackBar(SnackBar(content: Text('Could not stop: $error')));
      }));
    },
    commands: session.commands,
  );

  /// [child], taking files dropped on it as attachments where this app can read them.
  Widget _dropping(ChatSession session, Widget child) {
    if (widget.kit.statFile == null || !widget.store.canAttach(session)) return child;
    return DropTarget(
      onDragDone: (details) async {
        final messenger = ScaffoldMessenger.of(context);
        final problems = await addPaths(widget.store, session, [
          for (final file in details.files)
            if (file.path.isNotEmpty) file.path,
        ], widget.kit);
        if (problems.isNotEmpty) messenger.showSnackBar(SnackBar(content: Text(problems.join('\n'))));
      },
      child: child,
    );
  }
}

/// The lines a session's changes add and remove, in the header: it opens the inspector at the changes.
class _ChangesChip extends StatelessWidget {
  const _ChangesChip({required this.added, required this.removed, required this.onPressed});

  final int added, removed;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final mono = TextStyle(fontFamily: look.codeFont, fontSize: 12, fontWeight: FontWeight.w600);
    return Tooltip(
      message: 'Changes',
      child: InkWell(
        key: const ValueKey('changes chip'),
        onTap: onPressed,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
          child: Row(mainAxisSize: MainAxisSize.min, children: [
            Text('+$added', style: mono.copyWith(color: look.success)),
            const SizedBox(width: 4),
            Text('−$removed', style: mono.copyWith(color: look.danger)),
          ]),
        ),
      ),
    );
  }
}

/// Says in the header that the session is in Bypass, which the mode control in the composer also shows. Leaving it is a choice of
/// another mode there.
class _BypassMark extends StatelessWidget {
  const _BypassMark({required this.narrow});

  final bool narrow;

  static const message = 'Bypass: each request that offers to allow is allowed without asking you.';

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Tooltip(
      message: message,
      child: Padding(
        key: const Key('bypass-mark'),
        padding: const EdgeInsets.symmetric(horizontal: 4),
        child: narrow
            ? Icon(Icons.fast_forward_outlined, size: 18, color: scheme.tertiary)
            : Chip(
                avatar: Icon(Icons.fast_forward_outlined, size: 16, color: scheme.onTertiaryContainer),
                label: Text('Bypass', style: TextStyle(color: scheme.onTertiaryContainer)),
                backgroundColor: scheme.tertiaryContainer,
                side: BorderSide.none,
                visualDensity: VisualDensity.compact,
              ),
      ),
    );
  }
}
