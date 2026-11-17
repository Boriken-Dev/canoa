/// What a session may hand a task to, and the request that hands one over: the host's `_canoa/delegate/targets` answer and the
/// body of `_canoa/delegate/start`.
library;

/// One model a source lists, with the efforts it takes.
final class TargetModel {
  const TargetModel(this.id, this.name, this.efforts);

  final String id;
  final String name;
  final List<String> efforts;
}

/// A profile an agent offers, with the loadouts of it a task may name.
final class TargetProfile {
  const TargetProfile(this.name, this.loadouts);

  final String name;
  final List<String> loadouts;
}

/// One agent a session may hand a task to, as the host lists it. The host lists accounts by name only, never a credential, and
/// only those the session's connection may use.
final class DelegateTarget {
  const DelegateTarget({
    required this.source,
    required this.title,
    required this.remote,
    required this.accounts,
    required this.ready,
    required this.missing,
    this.harness,
    this.defaultFolder,
    this.modelsKnown,
    this.models = const [],
    this.profiles = const [],
  });

  /// Reads one entry of the host's `targets`; null when it is not one.
  static DelegateTarget? parse(Object? raw) {
    if (raw is! Map || raw['source'] is! String) return null;
    final workspace = raw['workspace'];
    final models = raw['models'];
    final listed = models is Map && models['list'] is List ? models['list'] as List : const [];
    return DelegateTarget(
      source: raw['source'] as String,
      title: raw['title'] is String ? raw['title'] as String : raw['source'] as String,
      harness: raw['harness'] is String ? raw['harness'] as String : null,
      remote: raw['remote'] == true,
      accounts: [if (raw['accounts'] is List) for (final account in raw['accounts'] as List) if (account is String) account],
      ready: raw['ready'] != false,
      missing: [if (raw['missing'] is List) for (final program in raw['missing'] as List) if (program is String) program],
      defaultFolder: workspace is Map && workspace['default'] is String ? workspace['default'] as String : null,
      modelsKnown: models is Map ? models['known'] == true : null,
      profiles: [
        if (raw['profiles'] is List)
          for (final profile in raw['profiles'] as List)
            if (profile is Map && profile['name'] is String)
              TargetProfile(profile['name'] as String, [if (profile['loadouts'] is List) for (final loadout in profile['loadouts'] as List) if (loadout is String) loadout]),
      ],
      models: [
        for (final model in listed)
          if (model is Map && model['id'] is String)
            TargetModel(
              model['id'] as String,
              model['name'] is String ? model['name'] as String : model['id'] as String,
              [if (model['efforts'] is List) for (final effort in model['efforts'] as List) if (effort is String) effort],
            ),
      ],
    );
  }

  final String source, title;
  final String? harness;

  /// Whether it runs on another machine, which needs a folder there and takes no choice of model, effort or account.
  final bool remote;
  final List<String> accounts;

  /// Whether what it needs is here; [missing] names the programs that are not.
  final bool ready;
  final List<String> missing;

  /// The folder a task for it works in when none is named, for an agent of this host.
  final String? defaultFolder;

  /// Whether the host could list its models; null until they were asked for.
  final bool? modelsKnown;
  final List<TargetModel> models;

  /// The profiles its entry offers; a task may name one of them and one of its loadouts.
  final List<TargetProfile> profiles;

  /// The host's name for where it runs, as the person reads it.
  String get hostLabel => remote ? 'Another machine' : 'This host';
}

/// The session's targets with what the person set for them.
final class DelegateTargets {
  const DelegateTargets(this.targets, {this.defaultSource, this.maxRunning, this.sources, this.offered = const [], this.context = const [], this.contextBytes});

