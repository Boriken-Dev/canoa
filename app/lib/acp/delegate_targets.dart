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

  /// The host's name for where it runs, as the person reads it.
  String get hostLabel => remote ? 'Another machine' : 'This host';
}

/// The session's targets with what the person set for them.
final class DelegateTargets {
  const DelegateTargets(this.targets, {this.defaultSource, this.maxRunning});

  /// Reads the host's answer; null when it is not one.
  static DelegateTargets? parse(Object? raw) {
    if (raw is! Map || raw['targets'] is! List) return null;
    return DelegateTargets(
      [for (final each in raw['targets'] as List) ?DelegateTarget.parse(each)],
      defaultSource: raw['default'] is String ? raw['default'] as String : null,
      maxRunning: raw['maxRunning'] is int ? raw['maxRunning'] as int : null,
    );
  }

  final List<DelegateTarget> targets;
  final String? defaultSource;
  final int? maxRunning;
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
  });

  final String requestId, source, task;
  final String? title, folder, workspace, model, effort, account, profile, loadout;

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
