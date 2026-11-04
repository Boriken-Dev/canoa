import 'attention/notifier.dart';
import 'config/settings_dialog.dart';
import 'dart:async';
import 'config/canoa_config.dart';
import 'config/canoa_config_file.dart';
import 'desktop/desktop_shell.dart';
import 'desktop/host_supervisor.dart';
import 'desktop/host_tool.dart';
import 'desktop/hosting_dialog.dart';
import 'extension_context.dart';
import 'acp/browser_connect.dart';
import 'acp/connect.dart' show chatEndpoints;
import 'acp/provider_connect.dart';
import 'acp/agent_commands.dart';
import 'acp/client.dart';
import 'acp/local_agent.dart';
import 'package:coqui_acp/page.dart';
import 'package:desktop_drop/desktop_drop.dart';
import 'connect_dialog.dart';
import 'package:file_selector/file_selector.dart';
import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart' show KeyDownEvent, KeyRepeatEvent, LogicalKeyboardKey;

import 'chat/agent_events.dart' show AgentCommandInfo;
import 'chat/changes.dart';
import 'chat/clipboard.dart';
import 'chat/composer.dart';
import 'chat/new_session_view.dart';
import 'chat/session_options.dart' show OptionRole;
import 'chat/palette.dart';
import 'chat/queue_panel.dart';
import 'chat/question_card.dart';
import 'chat/permission_card.dart';
import 'chat/transcript.dart';
import 'chat/update_log.dart';
import 'auth/peer_proof.dart' show PeerMethod;
import 'ledger/ledger_backend.dart';
import 'ledger/ledger_list.dart';
import 'ledger/session_ledger.dart';
import 'profile/apply.dart';
import 'profile/launch.dart';
import 'profile/profile.dart' show OverlaySource;
import 'profile/profile_home.dart';
import 'profile/profile_home_platform.dart';
import 'profile/profile_layers.dart';
import 'profile/profile_start_dialog.dart';
import 'profile/profiles_dialog.dart';
import 'session_store.dart';
import 'sources/sources.dart';
import 'drafts/draft_backend.dart';
import 'drafts/draft_store.dart';
import 'theme/canoa_mark.dart';
import 'theme/look.dart';
import 'auth/browser_peer.dart' show verifyBrowserPeer;
import 'worktree/close_dialog.dart';
import 'worktree/host.dart';
import 'worktree/local.dart';
import 'worktree/worktree.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final config = await loadConfig();
  // Beside Canoa.cfg on a desktop, in this origin's IndexedDB on the web.
  final ledger = SessionLedger(platformLedgerBackend(configPath: desktopConfigPath()), settings: config.sessions);
  await ledger.load();
  final drafts = DraftKeeper(platformDraftBackend(configPath: desktopConfigPath()));
  final shell = createDesktopShell();
  final settingsSignal = ValueNotifier<int>(0);
  final hostingSignal = ValueNotifier<int>(0);
  final host = shell.isDesktop ? HostSupervisor(findHostTool()) : null;
  await shell.initialize(
    config,
    onSettings: () => settingsSignal.value++,
    onHosting: host == null ? null : () => hostingSignal.value++,
  );
  runApp(CanoaApp(
    config: config, shell: shell, settingsSignal: settingsSignal,
    host: host, hostingSignal: hostingSignal, ledger: ledger, notifier: platformNotifier(), drafts: drafts,
  ));
  await host?.apply(configPath: desktopConfigPath(), hasEntries: config.hostEntries.isNotEmpty);
}

/// Shared Flutter application for standalone desktop/web and extension views.
/// The extension's privileged browser permissions remain in its JS service worker.
class CanoaApp extends StatefulWidget {
  const CanoaApp({
    super.key, this.store, this.config = const CanoaConfig(),
    this.shell, this.settingsSignal, this.host, this.hostingSignal,
    this.configPath, this.writeConfig, this.ledger, this.notifier, this.drafts,
    this.checkSource, this.isGitRepository, this.pickFolder, this.pickFiles, this.statFile, this.clipboard, this.startAgent,
    this.worktrees,
  });

  /// Where sessions not yet started are kept between runs, for a store this app makes.
  final DraftKeeper? drafts;

  /// What a source needs and whether it is here; by default its programs are looked for on PATH.
  @visibleForTesting
  final Future<SourceCheck> Function(AgentSource source)? checkSource;

  /// The folder, file and clipboard services the composer uses, each the platform's by default.
  @visibleForTesting
  final bool Function(String folder)? isGitRepository;
  @visibleForTesting
  final Future<String?> Function()? pickFolder;
  @visibleForTesting
  final Future<List<XFile>> Function()? pickFiles;
  @visibleForTesting
  final Future<int?> Function(String path)? statFile;
  @visibleForTesting
  final ClipboardReader? clipboard;

  /// Starts an agent on this computer; by default as a child of this app.
  @visibleForTesting
  final StartAgent? startAgent;

  /// Makes worktrees for agents on this computer; by default by git.
  @visibleForTesting
  final Worktrees? Function()? worktrees;

  final SessionStore? store;

  /// How a session that needs the person reaches them outside the window; none, nothing does.
  final Notifier? notifier;

  /// The list of sessions kept across runs, for a store this app makes; a given [store] brings its own.
  final SessionLedger? ledger;
  final CanoaConfig config;
  final DesktopShell? shell;
  final ValueNotifier<int>? settingsSignal;

  /// The `canoa-host` this app runs, on a desktop.
  final HostSupervisor? host;
  final ValueNotifier<int>? hostingSignal;

  /// Where Canoa.cfg is and how it is written; by default the platform's place.
  @visibleForTesting
  final String? configPath;
  @visibleForTesting
  final Future<void> Function(CanoaConfig)? writeConfig;

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
  Future<void> _write(CanoaConfig changed) => (widget.writeConfig ?? saveConfig)(changed);

  // One of each at a time: a second copy would hold a configuration the first one changes.
  bool _hostingOpen = false, _settingsOpen = false;

  Future<void> _showHosting() async {
    final host = widget.host;
    if (host == null || _hostingOpen) return;
    final above = _navigator.currentContext;
    if (above == null) return;
    _hostingOpen = true;
    try {
      await showDialog<void>(
        context: above,
        builder: (_) => HostingDialog(supervisor: host, config: config, onSave: _saveHosting),
      );
    } finally {
      _hostingOpen = false;
    }
  }

  /// Saves Canoa.cfg once canoa-host finds nothing wrong with it, then serves it.
  /// Only [host] comes from the dialog: the rest is what the app has now, which Settings may have changed meanwhile.
  Future<String?> _saveHosting(CanoaConfig fromDialog) async {
    final host = widget.host;
    final path = _configPath;
    if (host == null || path == null) return 'Canoa.cfg has no place on this computer';
    final changed = config.withHost(fromDialog.host);
    if (changed.hostEntries.isNotEmpty) {
      final problem = await withFileBeside(path, changed.encode(), host.check);
      if (problem != null) return problem;
    }
    await _write(changed);
    if (mounted) setState(() => config = changed);
    await host.apply(configPath: path, hasEntries: changed.hostEntries.isNotEmpty);
    return null;
  }

