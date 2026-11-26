import 'package:material_ui/material_ui.dart';

import '../selectable.dart';
import 'overlay_page.dart' show oneLine;
import 'store_icon.dart';

/// The width of the Store's body from which cards sit two to a row.
const twoColumnsFrom = 900.0;

/// A scrolling list of [header] widgets, then [cards] in rows of two when the body is wide and of one when it is narrow, then
/// [footer]. Rows are built as they scroll in, so a catalog of thousands of entries costs what is on screen.
class CardList extends StatelessWidget {
  const CardList({super.key, this.header = const [], this.cards = const [], this.footer = const [], this.evenFrom});

  /// Set for cards that are all one size: they sit in as many equal columns as fit this many logical pixels each, so the grid
  /// reflows by width. Null keeps one column, or two from [twoColumnsFrom].
  final double? evenFrom;

  final List<Widget> header;
  final List<Widget> cards;
  final List<Widget> footer;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, box) {
      final even = evenFrom;
      final columns = even != null ? ((box.maxWidth - 32 + 8) / (even + 8)).floor().clamp(1, 6) : (box.maxWidth >= twoColumnsFrom ? 2 : 1);
      final rows = (cards.length + columns - 1) ~/ columns;
      return ListView.builder(
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 16),
        itemCount: header.length + rows + footer.length,
        itemBuilder: (context, index) {
          if (index < header.length) return header[index];
          index -= header.length;
          if (index >= rows) return footer[index - rows];
          if (columns == 1) return cards[index];
          final first = index * columns;
          return Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
            for (var at = 0; at < columns; at++) ...[
              if (at > 0) const SizedBox(width: 8),
              Expanded(child: first + at < cards.length ? cards[first + at] : const SizedBox.shrink()),
            ],
          ]);
        },
      );
    },
  );
}

/// The height of every [CompactCard]: one size for all, so a grid of them is even.
const compactCardHeight = 72.0;

/// The narrowest a [CompactCard] is laid out: the columns of a grid of them are this wide, or wider.
const compactCardWidth = 340.0;

/// An installed item's card in a list: its icon, its name, one line about it (cut short with an ellipsis) and, when it has one, its
/// on/off switch. Every other fact and action is on the page the card opens.
class CompactCard extends StatelessWidget {
  const CompactCard({super.key, required this.leading, required this.title, this.summary, this.trailing, required this.onOpen});

  final Widget leading;
  final String title;
  final String? summary;

  /// Its on/off switch, when it has one.
  final Widget? trailing;
  final VoidCallback onOpen;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final quiet = text.bodySmall?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant);
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: InkWell(
        onTap: onOpen,
        borderRadius: BorderRadius.circular(12),
        child: SizedBox(
          height: compactCardHeight,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12),
            child: Row(children: [
              leading,
              const SizedBox(width: 12),
              Expanded(
                child: Column(mainAxisAlignment: MainAxisAlignment.center, crossAxisAlignment: CrossAxisAlignment.start, children: [
                  Text(oneLine(title), style: text.titleSmall, maxLines: 1, overflow: TextOverflow.ellipsis),
                  if (summary != null && summary!.isNotEmpty)
                    Text(oneLine(summary!), key: const Key('card-summary'), style: quiet, maxLines: 1, overflow: TextOverflow.ellipsis),
                ]),
              ),
              if (trailing != null) ...[const SizedBox(width: 8), trailing!],
            ]),
          ),
        ),
      ),
    );
  }
}

/// The page an installed item's card opens: everything about it, rebuilt whenever [listenable] changes, and closed once [present]
/// says the item is gone (it was removed from the page).
Future<void> showInstalledPage(BuildContext context,
        {required String title, required Listenable listenable, required WidgetBuilder builder, required bool Function() present}) =>
    Navigator.of(context).push<void>(selectablePage(
      builder: (_) => _InstalledPage(title: title, listenable: listenable, builder: builder, present: present),
    ));

class _InstalledPage extends StatefulWidget {
  const _InstalledPage({required this.title, required this.listenable, required this.builder, required this.present});

  final String title;
  final Listenable listenable;
  final WidgetBuilder builder;
  final bool Function() present;

  @override
  State<_InstalledPage> createState() => _InstalledPageState();
}

