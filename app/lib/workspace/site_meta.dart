import 'package:coqui_acp/coqui_acp.dart' show AcpSessionSetup;

/// The `_meta` key of a webpage's session request. The host reads the site from it, makes the session's scratch folder under its
/// own Documents folder and says which one in its answer under the same key.
const siteMeta = 'dev.boriken.canoa/site';

/// The `_meta` key of a `session/new` that asks the host to record the session nowhere and list it nowhere: a side chat's.
const ephemeralMeta = 'dev.boriken.canoa/ephemeral';

/// [base], with the mark that keeps the session out of the host's record and lists when [ephemeral].
AcpSessionSetup ephemeralSetup(AcpSessionSetup base, {required bool ephemeral}) => !ephemeral
    ? base
    : AcpSessionSetup(
        mcpServers: base.mcpServers,
        systemPrompt: base.systemPrompt,
        additionalDirectories: base.additionalDirectories,
        meta: {...?base.meta, ephemeralMeta: true},
      );

/// The `cwd` a webpage's `session/new` carries: ACP wants one and the page has none to name. The host replaces it.
const siteStandInFolder = '/';

/// What a session for [site] (a host name as `siteFqdn` writes it) is opened with, or opened again with: the host checks the name
/// and makes or finds the folder. The app is the person's own, which the host is told so that it does not hold the app's sessions to
/// what it holds a webpage's (a page's own request reaches the host by way of the extension, which removes this mark).
AcpSessionSetup siteSetup(String site) => AcpSessionSetup(meta: {siteMeta: {'site': site, 'own': true}});

/// The site name a served page's own chats go under: the host makes `<Documents>/Canoa/scratch/app/<id>` for each.
const servedSite = 'app';

/// The folder the host made for a webpage's session and its id, from the answer to `session/new`; null when it said none.
({String id, String folder})? madeFolder(Map<String, dynamic> answer) {
  final meta = answer['_meta'];
  final made = meta is Map<String, dynamic> ? meta[siteMeta] : null;
  if (made is! Map<String, dynamic>) return null;
  final id = made['id'], folder = made['folder'];
  return id is String && id.isNotEmpty && folder is String && folder.isNotEmpty ? (id: id, folder: folder) : null;
}