  Future<void> _showSettings({bool sources = false}) async {
    final shell = widget.shell;
    final above = _navigator.currentContext;
    if (above == null || _settingsOpen) return;
    _settingsOpen = true;
    final CanoaConfig? fromDialog;
    try {
      fromDialog = await showDialog<CanoaConfig>(
        context: above,
        builder: (_) => SettingsDialog(
          config: config, onQuit: shell != null && shell.isDesktop ? shell.quit : null, ledger: store.ledger,
          onProfiles: _showProfiles, agents: [for (final agent in [...agentPresets, ...config.agents]) agent.label],
          onAllowNotifications: kIsWeb ? widget.notifier?.requestPermission : null,
          presets: canRunLocalAgents ? agentPresets : const [],
          hosted: [
            if (widget.host != null)
              for (final entry in config.hostEntries)
                if (entry.kind == 'agent') entry.name,
          ],
          checkSource: _check,
          openSources: sources,
        ),
      );
    } finally {
      _settingsOpen = false;
    }
    if (fromDialog == null || !mounted) return;
    // [host] and the profile tables stay as the app has them now: Hosting may have changed one, and an overlay installed
    // from Profiles is pinned in [overlays], while Settings was open.
    final changed = fromDialog.withHost(config.host).withProfilesOf(config);
    setState(() => config = changed);
    store.embedLimit = changed.embedLimit;
    if (_checked) unawaited(_checkSources());
    store.ledger?.configure(changed.sessions);
    for (final agent in {...store.fullAccessAgents, ...changed.fullAccess}) {
      store.setAgentFullAccess(agent, changed.fullAccess.contains(agent));
    }
    await widget.shell?.update(changed);
    try {
      await _write(changed);
    } on UnsupportedError {
      // Canoa.cfg in web/PWA deployments is read-only.
    } catch (error) {
      if (mounted) {
        _messenger.currentState?.showSnackBar(
          SnackBar(content: Text('Could not save Canoa.cfg: $error')),
        );
      }
    }
  }
  /// Ends full access for [agent], in its sessions and in Canoa.cfg: the banner's one action, where the agent's setting turned it on.
  Future<void> _endAgentFullAccess(String agent) async {
    store.setAgentFullAccess(agent, false);
    if (!config.fullAccess.contains(agent)) return;
    final changed = config.copyWith(fullAccess: {...config.fullAccess}..remove(agent));
    setState(() => config = changed);
    try {
      await _write(changed);
    } on UnsupportedError {
      // Canoa.cfg in web/PWA deployments is read-only.
    } catch (error) {
      _messenger.currentState?.showSnackBar(SnackBar(content: Text('Could not save Canoa.cfg: $error')));
    }
  }

  /// The profiles as Canoa.cfg has them now, for every agent the app offers or the file gives a profile.
  Future<void> _showProfiles() async {
    final above = _navigator.currentContext;
    if (above == null) return;
    await showDialog<void>(
      context: above,
      builder: (_) => ProfilesDialog(
        config: config,
        agents: {for (final agent in [...agentPresets, ...config.agents]) agent.label, ...config.agentProfiles.keys}
            .toList(),
        home: platformProfileHome(configPath: _configPath),
        onPin: (name, digest) async {
          final changed = config.withOverlayPin(name, digest);
          await _write(changed);
          if (mounted) setState(() => config = changed);
          return changed;
        },
        onSave: (changed) async {
          await _write(changed);
          if (mounted) setState(() => config = changed);
          return changed;
        },
      ),
    );
  }

