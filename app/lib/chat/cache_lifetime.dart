/// How long an agent's prompt cache should live, which canoa.cfg's `cache_lifetime` and `canoa agent --cache-lifetime` name.
enum CacheLifetime {
  /// The harness's own.
  harness('default', 'Default', 'the harness\'s own'),

  /// About five minutes: for one-shot and background runs.
  short('short', 'Short', 'about 5 minutes'),

  /// About an hour: for a session someone is active in.
  long('long', 'Long', 'about an hour');

  const CacheLifetime(this.id, this.label, this.about);

  /// The name the host and canoa.cfg know it by.
  final String id;
  final String label;

  /// What it comes to, for a menu.
  final String about;

  /// A chat session asks this unless the person chooses or the agent's entry in canoa.cfg says otherwise.
  static const chat = CacheLifetime.long;

  /// The lifetime named [id], or null for anything else.
  static CacheLifetime? named(Object? id) => values.where((each) => each.id == id).firstOrNull;
}

/// The `_meta` key of `initialize` through which the chat tells a host how long a session's prompt cache should live.
const cacheLifetimeMeta = 'dev.boriken.canoa/cache_lifetime';

/// The value under [cacheLifetimeMeta]: [lifetime], and whether the person [chosen] it for this session, which wins over the
/// entry's `cache_lifetime`; otherwise it is the chat's default, which the entry replaces.
Map<String, Object?> cacheLifetimeRequest(CacheLifetime lifetime, {required bool chosen}) =>
    {'lifetime': lifetime.id, 'chosen': chosen};
