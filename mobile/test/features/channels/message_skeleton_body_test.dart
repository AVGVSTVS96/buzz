import 'package:buzz/features/channels/message_skeleton_body.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/skeleton.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final (mime, name, kind) in [
    ('image/jpeg', 'photo.jpg', 'image'),
    ('video/mp4', 'clip.mp4', 'video'),
    ('audio/mp4', 'voice.m4a', 'audio'),
    ('video/mp4', 'voice-note-123.mp4', 'audio'),
    ('application/pdf', 'report.pdf', 'file'),
  ]) {
    testWidgets('uses $kind shape for $name without loading media', (
      tester,
    ) async {
      final url = 'https://example.com/$name';
      await tester.pumpWidget(
        MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: Center(
              child: SizedBox(
                width: 240,
                child: MessageSkeletonBody(
                  content: '![attachment]($url)',
                  tags: [
                    [
                      'imeta',
                      'url $url',
                      'm $mime',
                      'filename $name',
                      'dim 1200x2400',
                    ],
                  ],
                ),
              ),
            ),
          ),
        ),
      );
      final shape = find.byKey(ValueKey('message-skeleton-$kind:$url'));
      expect(shape, findsOneWidget);
      expect(find.byType(Image), findsNothing);
      expect(tester.getSize(shape).width, lessThanOrEqualTo(240));
      if (kind == 'image' || kind == 'video') {
        expect(tester.getSize(shape), const Size(120, 240));
      } else {
        expect(tester.getSize(shape).height, 64);
      }
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets('includes both caption and multiple attachment shapes', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        theme: AppTheme.light(),
        home: const Scaffold(
          body: SizedBox(
            width: 280,
            child: MessageSkeletonBody(
              content:
                  'Caption\n![photo](https://example.com/a.jpg)\n![voice](https://example.com/b.m4a)',
              tags: [],
            ),
          ),
        ),
      ),
    );
    expect(
      find.byKey(
        const ValueKey('message-skeleton-image:https://example.com/a.jpg'),
      ),
      findsOneWidget,
    );
    expect(
      find.byKey(
        const ValueKey('message-skeleton-audio:https://example.com/b.m4a'),
      ),
      findsOneWidget,
    );
    expect(find.byType(SkeletonBar), findsWidgets);
    expect(tester.takeException(), isNull);
  });
}