  /// What the conversation shown offers the keyboard and the palette.
  final _conversation = _ConversationHandle();

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
      final client = await connectProviderAgent(acp, agent, decider: store.askPermission, elicitor: store.askQuestion);
      try {
        final opened = await client.openSession(cwd: cwd);
        if (!mounted) throw StateError('Canoa UI closed');
        final local = store.addProviderSession(
          title: title.isEmpty ? agent.label : title,
          agent: agent.label,
          peerId: client.peerId,
          remoteSessionId: opened.id,
        );
        local.agentNames = title.isEmpty;
        store.applyOpened(local, opened);
        store.attachVerifiedClient(local.id, client);
        _ownedClients.add(client);
        if (_hasProfiles) store.noteNoProfile(local);
      } catch (_) {
        await client.close();
        rethrow;
      }
    }
  }

  /// Whether Canoa.cfg sets any profile, so an agent given none of it says so.
  bool get _hasProfiles => config.overlays.isNotEmpty || !config.profile.isEmpty || config.agentProfiles.isNotEmpty;

  /// The profile [agent] starts with in [cwd], from [overlays] (by default all of Canoa.cfg's), once the person has seen it and
  /// chosen its MCP servers; empty where there is none or no profile home. The dialog is shown only for a profile with more
  /// than a model, effort or mode, which are chosen beside Send. Throws when the person does not start it, or its environment
  /// cannot be built.
  Future<(PreparedProfile, Set<String>, List<LayerNote>)> _prepareProfile(String agent, String? cwd, ProfileHome? home,
      {Map<String, OverlaySource>? overlays}) async {
    if (home == null) return (PreparedProfile.empty, const <String>{}, const <LayerNote>[]);
    final layers = await profileLayers(
      overlays: overlays ?? config.overlays, user: config.profile, agents: config.agentProfiles, agent: agent,
      home: home, workingDirectory: cwd, iconLimits: config.store.iconLimits,
    );
    final prepared = await prepareProfile(layers, home: home, lookup: environmentVariable, findProgram: findProgram);
    final approve = prepared.environment.isNotEmpty || prepared.contexts.isNotEmpty || prepared.skills.isNotEmpty ||
        prepared.mcp.isNotEmpty || prepared.problems.isNotEmpty;
    if (!approve) return (prepared, const <String>{}, layers.notes);
    final above = _navigator.currentContext;
    if (above == null || !above.mounted) throw StateError('Canoa UI closed');
    final choice = await showDialog<ProfileStartChoice>(
      context: above,
      builder: (_) => ProfileStartDialog(
        agent: agent, prepared: prepared, harness: presetHarnesses[agent],
        remembered: config.agentMcp[agent] ?? const {}, notes: layers.notes,
      ),
    );
    if (choice == null) throw StateError('its profile was not accepted');
    if (choice.remember) {
      final changed = config.withAgentMcp(agent, {
        for (final server in prepared.mcp)
          server.name: choice.on.contains(server.name) ? (server.confined ? server.digest : true) : false,
      });
      try {
        await _write(changed);
        if (mounted) setState(() => config = changed);
      } on UnsupportedError {
        // Canoa.cfg in web/PWA deployments is read-only.
      } catch (error) {
        // The choices still apply to this session; only keeping them failed.
        _messenger.currentState?.showSnackBar(SnackBar(content: Text('Could not save Canoa.cfg: $error')));
      }
    }
    // What the dialog showed is what goes: its notes are said once, there.
    return (prepared, choice.on, const <LayerNote>[]);
  }

  /// Makes, reads and removes worktrees where a session's agent runs: by git here for one this app runs, through the host for
  /// one it reached at [endpoint]; null where neither can.
  Worktrees? _worktrees(SessionPlace place, {String? endpoint, String? fallback, String? peerId, String? method}) {
    if (place == SessionPlace.local) return localWorktrees();
    if (place != SessionPlace.host && place != SessionPlace.peer) return null;
    final uri = Uri.tryParse(endpoint ?? '');
    final proof = PeerMethod.values.where((each) => each.id == method).firstOrNull;
    if (uri == null || proof == null || peerId == null) return null;
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
      choice = await showDialog<WorktreeChoice>(
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

  /// The sources Settings has: the presets it does not hide, Canoa.cfg's `[agents.NAME]` commands, and the agent entries of
  /// the canoa-host this app runs.
  List<AgentSource> get _sources => [
    for (final preset in agentPresets)
      if (!config.hiddenSources.contains(preset.label)) AgentSource.preset(preset),
    for (final command in config.agents) AgentSource.command(command),
    if (widget.host != null)
      for (final entry in config.hostEntries)
        if (entry.kind == 'agent') AgentSource.hosted(entry.name),
  ];

  /// What each source needs and whether it is here, as last checked.
  Map<AgentSource, SourceCheck> _checks = const {};

  Future<SourceCheck> _check(AgentSource source) async {
    if (widget.checkSource case final check?) return check(source);
    if (source.command case final command?) return checkCommand(command, findProgram);
    final host = widget.host;
    final serving = host?.listening.any((listening) => listening.name == source.hostEntry) ?? false;
    return serving
        ? const SourceCheck()
        : SourceCheck(problem: 'canoa-host is not serving ${source.label} now: open Hosting to see why.');
  }

  /// Whether the sources have been looked for: only once a new session or Settings needs them, since each runs a lookup.
  bool _checked = false;

  Future<void> _checkSources() async {
    _checked = true;
    final sources = _sources;
    final checks = {for (final source in sources) source: await _check(source)};
    if (mounted) setState(() => _checks = checks);
  }

  /// The source and folder chosen last, which a new session starts with.
  AgentSource? _lastSource;
  String? _lastFolder;

  void _noteLast() {
    final session = store.selected;
    if (session != null && session.isDraft && !_checked) unawaited(_checkSources());
    if (session == null || session.source == null) return;
    _lastSource = session.source;
    if (session.isDraft && session.cwd != null) _lastFolder = session.cwd;
    if (session.isDraft) _hint(session);
  }

  /// Folders sessions worked in, the last chosen first: a worktree's stands for its repository.
  List<String> get _recentFolders => {
    ?_lastFolder,
    for (final session in store.sessions)
      if (session.source != null && session.cwd != null) session.worktree?.repo ?? session.cwd!,
    for (final entry in store.ledger?.entries ?? const <LedgerEntry>[])
      if (entry.place == SessionPlace.local || entry.place == SessionPlace.host)
        ?(entry.worktree?.repo ?? entry.cwd),
  }.take(8).toList();

  /// A new session with nothing started, from the source and folder chosen last; one already waiting untouched is shown
  /// instead of another.
  void _newSession() {
    if (!canRunLocalAgents && widget.host == null) {
      store.createPreviewSession();
      return;
    }
    for (final session in store.sessions) {
      if (session.status == SessionStatus.draft && session.messages.isEmpty && session.draftText.trim().isEmpty &&
          session.attachments.isEmpty) {
        store.select(session.id);
        return;
      }
    }
    if (!_checked) unawaited(_checkSources());
    final sources = _sources;
    final source = sources.contains(_lastSource)
        ? _lastSource
        : sources.where((source) => _checks[source]?.ready ?? false).firstOrNull ?? sources.firstOrNull;
    final draft = store.createDraft(source: source, cwd: _lastFolder ?? _recentFolders.firstOrNull ?? defaultWorkingDirectory());
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
    if (source.kind == SourceKind.hosted) {
      store.defaultChoices(draft);
      return;
    }
    unawaited(() async {
      try {
        final layers = await profileLayers(
          overlays: _overlaysFor(draft), user: config.profile, agents: config.agentProfiles, agent: source.label,
          home: platformProfileHome(configPath: _configPath), workingDirectory: draft.cwd, iconLimits: config.store.iconLimits,
        );
        // The person may have chosen another source meanwhile, whose own defaults are on their way.
        if (draft.source != source) return;
        final effective = layers.effective;
        store.defaultChoices(draft, hints: {
          if (effective.model case final model?) OptionRole.model: model.value,
          if (effective.effort case final effort?) OptionRole.effort: effort.value,
          if (effective.mode case final mode?) OptionRole.mode: mode.value,
        });
      } on Object catch (error) {
        debugPrint('Canoa: no profile defaults for ${source.label}: $error');
        if (draft.source == source) store.defaultChoices(draft);
      }
    }());
  }

  /// Canoa.cfg's overlays that [session] starts with.
  Map<String, OverlaySource> _overlaysFor(ChatSession session) {
    final chosen = session.overlays;
    return chosen == null
        ? config.overlays
        : {for (final MapEntry(:key, :value) in config.overlays.entries) if (chosen.contains(key)) key: value};
  }

  /// Starts [draft]'s agent on its first message: the worktree is made, the profile prepared and shown where it has something
  /// to approve, the agent started and its session opened, and the draft bound to it. Anything made is taken back when it
  /// fails, and the draft stays as it was.
  Future<void> _startDraft(ChatSession draft) async {
    final source = draft.source;
    if (source == null) throw StateError('Choose where its agent comes from first');
    final check = _checks[source] ?? await _check(source);
    if (!check.ready) {
      throw StateError(check.describe(source.label, needs: source.command == null ? const [] : commandNeeds(source.command!))!);
    }
    final folder = draft.cwd == null || draft.cwd!.isEmpty ? defaultWorkingDirectory() : draft.cwd;
    if (folder == null) throw StateError('Choose its folder first');
    // Worktree shows, and applies, only for a folder in a git repository.
    final worktree = draft.wantsWorktree && (widget.isGitRepository ?? inGitRepository)(folder);
    final starting = _Starting();
    _inFlight.add(starting);
    try {
      if (source.command case final command?) {
        await _startLocal(draft, command, folder, worktree, starting);
      } else {
        await _startHosted(draft, source.hostEntry!, folder, worktree, starting);
      }
    } finally {
      _inFlight.remove(starting);
    }
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

  Future<void> _startLocal(ChatSession draft, AgentCommand command, String folder, bool worktree, _Starting starting) async {
    final worktrees = worktree ? (widget.worktrees ?? localWorktrees)() : null;
    if (worktree && worktrees == null) throw StateError('This build cannot make a worktree');
    // Named after the source: a branch is kept in the session list, which keeps no words of a message.
    final made = worktrees == null ? null : await worktrees.create(folder, worktreeBranch(command.label));
    starting
      ..worktrees = worktrees
      ..made = made;
    if (_quitting) throw StateError('Canoa is quitting');
    final where = made?.path ?? folder;
    VerifiedAgent? client;
    final ProfiledAgent started;
    final PreparedProfile prepared;
    final List<LayerNote> notes;
    try {
      final home = platformProfileHome(configPath: _configPath);
      final Set<String> on;
      (prepared, on, notes) = await _prepareProfile(command.label, where, home, overlays: _overlaysFor(draft));
      started = await startProfiled(
        command, cwd: where, prepared: prepared, mcpOn: on, files: home is SessionFiles ? home as SessionFiles : null,
        decider: store.askPermission, elicitor: store.askQuestion, start: widget.startAgent ?? startLocalAgent,
      );
      client = started.client;
      starting.client = client;
      if (_quitting) throw StateError('Canoa is quitting');
      final opened = await client.openSession(cwd: where, setup: started.launch.setup);
      if (!mounted || _quitting) throw StateError('Canoa UI closed');
      store.bindDraft(draft, client, opened, cwd: where, worktree: made);
      _ownedClients.add(client);
    } catch (_) {
      await client?.close();
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
    await store.applyProfile(draft, started.launch, prepared, harness: started.harness, notes: notes);
  }

  Future<void> _startHosted(ChatSession draft, String name, String folder, bool worktree, _Starting starting) async {
    final host = widget.host;
    final listening = host?.listening.where((listening) => listening.name == name).firstOrNull;
    if (host == null || listening == null) throw StateError('canoa-host is not serving $name now: open Hosting to see why');
    final method = PeerMethod.values.where((method) => method.id == (listening.method ?? host.peerMethod)).firstOrNull;
    if (method == null) throw StateError('canoa-host did not say which method it proves by');
    final endpoint = Uri.parse(listening.url);
    final worktrees = worktree
        ? _worktrees(SessionPlace.host, endpoint: listening.url, peerId: listening.peer, method: method.id)
        : null;
    if (worktree && worktrees == null) throw StateError('Canoa cannot ask canoa-host for a worktree');
    final made = worktrees == null ? null : await worktrees.create(folder, worktreeBranch(name));
    starting
      ..worktrees = worktrees
      ..made = made;
    if (_quitting) throw StateError('Canoa is quitting');
    final where = made?.path ?? folder;
    VerifiedAgent? client;
    try {
      client = await connectBrowserAcp(
        chatEndpoints(webSocket: listening.url, webTransport: '', preferWebTransport: config.preferWebTransport),
        pinnedPeerId: listening.peer, method: method,
        timeout: Duration(seconds: config.connectTimeout),
        decider: store.askPermission, elicitor: store.askQuestion,
      );
      starting.client = client;
      if (_quitting) throw StateError('Canoa is quitting');
      final opened = await client.openSession(cwd: where);
      if (!mounted || _quitting) throw StateError('Canoa UI closed');
      store.bindDraft(draft, client, opened, cwd: where, worktree: made, place: SessionPlace.host, endpoint: endpoint,
          method: method.id);
      _ownedClients.add(client);
    } catch (_) {
      await client?.close();
      if (made != null) await _unmake(worktrees!, made);
      rethrow;
    }
    if (_hasProfiles) store.noteNoProfile(draft);
  }

  /// Opens a session from the list again: its agent is started or reached again, then asked to open it.
  Future<void> _reopen(LedgerEntry entry) async {
    for (final session in store.sessions) {
      if (session.peerId == entry.peerId && session.remoteSessionId == entry.sessionId) {
        store.select(session.id);
        return;
      }
    }
    final why = entry.whyNotReopened;
    if (why != null) throw StateError(why);
    await _findWorktree(entry);
    final VerifiedAgent client;
    ProfiledAgent? started;
    PreparedProfile? prepared;
    var notes = const <LayerNote>[];
    switch (entry.place) {
      case SessionPlace.local:
        final command = [...agentPresets, ...config.agents].where((agent) => agent.label == entry.entry).firstOrNull;
        if (command == null) throw StateError('No agent named ${entry.entry} is set up here now');
        if (!canRunLocalAgents) throw StateError('This build cannot run agents on this computer');
        final cwd = entry.cwd;
        if (cwd == null) throw StateError('Canoa did not note where this session was, which the agent needs to open it');
        final home = platformProfileHome(configPath: _configPath);
        final Set<String> on;
        (prepared, on, notes) = await _prepareProfile(command.label, cwd, home);
        started = await startProfiled(
          command, cwd: cwd, prepared: prepared, mcpOn: on, files: home is SessionFiles ? home as SessionFiles : null,
          decider: store.askPermission, elicitor: store.askQuestion,
        );
        client = started.client;
      case SessionPlace.host || SessionPlace.peer:
        final endpoint = Uri.tryParse(entry.endpoint ?? '');
        final method = PeerMethod.values.where((method) => method.id == entry.method).firstOrNull;
        if (endpoint == null || method == null) throw StateError('Canoa did not note how to reach that peer');
        final overWebTransport = endpoint.scheme == 'https';
        client = await connectBrowserAcp(
          chatEndpoints(
            webSocket: overWebTransport ? (entry.fallback ?? '') : endpoint.toString(),
            webTransport: overWebTransport ? endpoint.toString() : '',
            preferWebTransport: config.preferWebTransport,
          ),
          pinnedPeerId: entry.peerId, method: method,
          timeout: Duration(seconds: config.connectTimeout),
          decider: store.askPermission, elicitor: store.askQuestion,
        );
      case SessionPlace.browser:
        throw StateError(why ?? 'Reopen it from Earlier conversations');
    }
    try {
      await store.reopen(entry, client, launch: started?.launch, prepared: prepared, harness: started?.harness, notes: notes);
      _ownedClients.add(client);
    } catch (_) {
      await client.close();
      rethrow;
    }
  }

  /// The host's entries come and go as it starts and ends, and a hosted source can run only while it serves one.
  int _serving = -1;
  void _hostChanged() {
    final serving = widget.host?.listening.length ?? 0;
    if (serving == _serving) return;
    _serving = serving;
    if (_checked) unawaited(_checkSources());
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
    final ws = endpoints.webSocket!;
    final wt = target.webTransport.isEmpty ? null : Uri.parse(target.webTransport);
    final pinnedPeerId = target.peerId.toLowerCase();
    final place = widget.host?.peerId?.toLowerCase() == pinnedPeerId ? SessionPlace.host : SessionPlace.peer;
    // The host makes it inside its working roots, before any agent starts, and says where.
    final worktrees = target.worktree
        ? _worktrees(place, endpoint: (wt ?? ws).toString(), fallback: wt == null ? null : ws.toString(),
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
        decider: store.askPermission, elicitor: store.askQuestion,
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
        endpoint: wt ?? ws,
        websocketFallback: wt == null ? null : ws,
        place: place,
        method: target.method.id,
        cwd: cwd,
        worktree: made,
      );
      local.agentNames = target.title.isEmpty;
      store.applyOpened(local, opened);
      store.attachVerifiedClient(local.id, client);
      _ownedClients.add(client);
      if (_hasProfiles) store.noteNoProfile(local);
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
    widget.settingsSignal?.addListener(_requestSettings);
    widget.hostingSignal?.addListener(_requestHosting);
    store = widget.store ??
        SessionStore(recorder: updateRecorderFromEnvironment(), ledger: widget.ledger, fullAccessAgents: config.fullAccess,
            drafts: widget.drafts, embedLimit: config.embedLimit);
    store.starter = _startDraft;
    store.addListener(_noteLast);
    widget.host?.addListener(_hostChanged);
    unawaited(store.restoreDrafts((kind, label) =>
        _sources.where((source) => source.kind.name == kind && source.label == label).firstOrNull));
    // Closing a window to the tray keeps its agents. Quitting does not, and `dispose` does not run when the window is destroyed.
    widget.shell?.beforeQuit = _endAgents;
    _conversation.close = _closeSession;
    _attention = store.attention.listen((needs) => unawaited(_notify(needs.$1, needs.$2)));
    unawaited(widget.notifier?.initialize(_openFromNotification));
    store.addListener(_showWaiting);
    unawaited(_claimContext());
    _contextTimer = Timer.periodic(
      const Duration(seconds: 2), (_) => unawaited(_claimContext()),
    );
  }

  @override
  void didUpdateWidget(CanoaApp oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.config != oldWidget.config) config = widget.config;
  }

  @override
  void dispose() {
    _contextTimer?.cancel();
    unawaited(_attention?.cancel());
    store.removeListener(_showWaiting);
    store.removeListener(_noteLast);
    if (identical(store.starter, _startDraft)) store.starter = null;
    widget.host?.removeListener(_hostChanged);
    widget.settingsSignal?.removeListener(_requestSettings);
    widget.hostingSignal?.removeListener(_requestHosting);
    widget.shell?.beforeQuit = null;
    if (ownsStore) store.dispose();
    for (final client in _ownedClients) {
      unawaited(client.close());
    }
    super.dispose();
  }

  /// What the composer offers, from this app's sources, folders, files and clipboard.
  ComposerKit get _kit => ComposerKit(
    sources: _sources,
    checks: _checks,
    recentFolders: _recentFolders,
    inGitRepository: widget.isGitRepository ?? inGitRepository,
    pickFolder: widget.pickFolder ?? (canRunLocalAgents ? () => getDirectoryPath() : null),
    pickFiles: widget.pickFiles ?? () => openFiles(),
    statFile: widget.statFile ?? (canRunLocalAgents ? localFileSize : null),
    clipboard: widget.clipboard ?? const SystemClipboard(),
    overlays: config.overlays.keys.toList(),
    pasteAsFile: config.pasteAsFile,
    onSourceSettings: () => unawaited(_showSettings(sources: true)),
    onFullAccess: (session) async {
      final above = _navigator.currentContext;
      if (above != null && above.mounted && await confirmFullAccess(above, session)) store.setFullAccess(session, true);
    },
    onEndFullAccess: (session) {
      if (session.fullAccess == FullAccess.agent) {
        unawaited(_endAgentFullAccess(session.agent));
      } else {
        store.setFullAccess(session, false);
      }
    },
    onProfile: (session) {
      final above = _navigator.currentContext;
      final report = session.profile;
      if (above == null || report == null) return;
      unawaited(showDialog<void>(context: above, builder: (_) => ProfileReportDialog(report: report)));
    },
  );

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      navigatorKey: _navigator,
      scaffoldMessengerKey: _messenger,
      title: 'Canoa',
      debugShowCheckedModeBanner: false,
      theme: themeFor(config.look, Brightness.light),
      darkTheme: themeFor(config.look, Brightness.dark),
      themeMode: config.themeMode,
      home: AnimatedBuilder(
        animation: store,
        builder: (context, _) => _CanoaHome(
          store: store, onConnect: _connect, onUseBrowserAgents: _useBrowserAgents, onNewSession: _newSession, onSettings: _showSettings,
          onHosting: widget.host == null ? null : _showHosting, onReopen: _reopen,
          onEndAgentFullAccess: _endAgentFullAccess,
          conversation: _conversation,
          kit: _kit,
        ),
      ),
    );
  }
}

