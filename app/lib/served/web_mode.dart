/// Where the person sits in front of a served page, as the host that serves it says (`canoa.json`'s `web`): at the host's own
/// computer, or somewhere else with the browser as the interface. Features branch on this, not on desktop or web.
enum WebMode {
  /// The browser is at the host's computer: the host starts an editor or file manager there.
  local,

  /// The browser is somewhere else: files go through the app, and an editor connects over SSH.
  remote;

  /// The mode a host's word names, or null for anything else (an older host says none).
  static WebMode? named(Object? name) => values.where((mode) => mode.name == name).firstOrNull;

  /// [said] when the host gave one, else local for a page reached over loopback and remote for any other name.
  static WebMode of(WebMode? said, Uri page) => said ?? (isLoopbackHost(page.host) ? local : remote);
}

/// Whether [host] (as a URL spells it, without brackets) is this computer's own loopback.
bool isLoopbackHost(String host) {
  final lower = host.toLowerCase();
  if (lower == 'localhost' || lower == '::1') return true;
  final parts = lower.split('.');
  return parts.length == 4 && parts.first == '127' && parts.every((part) => int.tryParse(part) != null);
}
