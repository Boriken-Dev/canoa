/// What an agent's A2UI views may cost, against an agent (or an overlay's file) that sends far more than a person could read. Each
/// is a setting with a default and no floor; a surface that goes past one is cut and says so, and the limits never reach the
/// person's own access to anything.
final class A2uiLimits {
  const A2uiLimits({
    this.maxNodes = 400,
    this.maxDepth = 12,
    this.maxRows = 200,
    this.maxDataBytes = 256 * 1024,
    this.maxMessageBytes = 512 * 1024,
    this.maxSurfaces = 8,
    this.maxUpdatesPerSecond = 40,
    this.maxText = 20000,
    this.maxErrorReports = 3,
  });

  /// The most components one surface keeps; a component past it is dropped.
  final int maxNodes;

  /// The deepest a tree is drawn; a branch past it shows a notice.
  final int maxDepth;

  /// The most rows a list template draws from its data.
  final int maxRows;

  /// The most a surface's data model may hold, as JSON; an update that would pass it is refused.
  final int maxDataBytes;

  /// The most one message may hold, as JSON.
  final int maxMessageBytes;

  /// The most surfaces one session keeps; a new one past it is refused.
  final int maxSurfaces;

  /// The most messages taken in one second; the rest of that second's are dropped.
  final int maxUpdatesPerSecond;

  /// The longest text a component draws.
  final int maxText;

  /// The most problems of one surface told back to the agent, so a surface that cannot be drawn never starts a loop of turns.
  final int maxErrorReports;
}