class _CanoaHome extends StatelessWidget {
  const _CanoaHome({
    required this.store, required this.onConnect, required this.onUseBrowserAgents, required this.onNewSession, required this.onSettings,
    this.onHosting, this.onReopen, this.onEndAgentFullAccess, required this.conversation, this.kit = const ComposerKit(),
  });

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;

  /// Opens a new session's empty view.
  final VoidCallback onNewSession;
  final ComposerKit kit;
  final VoidCallback onSettings;
  final VoidCallback? onHosting;
  final Future<void> Function(LedgerEntry)? onReopen;
  final Future<void> Function(String agent)? onEndAgentFullAccess;
  final _ConversationHandle conversation;

  void _connect(BuildContext context) => unawaited(showCanoaConnectDialog(
    context,
    onConnect,
    browserProvider: findAcp(timeout: const Duration(milliseconds: 300)),
    onUseBrowserAgents: onUseBrowserAgents,
  ));

  /// The session [step] places from the one chosen, going round the list.
  void _step(int step) {
    final sessions = store.sessions;
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

  Future<void> _palette(BuildContext context) {
    final session = store.selected;
    return showDialog<void>(
      context: context,
      builder: (_) => CommandPalette(items: [
        PaletteItem('New session', onNewSession, icon: Icons.add_comment_outlined),
        PaletteItem('Connect to an agent', () => _connect(context), icon: Icons.link_rounded),
        if (session != null && conversation.showChanges != null && changesOf(session).isNotEmpty)
          PaletteItem('Show the changes', conversation.showChanges!, icon: Icons.difference_outlined),
        if (session?.status == SessionStatus.working) PaletteItem('Stop the turn', _stop, icon: Icons.stop_circle_outlined),
        PaletteItem('Settings', onSettings, icon: Icons.settings_outlined),
        if (onHosting != null) PaletteItem('Hosting', onHosting!, icon: Icons.dns_outlined),
        PaletteItem('Keyboard shortcuts', () => unawaited(showShortcutHelp(context)), icon: Icons.keyboard_outlined),
        for (final other in store.sessions)
          if (other.id != store.selectedId)
            PaletteItem('Go to ${other.title}', () => store.select(other.id), detail: other.agent, icon: Icons.chat_bubble_outline),
        for (final command in session?.commands ?? const <AgentCommandInfo>[])
          PaletteItem('/${command.name}', () => conversation.insert?.call('/${command.name} '),
              detail: command.description.isEmpty ? 'A command of the agent\'s' : command.description, icon: Icons.terminal),
      ]),
    );
  }

  /// The keyboard's shortcuts, wherever focus is in the window.
  Map<ShortcutActivator, VoidCallback> _shortcuts(BuildContext context) => {
    chord(LogicalKeyboardKey.keyK): () => unawaited(_palette(context)),
    chord(LogicalKeyboardKey.keyN): onNewSession,
    for (final (index, key) in const [
      LogicalKeyboardKey.digit1, LogicalKeyboardKey.digit2, LogicalKeyboardKey.digit3,
      LogicalKeyboardKey.digit4, LogicalKeyboardKey.digit5, LogicalKeyboardKey.digit6,
      LogicalKeyboardKey.digit7, LogicalKeyboardKey.digit8, LogicalKeyboardKey.digit9,
    ].indexed)
      chord(key): () {
        if (index < store.sessions.length) store.select(store.sessions[index].id);
      },
    chord(LogicalKeyboardKey.tab): () => _step(1),
    chord(LogicalKeyboardKey.tab, shift: true): () => _step(-1),
    chord(LogicalKeyboardKey.keyL): () => conversation.focusComposer?.call(),
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
      child: Focus(autofocus: true, child: _layout(context)),
    );
  }