class _InstalledPageState extends State<_InstalledPage> {
  bool _closing = false;

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: Text(oneLine(widget.title), key: const Key('installed-page-title'))),
    body: ListenableBuilder(
      listenable: widget.listenable,
      builder: (context, _) {
        if (!widget.present()) {
          if (!_closing) {
            _closing = true;
            WidgetsBinding.instance.addPostFrameCallback((_) {
              if (mounted) Navigator.of(context).maybePop();
            });
          }
          return const SizedBox.shrink();
        }
        return ListView(
          key: const Key('installed-page'),
          padding: const EdgeInsets.fromLTRB(16, 8, 16, 16),
          children: [widget.builder(context)],
        );
      },
    ),
  );
}

/// An installed item's full card: an icon, a title, lines under it, and what the person can do.
class LoadoutCard extends StatelessWidget {
  const LoadoutCard({super.key, required this.leading, required this.title, this.lines = const [], this.provenance,
      this.trailing, this.footer, this.onTap, this.badge});

  final Widget leading;
  final String title;
  final List<String> lines;

  /// Where it comes from, in one line, under the others.
  final String? provenance;
  final Widget? trailing;
  final Widget? footer;
  final VoidCallback? onTap;

  /// A mark beside the title, such as "Update available".
  final Widget? badge;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
              leading,
              const SizedBox(width: 12),
              Expanded(
                child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                  if (badge == null)
                    Text(oneLine(title), style: text.titleSmall)
                  else
                    Wrap(spacing: 8, runSpacing: 2, crossAxisAlignment: WrapCrossAlignment.center, children: [
                      Text(oneLine(title), style: text.titleSmall),
                      badge!,
                    ]),
                  for (final line in lines) Text(oneLine(line), style: text.bodyMedium),
                  if (provenance != null)
                    Text(oneLine(provenance!), key: const Key('provenance'),
                        style: text.bodySmall?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant)),
                ]),
              ),
              if (trailing != null) ...[const SizedBox(width: 8), trailing!],
            ]),
            if (footer != null) ...[const SizedBox(height: 8), footer!],
          ]),
        ),
      ),
    );
  }
}

/// A Discover card: icon, name, a two-line summary, where it comes from, what it needs, whether it is here, and one labelled
/// action. The card opens the entry's details; the action goes on from there, or straight to the review.
class StoreCard extends StatelessWidget {
  const StoreCard({super.key, required this.leading, required this.title, required this.source, required this.actionLabel,
      required this.onAction, this.onOpen, this.summary, this.compatibility, this.state, this.actionKey});

  final Widget leading;
  final String title;
  final String? summary;

  /// Where it comes from, one line.
  final String source;

  /// What it needs of this computer or the agent, one line; null when nothing is said.
  final String? compatibility;

  /// `Installed` and the like; null when it is not here.
  final String? state;
  final String actionLabel;
  final Key? actionKey;
  final VoidCallback? onAction;
  final VoidCallback? onOpen;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    final quiet = text.bodySmall?.copyWith(color: Theme.of(context).colorScheme.onSurfaceVariant);
    return Card(
      margin: const EdgeInsets.only(bottom: 8),
      child: InkWell(
        onTap: onOpen ?? onAction,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
            leading,
            const SizedBox(width: 12),
            Expanded(
              child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
                Text(oneLine(title), style: text.titleSmall, maxLines: 1, overflow: TextOverflow.ellipsis),
                if (summary != null) Text(oneLine(summary!), style: text.bodyMedium, maxLines: 2, overflow: TextOverflow.ellipsis),
                Text(oneLine(source), key: const Key('provenance'), style: quiet, maxLines: 1, overflow: TextOverflow.ellipsis),
                if (compatibility != null)
                  Text(oneLine(compatibility!), key: const Key('compatibility'), style: quiet, maxLines: 1, overflow: TextOverflow.ellipsis),
                if (state != null) Text(oneLine(state!), key: const Key('card-state'), style: text.labelMedium),
              ]),
            ),
            const SizedBox(width: 8),
            FilledButton.tonal(key: actionKey, onPressed: onAction, child: Text(actionLabel)),
          ]),
        ),
      ),
    );
  }
}

/// A letter on a disc, for a card with no icon of its own.
class LoadoutMark extends StatelessWidget {
  const LoadoutMark({super.key, required this.name, this.icon});
  final String name;
  final IconData? icon;

  @override
  Widget build(BuildContext context) => icon != null
      ? CircleAvatar(radius: 16, child: Icon(icon, size: 18))
      : OverlayPlaceholder(name: name, size: 32);
}