  /// Reads the host's answer; null when it is not one.
  static DelegateTargets? parse(Object? raw) {
    if (raw is! Map || raw['targets'] is! List) return null;
    return DelegateTargets(
      [for (final each in raw['targets'] as List) ?DelegateTarget.parse(each)],
      defaultSource: raw['default'] is String ? raw['default'] as String : null,
      maxRunning: raw['maxRunning'] is int ? raw['maxRunning'] as int : null,
      sources: raw['sources'] is List ? [for (final name in raw['sources'] as List) if (name is String) name] : null,
      offered: [
        if (raw['offered'] is List)
          for (final each in raw['offered'] as List)
            if (each is Map && each['source'] is String) (each['source'] as String, each['title'] is String ? each['title'] as String : each['source'] as String),
      ],
      context: [if (raw['context'] is List) for (final kind in raw['context'] as List) if (kind is String) kind],
      contextBytes: raw['contextBytes'] is int ? raw['contextBytes'] as int : null,
    );
  }

  final List<DelegateTarget> targets;
  final String? defaultSource;
  final int? maxRunning;

  /// The agents the person narrowed the session to; null when they have not.
  final List<String>? sources;

  /// Every agent the connection may use, by name and title, narrowed or not: what the person's setup chooses among.
  final List<(String, String)> offered;

  /// What the session's agent may attach to a task by its own call (`messages`, `files`); none until the person lets it.
  final List<String> context;

  /// The most of a conversation's turns, or of its files, a task may be handed.
  final int? contextBytes;
}

/// What handing the conversation's turns or touched files on would give a task, as the host shows it before anything goes. A task
/// started with [id] is given exactly [text].
final class ContextPreview {
  const ContextPreview({required this.id, required this.kind, required this.text, required this.items, required this.skipped, required this.truncated});

  static ContextPreview? parse(Object? raw) {
    if (raw is! Map || raw['contextId'] is! String || raw['text'] is! String) return null;
    return ContextPreview(
      id: raw['contextId'] as String,
      kind: raw['kind'] is String ? raw['kind'] as String : '',
      text: raw['text'] as String,
      items: [
        if (raw['items'] is List)
          for (final item in raw['items'] as List)
            if (item is Map && item['name'] is String) (item['name'] as String, item['bytes'] is int ? item['bytes'] as int : 0),
      ],
      skipped: [if (raw['skipped'] is List) for (final line in raw['skipped'] as List) if (line is String) line],
      truncated: raw['truncated'] == true,
    );
  }

  final String id, kind, text;

  /// Each turn or file in it, by name, with its size in bytes.
  final List<(String, int)> items;

  /// What was left out, and why.
  final List<String> skipped;
  final bool truncated;
}

/// A task the person asks the host to start, as `_canoa/delegate/start` carries it. [requestId] names this one request: the host
/// answers a second request with the same id by the task the first made, so sending it again after an answer was lost never
/// starts the work twice.
final class DelegateRequest {
  const DelegateRequest({
    required this.requestId,
    required this.source,
    required this.task,
    this.title,
    this.folder,
    this.workspace,
    this.model,
    this.effort,
    this.account,
    this.profile,
    this.loadout,
    this.context,
    this.contextId,
  });

  final String requestId, source, task;
  final String? title, folder, workspace, model, effort, account, profile, loadout;

  /// `messages` or `files`, with the [contextId] of the preview the person was shown; both or neither.
  final String? context, contextId;

  Map<String, Object?> params(String sessionId) => {
    'sessionId': sessionId,
    'requestId': requestId,
    'source': source,
    'task': task,
    'title': ?title,
    'folder': ?folder,
    'workspace': ?workspace,
    'model': ?model,
    'effort': ?effort,
    'account': ?account,
    'profile': ?profile,
    'loadout': ?loadout,
    'context': ?context,
    'contextId': ?contextId,
  };
}

/// What became of asking the host to start a task.
sealed class DelegateOutcome {
  const DelegateOutcome();
}

/// The host made the task, or had made it for this request already ([repeated]).
final class DelegateStarted extends DelegateOutcome {
  const DelegateStarted(this.taskId, {this.repeated = false});
  final String taskId;
  final bool repeated;
}

/// The host refused it, and says why: a choice it does not offer is named.
final class DelegateRefused extends DelegateOutcome {
  const DelegateRefused(this.message);
  final String message;
}

/// The host did not answer in time: the task may or may not have been made. The app never sends it again by itself.
final class DelegateUncertain extends DelegateOutcome {
  const DelegateUncertain();
}
