/// The `_meta` key of a `usage_update` under which Claude's adapter may report what the context window holds, by kind: the
/// shape of the Claude Agent SDK's context usage, of which only each category's `name`, `tokens` and `isDeferred` are read.
const claudeContextMeta = '_claude/contextUsage';

/// One kind of thing in an agent's context window and how many tokens it takes there. A [deferred] kind is loaded only when
/// used, so its tokens are not part of what the window holds now.
typedef ContextKind = ({String label, int tokens, bool deferred});

/// The kinds a `usage_update` reports, in the order the agent gave them; empty when it reports none, which is how every agent
/// but a Claude adapter that sends [claudeContextMeta] does today. Entries that are not a name with a whole token count are
/// skipped, and nothing is added for a kind the agent did not name.
List<ContextKind> contextKindsOf(Map<String, dynamic> update) {
  final meta = update['_meta'];
  final report = meta is Map ? meta[claudeContextMeta] : null;
  final categories = report is Map ? report['categories'] : null;
  if (categories is! List) return const [];
  final kinds = <ContextKind>[];
  for (final each in categories) {
    if (each is! Map) continue;
    final name = each['name'], tokens = each['tokens'];
    if (name is! String || name.trim().isEmpty || tokens is! int || tokens < 0) continue;
    kinds.add((label: name.trim(), tokens: tokens, deferred: each['isDeferred'] == true));
  }
  return kinds;
}

/// Whether [kind] names the window's unused room, which the inspector draws as the bar's empty part rather than a segment.
bool isFreeSpace(ContextKind kind) => kind.label.toLowerCase() == 'free space';