  Widget _layout(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      final narrow = constraints.maxWidth < 730 ||
          Uri.base.queryParameters['view'] == 'panel';

      return Scaffold(
        drawer: narrow
            ? Drawer(child: SafeArea(child: _SessionRail(store: store, onConnect: onConnect, onUseBrowserAgents: onUseBrowserAgents, onNewSession: onNewSession, onSettings: onSettings, onHosting: onHosting, onReopen: onReopen)))
            : null,
        body: SafeArea(
          child: Row(
            children: [
              if (!narrow)
                SizedBox(
                  width: context.look.railWidth,
                  child: _SessionRail(store: store, onConnect: onConnect, onUseBrowserAgents: onUseBrowserAgents, onNewSession: onNewSession, onSettings: onSettings, onHosting: onHosting, onReopen: onReopen),
                ),
              Expanded(
                child: _Conversation(
                  key: ValueKey(store.selectedId ?? 'empty'),
                  store: store,
                  narrow: narrow,
                  onEndAgentFullAccess: onEndAgentFullAccess,
                  handle: conversation,
                  kit: kit,
                ),
              ),
            ],
          ),
        ),
      );
    });
  }
}

class _SessionRail extends StatelessWidget {
  const _SessionRail({
    required this.store, required this.onConnect, required this.onUseBrowserAgents, required this.onNewSession, required this.onSettings,
    this.onHosting, this.onReopen,
  });

