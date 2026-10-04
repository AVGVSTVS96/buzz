import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/skeleton.dart';
import 'message_media.dart';

/// Loading shapes derived from a known message, without fetching its media.
class MessageSkeletonBody extends StatelessWidget {
  final String content;
  final List<List<String>> tags;

  const MessageSkeletonBody({
    required this.content,
    required this.tags,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    final metadata = parseImetaTags(tags);
    final urls = RegExp(
      r'https?://[^\s)<>]+',
    ).allMatches(content).map((match) => match.group(0)!).toSet();
    final attachments = urls
        .where(
          (url) => metadata.containsKey(url) || classifyMediaUrl(url) != null,
        )
        .toList();
    var text = content;
    for (final url in attachments) {
      text = text.replaceAll(
        RegExp('!?\\[[^\\]]*\\]\\(${RegExp.escape(url)}\\)'),
        '',
      );
      text = text.replaceAll(url, '');
    }
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = constraints.hasBoundedWidth
            ? constraints.maxWidth
            : 280.0;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            if (text.trim().isNotEmpty)
              for (
                var line = 0;
                line <
                    (text.length / math.max(1, width / 8)).ceil().clamp(1, 4);
                line++
              )
                Padding(
                  padding: const EdgeInsets.only(bottom: Grid.half),
                  child: SkeletonBar(
                    width: math.min(width, math.max(48, text.length * 7.0)),
                    height: 16,
                  ),
                ),
            for (final url in attachments)
              Padding(
                padding: const EdgeInsets.only(bottom: Grid.xxs),
                child: _attachment(context, url, metadata[url], width),
              ),
          ],
        );
      },
    );
  }

  Widget _attachment(
    BuildContext context,
    String url,
    ImetaEntry? meta,
    double width,
  ) {
    final kind = classifyMediaUrl(url, imeta: meta);
    if (kind == MessageMediaKind.image || kind == MessageMediaKind.video) {
      final rawRatio = meta?.aspectRatio;
      final ratio = rawRatio != null && rawRatio.isFinite && rawRatio > 0
          ? rawRatio.clamp(0.2, 4.0)
          : (kind == MessageMediaKind.video ? 16 / 9 : 1.0);
      final height = math.min(240.0, math.min(width, 320.0) / ratio);
      return SizedBox(
        key: ValueKey('message-skeleton-${kind!.name}:$url'),
        width: height * ratio,
        height: height,
        child: Stack(
          alignment: Alignment.center,
          children: [
            Opacity(
              opacity: kind == MessageMediaKind.video ? 0.35 : 1,
              child: SkeletonBar(
                width: double.infinity,
                height: double.infinity,
                borderRadius: BorderRadius.circular(Radii.md),
              ),
            ),
            if (kind == MessageMediaKind.video)
              const Icon(
                Icons.play_circle_outline,
                size: 48,
                color: Colors.white,
              ),
          ],
        ),
      );
    }
    if (kind == MessageMediaKind.audio) {
      return SizedBox(
        key: ValueKey('message-skeleton-audio:$url'),
        width: math.min(width, 320),
        height: 64,
        child: Row(
          children: [
            SkeletonBar(
              width: 40,
              height: 40,
              borderRadius: BorderRadius.circular(Radii.full),
            ),
            const SizedBox(width: Grid.xxs),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  SizedBox(
                    height: 24,
                    child: Row(
                      children: [
                        for (var i = 0; i < 24; i++)
                          Expanded(
                            child: Padding(
                              padding: const EdgeInsets.symmetric(
                                horizontal: 1,
                              ),
                              child: SkeletonBar(
                                width: 3,
                                height: [8.0, 16.0, 24.0, 12.0, 20.0][i % 5],
                              ),
                            ),
                          ),
                      ],
                    ),
                  ),
                  const SizedBox(height: Grid.half),
                  const SkeletonBar(width: 36, height: 10),
                ],
              ),
            ),
          ],
        ),
      );
    }
    return SizedBox(
      key: ValueKey('message-skeleton-file:$url'),
      height: 64,
      child: Row(
        children: [
          const SkeletonBar(width: 36, height: 44),
          const SizedBox(width: Grid.xxs),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                const SkeletonBar(width: double.infinity, height: 14),
                const SizedBox(height: Grid.xxs),
                const SkeletonBar(width: 56, height: 10),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
