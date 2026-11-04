import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../profile/aos.dart';

/// An overlay's or a catalog's icon as AOS lets a store show one: PNG or WebP only, its size and the dimensions in its
/// header checked against [limits] before anything decodes it, decoded at no more than twice [size] on a side, and
/// shown as the still pixels of its first frame. Anything refused or undecodable shows [OverlayPlaceholder] drawn from
/// [name], never another image.
class StillIcon extends StatefulWidget {
  const StillIcon({super.key, required this.name, required this.path, required this.bytes, this.size = 40,
      this.limits = const IconLimits()});

  /// The overlay's name, for the placeholder.
  final String name;

  /// The icon's path, whose extension names its format; null when there is no icon.
  final String? path;
  final List<int>? bytes;
  final double size;
  final IconLimits limits;

  @override
  State<StillIcon> createState() => _StillIconState();
}

class _StillIconState extends State<StillIcon> {
  ui.Image? _image;
  var _generation = 0;

  @override
  void initState() {
    super.initState();
    _decode();
  }

  @override
  void didUpdateWidget(StillIcon old) {
    super.didUpdateWidget(old);
    if (old.bytes != widget.bytes || old.path != widget.path || old.size != widget.size) _decode();
  }

  @override
  void dispose() {
    _generation++;
    _image?.dispose();
    super.dispose();
  }

  Future<void> _decode() async {
    final generation = ++_generation;
    // Called from initState or didUpdateWidget, so the build that follows no longer paints the image disposed here.
    final old = _image;
    _image = null;
    old?.dispose();
    final path = widget.path, bytes = widget.bytes;
    if (path == null || bytes == null) return;
    try {
      final header = checkIcon(path, bytes, limits: widget.limits);
      // The longer side decodes to at most twice the space shown, keeping the aspect ratio.
      final side = (widget.size * 2).ceil();
      final wide = header.width >= header.height;
      final codec = await ui.instantiateImageCodec(Uint8List.fromList(bytes),
          targetWidth: wide && header.width > side ? side : null,
          targetHeight: !wide && header.height > side ? side : null);
      try {
        final frame = await codec.getNextFrame();
        if (!mounted || generation != _generation) {
          frame.image.dispose();
          return;
        }
        setState(() => _image = frame.image);
      } finally {
        codec.dispose();
      }
    } on Object {
      if (mounted && generation == _generation) setState(() => _image = null);
    }
  }

  @override
  Widget build(BuildContext context) => SizedBox.square(
    dimension: widget.size,
    child: _image == null
        ? OverlayPlaceholder(name: widget.name, size: widget.size)
        : RawImage(image: _image, fit: BoxFit.contain, filterQuality: FilterQuality.medium),
  );
}

/// The store's own placeholder for an overlay with no icon it may show: the first letter of its name on a colour taken
/// from the name.
class OverlayPlaceholder extends StatelessWidget {
  const OverlayPlaceholder({super.key, required this.name, this.size = 40});
  final String name;
  final double size;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final hue = name.codeUnits.fold<int>(0, (sum, unit) => (sum * 31 + unit) & 0xffff) % 360;
    final letter = name.isEmpty ? '?' : String.fromCharCode(name.runes.first).toUpperCase();
    return Container(
      key: const Key('overlay-placeholder'),
      width: size,
      height: size,
      alignment: Alignment.center,
      decoration: BoxDecoration(
        color: HSLColor.fromAHSL(1, hue.toDouble(), 0.45, scheme.brightness == Brightness.dark ? 0.35 : 0.7).toColor(),
        borderRadius: BorderRadius.circular(size / 5),
      ),
      child: Text(letter, style: TextStyle(fontSize: size / 2, fontWeight: FontWeight.w600, color: scheme.onSurface)),
    );
  }
}