  final SessionStore store;
  final Future<void> Function(ConnectTarget) onConnect;
  final UseBrowserAgents onUseBrowserAgents;
  final VoidCallback onNewSession;
  final VoidCallback onSettings;
  final VoidCallback? onHosting;
  final Future<void> Function(LedgerEntry)? onReopen;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: context.look.rail,
      child: Column(children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(18, 18, 10, 12),
          // The wordmark never wraps: where it and the buttons do not fit on one line, the buttons go under it. The full width puts
          // them at the right end when they do fit.
          child: SizedBox(width: double.infinity, child: Wrap(
            alignment: WrapAlignment.spaceBetween,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              Row(mainAxisSize: MainAxisSize.min, children: [
                CanoaMark(color: context.look.accent),
                const SizedBox(width: 10),
                const Text('Canoa', softWrap: false, style: TextStyle(
                  fontSize: 22, fontWeight: FontWeight.w700,
                )),
              ]),
              Row(mainAxisSize: MainAxisSize.min, children: [
                IconButton(
                  tooltip: 'Canoa settings',
                  onPressed: onSettings,
                  icon: const Icon(Icons.settings_outlined),
                ),
                if (onHosting != null)
                  IconButton(
                    tooltip: 'Hosting',
                    onPressed: onHosting,
                    icon: const Icon(Icons.dns_outlined),
                  ),
                IconButton(
                  tooltip: 'Connect to verified agent',
                  onPressed: () => showCanoaConnectDialog(
                    context,
                    onConnect,
                    browserProvider: findAcp(timeout: const Duration(milliseconds: 300)),
                    onUseBrowserAgents: onUseBrowserAgents,
                  ),
                  icon: const Icon(Icons.link_rounded),
                ),
                IconButton(
                  tooltip: 'New session',
                  onPressed: () {
                    onNewSession();
                    if (Scaffold.maybeOf(context)?.isDrawerOpen ?? false) {
                      Navigator.pop(context);
                    }
                  },
                  icon: const Icon(Icons.add_comment_outlined),
                ),
              ]),
            ],
          )),
        ),
        const Divider(height: 1),
        Expanded(
          child: ListView.builder(
            padding: const EdgeInsets.symmetric(vertical: 8),
            itemCount: store.sessions.length + (store.ledger == null ? 0 : 1),
            itemBuilder: (context, index) {
              // The open conversations, then the ledger's sessions that are not open now, by harness.
              if (index == store.sessions.length) {
                return LedgerList(ledger: store.ledger!, store: store, onReopen: onReopen);
              }
              final session = store.sessions[index];
              final active = session.id == store.selectedId;
              return Padding(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                child: ListTile(
                  selected: active,
                  selectedTileColor: context.look.selected,
                  shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
                  leading: session.fullAccess == null
                      ? const Icon(Icons.chat_bubble_outline, size: 21)
                      : Icon(Icons.gpp_maybe_outlined, size: 21, color: Theme.of(context).colorScheme.error),
                  title: Text(session.title, maxLines: 1, overflow: TextOverflow.ellipsis),
                  trailing: _SessionMark(session: session),
                  subtitle: Text(
                    [
                      if (session.fullAccess != null) 'Full access',
                      switch (session.status) {
                        SessionStatus.preview => 'Preview · offline',
                        SessionStatus.draft => 'New',
                        SessionStatus.connecting when session.isDraft => 'Starting',
                        final status => status.name,
                      },
                      if (session.worktree case final worktree?) worktree.branch,
                    ].join(' · '),
                    maxLines: 1,
                  ),
                  onTap: () {
                    store.select(session.id);
                    if (Scaffold.maybeOf(context)?.isDrawerOpen ?? false) {
                      Navigator.pop(context);
                    }
                  },
                ),
              );
            },
          ),
        ),
        const Divider(height: 1),
        Padding(
          padding: const EdgeInsets.all(16),
          child: Text(
            // An agent this app runs is its own child, and only one elsewhere is reached as a peer.
            canRunLocalAgents
                ? 'Agents on other machines need a verified ACP peer.'
                : 'Agent connections and sync require verified ACP peers.',
            style: TextStyle(color: context.look.muted, fontSize: 12),
          ),
        ),
      ]),
    );
  }
}

