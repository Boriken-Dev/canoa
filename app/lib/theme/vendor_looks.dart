import 'package:material_ui/material_ui.dart';

import '../brand.dart';
import '../l10n/text_pack.dart';
import 'look.dart';

/// The conversation tokens a look sets apart from the shell's.
typedef ChatTokens = ({Color user, Color notice, Color added, Color removed, Color code});

/// The families the host lists (`looks/list`): one document per agent that carries a look, each with colour tokens for both
/// brightnesses. The app keeps Canoa's own look and the terminal's built in and draws every other family from these, so until a host
/// answers (and for a host that lists no such family) a family a config names draws in Canoa's look.
///
/// A token the document lacks, or holds in a form this app does not read, takes the Canoa look's value.
final class VendorLooks extends ChangeNotifier {
  final Map<LookId, _Document> _documents = {};
  // One look per family and brightness until the answer changes: a theme compares its look by identity.
  final Map<(LookId, Brightness), CanoaLook> _shells = {};
  bool _answered = false;
  String? _problem;
  int _revision = 0;

  /// How many times the set of looks changed, for a cache keyed on it.
  int get revision => _revision;

  /// Whether the host has answered, whatever it listed.
  bool get answered => _answered;

  /// The families the host listed, in the order it listed them, each with its label.
  List<LookId> get families => [for (final document in _documents.values) document.id];

  /// Whether the host listed [id]'s family.
  bool has(LookId id) => _documents.containsKey(id);

  /// Why a host-supplied family draws in Canoa's look instead, or null when it does not (or [id] is built in).
  String? fallbackReason(LookId id) {
    if (id.isBuiltIn || has(id)) return null;
    if (_problem != null) return "The ${id.label} family comes from the host, which could not be asked ($_problem). ${Brand.name}'s look is shown.";
    if (!_answered) return "The ${id.label} family comes from the host, which has not answered yet. ${Brand.name}'s look is shown until it does.";
    return "This host lists no ${id.label} family. ${Brand.name}'s look is shown.";
  }

  /// The provider whose conversations [id]'s family belongs to, as the host's document names it, or null.
  String? vendorOf(LookId id) => _documents[id]?.vendor;

  /// The family whose vendor is [vendor], among those the host listed.
  LookId? familyOfVendor(String vendor) {
    for (final entry in _documents.entries) {
      if (entry.value.vendor == vendor) return entry.key;
    }
    return null;
  }

  /// Takes the host's answer to `looks/list`. A document that is not an object with a known `id` is skipped.
  void adopt(Object? answer) {
    final next = <LookId, _Document>{};
    final list = answer is Map ? answer['looks'] : null;
    if (list is List) {
      for (final item in list) {
        final document = _Document.parse(item);
        if (document != null) next[document.id] = document;
      }
    }
    _documents
      ..clear()
      ..addAll(next);
    _answered = true;
    _problem = null;
    _changed();
  }

  /// Notes that the host could not be asked, with [reason].
  void failed(String reason) {
    _problem = reason;
    _changed();
  }

  /// Forgets every document, as before a host has answered.
  void clear() {
    _documents.clear();
    _answered = false;
    _problem = null;
    _changed();
  }

  void _changed() {
    _revision++;
    _shells.clear();
    notifyListeners();
  }

  /// [id]'s shell tokens in [brightness], each one the document lacks taken from [fallback]; null when it has no such family.
  CanoaLook? shell(LookId id, Brightness brightness, CanoaLook fallback) {
    final kept = _shells[(id, brightness)];
    if (kept != null) return kept;
    final built = _shell(id, brightness, fallback);
    if (built != null) _shells[(id, brightness)] = built;
    return built;
  }

  CanoaLook? _shell(LookId id, Brightness brightness, CanoaLook fallback) {
    final tokens = _documents[id]?.of(brightness);
    if (tokens == null) return null;
    final muted = tokens.color('muted') ?? fallback.muted;
    final page = tokens.color('page') ?? fallback.page;
    return CanoaLook(
      id: id,
      page: page,
      rail: tokens.color('rail') ?? fallback.rail,
      bar: tokens.color('bar') ?? fallback.bar,
      selected: tokens.color('selected') ?? fallback.selected,
      surface: tokens.color('surface') ?? fallback.surface,
      raised: tokens.color('raised') ?? fallback.raised,
      border: tokens.color('border') ?? Color.alphaBlend(muted.withValues(alpha: 0.3), page),
      text: tokens.color('text') ?? fallback.text,
      muted: muted,
      accent: tokens.color('accent') ?? fallback.accent,
      onAccent: tokens.color('onAccent') ?? fallback.onAccent,
      success: tokens.color('success') ?? fallback.success,
      warning: tokens.color('warning') ?? fallback.warning,
      danger: tokens.color('danger') ?? fallback.danger,
      radius: tokens.number('radius') ?? fallback.radius,
      controlRadius: tokens.number('controlRadius') ?? fallback.controlRadius,
      railWidth: tokens.number('railWidth') ?? fallback.railWidth,
      words: _documents[id]?.text,
    );
  }

  /// [id]'s conversation tokens in [brightness], each one the document lacks taken from [fallback]; [fallback] whole when the host
  /// listed no such family.
  ChatTokens chat(LookId id, Brightness brightness, ChatTokens fallback) {
    final tokens = _documents[id]?.of(brightness)?.table('chat');
    if (tokens == null) return fallback;
    return (
      user: tokens.color('user') ?? fallback.user,
      notice: tokens.color('notice') ?? fallback.notice,
      added: tokens.color('added') ?? fallback.added,
      removed: tokens.color('removed') ?? fallback.removed,
      code: tokens.color('code') ?? fallback.code,
    );
  }
}

/// The families of the host this app reaches, which [lookFor] and [chatLookFor] consult.
final VendorLooks vendorLooks = VendorLooks();

final class _Document {
  _Document(this.id, this.vendor, this._light, this._dark, this.text);

  final LookId id;
  final String vendor;

  /// What the document sets of the words and symbols, from its `text` table.
  final TextPack? text;
  final _Tokens? _light, _dark;

  _Tokens? of(Brightness brightness) => brightness == Brightness.dark ? _dark : _light;

  static _Document? parse(Object? item) {
    if (item is! Map) return null;
    final id = item['id'];
    final vendor = item['vendor'];
    final label = item['label'];
    final look = id is String ? LookId.named(id, label: label is String ? label : null) : null;
    // Canoa's and the terminal's looks are built in: a host does not replace them.
    if (look == null || look.isBuiltIn || vendor is! String) return null;
    return _Document(look, vendor, _Tokens.parse(item['light']), _Tokens.parse(item['dark']), TextPack.parse(item['text']));
  }
}

final class _Tokens {
  _Tokens(this._values);

  final Map<Object?, Object?> _values;

  static _Tokens? parse(Object? value) => value is Map ? _Tokens(value) : null;

  _Tokens? table(String name) => parse(_values[name]);

  /// `#RRGGBB`, or null for anything else.
  Color? color(String name) {
    final value = _values[name];
    if (value is! String || !RegExp(r'^#[0-9a-fA-F]{6}$').hasMatch(value)) return null;
    return Color(0xFF000000 | int.parse(value.substring(1), radix: 16));
  }

  double? number(String name) {
    final value = _values[name];
    return value is num && value.isFinite && value >= 0 ? value.toDouble() : null;
  }
}