/// What a start under way has made so far, for quitting to take back.
final class _Starting {
  Worktrees? worktrees;
  SessionWorktree? made;
  VerifiedAgent? client;
}

/// What a session in the list has for the person: something waiting for their answer, a turn running, or a turn that ended
/// since they last looked.
class _SessionMark extends StatelessWidget {
  const _SessionMark({required this.session});

  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    if (session.needsYou) {
      return Tooltip(message: 'Needs you', child: Icon(Icons.front_hand_outlined, size: 18, color: scheme.error));
    }
    if (session.status == SessionStatus.working) {
      return const Tooltip(
        message: 'Working',
        child: SizedBox.square(dimension: 14, child: CircularProgressIndicator(strokeWidth: 2)),
      );
    }
    if (session.unread) {
      return Tooltip(
        message: 'New since you looked',
        child: Container(width: 10, height: 10, decoration: BoxDecoration(color: scheme.primary, shape: BoxShape.circle)),
      );
    }
    return const SizedBox.shrink();
  }
}

/// What the conversation shown offers the keyboard and the palette, set by it while it is shown.
final class _ConversationHandle {
  Object? owner;
  VoidCallback? focusComposer, showChanges;
  ValueChanged<String>? insert;

  /// Ends a conversation as the app does, asking after its worktree; set by the app.
  Future<void> Function(ChatSession)? close;
}

class _Conversation extends StatefulWidget {
  const _Conversation({
    super.key, required this.store, required this.narrow, this.onEndAgentFullAccess, required this.handle,
    this.kit = const ComposerKit(),
  });

  final _ConversationHandle handle;
  final ComposerKit kit;

  final SessionStore store;
  final bool narrow;

  /// Ends full access for an agent, where its setting is what turned it on.
  final Future<void> Function(String agent)? onEndAgentFullAccess;

  @override
  State<_Conversation> createState() => _ConversationState();
}

class _ConversationState extends State<_Conversation> {
  final TextEditingController _input = TextEditingController();
  final ScrollController _scroll = ScrollController();

  /// The composer's focus. Up in an empty composer brings back the last message the person sent here; otherwise Up moves the
  /// cursor as usual, since the composer's own handler sees the key before the text field's shortcuts do.
  late final FocusNode _composerFocus = FocusNode(onKeyEvent: (node, event) {
    if ((event is KeyDownEvent || event is KeyRepeatEvent) && event.logicalKey == LogicalKeyboardKey.arrowUp &&
        _input.text.isEmpty) {
      final last = widget.store.selected?.messages.lastWhere((message) => message.role == MessageRole.user,
          orElse: () => ChatMessage(role: MessageRole.user, text: '', at: DateTime.now()));
      if (last != null && last.text.isNotEmpty) {
        _input.value = TextEditingValue(text: last.text, selection: TextSelection.collapsed(offset: last.text.length));
        return KeyEventResult.handled;
      }
    }
    return _composer.currentState?.onKey(node, event) ?? KeyEventResult.ignored;
  });

  final _composer = GlobalKey<ComposerState>();

  /// The conversation's own selection, which Ctrl+A fills and Ctrl+C copies.
  final _transcript = GlobalKey<SelectableTranscriptState>();

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
    widget.handle
      ..owner = this
      ..focusComposer = _composerFocus.requestFocus
      ..showChanges = () {
        final session = widget.store.selected;
        if (session != null && changesOf(session).isNotEmpty) _showChanges(session);
      }
      ..insert = (text) {
        _input.value = TextEditingValue(text: text, selection: TextSelection.collapsed(offset: text.length));
        _composerFocus.requestFocus();
      };
  }

  /// Whether the transcript keeps its end in view as entries arrive: true until the person scrolls away from the end.
  bool _following = true;

  /// Whether the changes pane is beside the conversation, on a wide window.
  bool _changesOpen = false;

  /// Adds [text] to the message being written, on a line of its own.
  void _addToComposer(String text) {
    final had = _input.text;
    final joined = had.isEmpty || had.endsWith('\n') ? '$had$text' : '$had\n$text';
    _input.value = TextEditingValue(text: joined, selection: TextSelection.collapsed(offset: joined.length));
  }

  /// The changes pane: beside the conversation on a wide window, a sheet over it on a narrow one.
  void _showChanges(ChatSession session) {
    if (!widget.narrow) {
      setState(() => _changesOpen = !_changesOpen);
      return;
    }
    unawaited(showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      useSafeArea: true,
      builder: (sheet) => FractionallySizedBox(
        heightFactor: 0.9,
        child: ListenableBuilder(
          listenable: widget.store,
          builder: (context, _) => ChangesPane(
            store: widget.store,
            session: session,
            onComment: (text) {
              _addToComposer(text);
              Navigator.pop(sheet);
            },
            onClose: () => Navigator.pop(sheet),
          ),
        ),
      ),
    ));
  }

  /// How far from the end, in logical pixels, still counts as at the end.
  static const double _nearEnd = 24;

  bool _onScroll(ScrollNotification notification) {
    // Only the transcript's own scrolling, not a code box's inside an entry.
    if (notification.depth != 0 || (notification is! ScrollUpdateNotification && notification is! ScrollEndNotification)) {
      return false;
    }
    final atEnd = notification.metrics.extentAfter <= _nearEnd;
    if (atEnd != _following) setState(() => _following = atEnd);
    return false;
  }

  /// The transcript grew or shrank: while following, its end comes into view.
  bool _onMetrics(ScrollMetricsNotification notification) {
    if (notification.depth == 0 && _following && notification.metrics.extentAfter > 0) _toEnd();
    return false;
  }

  void _toEnd() {
    if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
  }

  void _jumpToLatest() {
    setState(() => _following = true);
    _toEnd();
  }

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
    _composerFocus.dispose();
    _input.dispose();
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _showEarlier(ChatSession session) async {
    final messenger = ScaffoldMessenger.of(context);
    final picked = await showDialog<AcpSessionInfo>(
      context: context,
      builder: (context) => EarlierConversations(list: (cursor) => widget.store.earlierSessions(session, cursor: cursor)),
    );
    if (picked == null) return;
    try {
      await widget.store.openEarlier(session, picked);
    } catch (error) {
      messenger.showSnackBar(SnackBar(content: Text('That conversation could not be opened: $error')));
    }
  }

  void _send() {
    final text = _input.text;
    final session = widget.store.selected;
    final attached = session?.attachments.isNotEmpty ?? false;
    if (text.trim().isEmpty && !attached) return;
    // A draft's first message stays in the composer until its agent has started and taken it, so a start that fails
    // leaves it to send again.
    final starting = session != null && session.isDraft;
    try {
      if (!starting) _input.clear();
      unawaited(widget.store.sendText(text).then((_) {
        if (starting && mounted && _input.text == text) _input.clear();
      }).catchError((Object error) {
        if (!mounted) return;
        // Not sent, so it is there to change and send again.
        if (_input.text.isEmpty) _input.text = text;
        final started = session?.remoteSessionId != null;
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('${starting && !started ? 'Not started' : 'Message not accepted'}: ${reasonOf(error)}')),
        );
      }));
    } on StateError {
      ScaffoldMessenger.of(context).showSnackBar(const SnackBar(
        content: Text('Connect and verify an ACP peer before sending.'),
      ));
    }
    setState(() => _following = true);
    WidgetsBinding.instance.addPostFrameCallback((_) => _toEnd());
  }

  /// Under the title: the agent, and once a session made here has started, its folder and worktree, which stay as they are.
  static String _subtitle(ChatSession? session) => session == null
      ? 'Multi-session agent chat'
      : [
          session.agent,
          if (!session.isDraft && session.source != null) ?session.cwd,
          if (session.worktree case final worktree?) 'worktree ${worktree.branch}',
        ].join(' · ');

  /// The state a session's header names.
  static String _state(ChatSession? session) => switch (session?.status) {
    null => 'Offline',
    SessionStatus.preview => 'Preview',
    SessionStatus.draft => 'New',
    SessionStatus.connecting when session!.isDraft => 'Starting',
    final status => status.name,
  };

  /// What the header offers for [session], beside closing it: as buttons, or under one menu where two or more would crowd a phone.
  List<Widget> _headerActions(ChatSession? session) {
    final actions = <(String, IconData, VoidCallback)>[
      if (session != null && widget.store.canListEarlier(session))
        ('Earlier conversations', Icons.history, () => unawaited(_showEarlier(session))),
      if (session != null && changesOf(session).isNotEmpty) ('Changes', Icons.difference_outlined, () => _showChanges(session)),
      if (session?.profile case final report?)
        ('Profile', Icons.tune, () => unawaited(showDialog<void>(
          context: context,
          builder: (_) => ProfileReportDialog(report: report),
        ))),
    ];
    if (!widget.narrow || actions.length < 2) {
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
    final session = widget.store.selected;
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
          child: Row(children: [
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
            ..._headerActions(session),
            if (session != null)
              IconButton(
                tooltip: 'Close conversation',
                icon: const Icon(Icons.close),
                onPressed: () => unawaited((widget.handle.close ?? widget.store.close)(session)),
              ),
            // On a phone the state is an icon whose tooltip names it, which leaves the conversation's name room to be read.
            if (widget.narrow)
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
          ]),
        ),
      ),
      if (session?.fullAccess != null)
        FullAccessBanner(
          session: session!,
          onTurnOff: () {
            final agent = session.fullAccess == FullAccess.agent ? widget.onEndAgentFullAccess : null;
            if (agent != null) {
              unawaited(agent(session.agent));
            } else if (session.fullAccess == FullAccess.agent) {
              widget.store.setAgentFullAccess(session.agent, false);
            } else {
              widget.store.setFullAccess(session, false);
            }
          },
        ),
      if (session == null)
        const Expanded(child: Center(child: Text('Create a conversation to begin.')))
      else if (session.isDraft)
        Expanded(
          child: _dropping(session, NewSessionView(
            store: widget.store,
            session: session,
            kit: widget.kit,
            composer: _composerFor(session, centred: true),
          )),
        )
      else ...[
        Expanded(
          child: _dropping(session, Stack(children: [
            NotificationListener<ScrollMetricsNotification>(
              onNotification: _onMetrics,
              child: NotificationListener<ScrollNotification>(
                onNotification: _onScroll,
                child: SelectableTranscript(
                  key: _transcript,
                  session: session,
                  child: ListView.builder(
                    controller: _scroll,
                    padding: EdgeInsets.symmetric(
                      horizontal: widget.narrow ? 12 : 40,
                      vertical: context.look.padding + 10,
                    ),
                    itemCount: session.messages.length,
                    itemBuilder: (context, index) {
                      return TranscriptEntry(message: session.messages[index], session: session);
                    },
                  ),
                ),
              ),
            ),
            if (!_following)
              Positioned(
                left: 0,
                right: 0,
                bottom: 12,
                child: Center(
                  child: IconButton.filledTonal(
                    tooltip: 'Jump to latest',
                    onPressed: _jumpToLatest,
                    icon: const Icon(Icons.arrow_downward_rounded),
                  ),
                ),
              ),
          ])),
        ),
        if (session.plan.isNotEmpty) SelectionArea(child: PlanPanel(entries: session.plan)),
        if (session.queue.isNotEmpty) QueuePanel(store: widget.store, session: session),
        if (session.pendingPermissions.isNotEmpty)
          PermissionCard(store: widget.store, session: session),
        if (session.pendingQuestions.isNotEmpty) QuestionCard(store: widget.store, session: session),
        SessionControls(session: session),
        Container(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 20),
          child: Center(
            heightFactor: 1,
            child: ConstrainedBox(
              constraints: BoxConstraints(maxWidth: context.look.messageWidth + 100),
              child: Column(mainAxisSize: MainAxisSize.min, children: [
                _composerFor(session),
                if (!canRunLocalAgents || session.place != SessionPlace.local) ...[
                  const SizedBox(height: 8),
                  Text('Remote sessions require verified peer authentication',
                    style: TextStyle(fontSize: 11, color: context.look.muted)),
                ],
              ]),
            ),
          ),
        ),
      ],
    ]);
    if (widget.narrow || !_changesOpen || session == null || changesOf(session).isEmpty) return conversation;
    return Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Expanded(child: conversation),
      const VerticalDivider(width: 1),
      SizedBox(
        width: 420,
        child: SelectionArea(
          child: ChangesPane(
            store: widget.store,
            session: session,
            onComment: _addToComposer,
            onClose: () => setState(() => _changesOpen = false),
          ),
        ),
      ),
    ]);
  }

  /// The composer of [session]: in the middle of a new session's view when [centred], else docked under the conversation.
  Widget _composerFor(ChatSession session, {bool centred = false}) => Composer(
    key: _composer,
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
