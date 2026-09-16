//! Responsive view of the existing TalkWindowState for small embedded stages.
//! No copied playback, input, timing or dialogue owner: source glyph indices,
//! alpha, typewriter cursor and click latch remain in the parent module.
use super::*;

type Line = Vec<(GlyphSpot, f32, usize)>;

#[derive(Default)]
pub(crate) struct Cache {
    active: bool,
    viewport: Vec2,
    label_version: u64,
    text_version: u64,
}

#[derive(Component)]
pub(crate) struct ResponsiveDialogueMetrics {
    pub(crate) font_px: f32,
    pub(crate) line_count: usize,
    pub(crate) bounds: Rect,
}

/// Soft line breaks change only positions. They do not insert source characters
/// or consume extra typewriter beats. Existing hard lines and raw indices stay.
fn wrap(lines: Vec<Line>, width: f32) -> Vec<Line> {
    let mut result = Vec::new();
    for source in lines {
        let mut line = Vec::new();
        let mut origin = 0.;
        let mut glyphs = source.into_iter().peekable();
        while let Some((spot, pen, raw)) = glyphs.next() {
            let closing_next = glyphs.peek().is_some_and(|(next, _, _)| {
                matches!(
                    next.ch,
                    '。' | '、' | '，' | '！' | '？' | '」' | '』' | '）' | ',' | '.' | '!' | '?'
                )
            });
            let reserve = if closing_next {
                spot.size * spot.visual
            } else {
                0.
            };
            if !line.is_empty() && pen - origin + spot.size * spot.visual + reserve > width {
                result.push(line);
                line = Vec::new();
                origin = pen;
            }
            line.push((spot, pen - origin, raw));
        }
        result.push(line);
    }
    if result.is_empty() {
        result.push(Vec::new());
    }
    result
}

fn metrics(lines: &[Line], step: f32) -> LayoutMetrics {
    let widths: Vec<f32> = lines
        .iter()
        .map(|line| {
            line.iter()
                .map(|(spot, pen, _)| pen + spot.size * spot.visual)
                .fold(0., f32::max)
        })
        .collect();
    LayoutMetrics {
        box_w: widths.iter().copied().fold(0., f32::max),
        rect_widths: widths.clone(),
        line_widths: widths,
        anchor_base: 0.,
        line_offsets: (0..lines.len())
            .map(|line| line as f32 * step * TEXT_SCALE)
            .collect(),
    }
}

fn bounds(viewport: Vec2, body_lines: usize, label_lines: usize, font: f32) -> Rect {
    let width = (viewport.x - 28.).max(100.);
    let height = body_lines.max(1) as f32 * font * 1.4 + label_lines.max(1) as f32 * 20. + 42.;
    let bottom = -viewport.y * 0.5 + 14.;
    Rect::from_corners(
        Vec2::new(-width * 0.5, bottom),
        Vec2::new(width * 0.5, bottom + height),
    )
}

/// An ordinary nine-slice view of the same source panel texture. The source's
/// center is degenerate, so the one-pixel center sample is stretched, as in the
/// original panel; corners remain rounded instead of stretching the glyphs.
fn panel(rect: Rect) -> Vec<(Rect, Vec2, Vec2)> {
    let radius = 18_f32.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let xs = [
        rect.min.x,
        rect.min.x + radius,
        rect.max.x - radius,
        rect.max.x,
    ];
    let ys = [
        rect.max.y,
        rect.max.y - radius,
        rect.min.y + radius,
        rect.min.y,
    ];
    let sx = [
        (PANEL_SRC.min.x, PANEL_SRC.min.x + PANEL_BORDER),
        (
            PANEL_SRC.min.x + PANEL_BORDER,
            PANEL_SRC.min.x + PANEL_BORDER + 1.,
        ),
        (PANEL_SRC.max.x - PANEL_BORDER, PANEL_SRC.max.x),
    ];
    let sy = [
        (PANEL_SRC.min.y, PANEL_SRC.min.y + PANEL_BORDER),
        (
            PANEL_SRC.min.y + PANEL_BORDER,
            PANEL_SRC.min.y + PANEL_BORDER + 1.,
        ),
        (PANEL_SRC.max.y - PANEL_BORDER, PANEL_SRC.max.y),
    ];
    let mut result = Vec::new();
    for y in 0..3 {
        for x in 0..3 {
            result.push((
                Rect::from_corners(Vec2::new(sx[x].0, sy[y].0), Vec2::new(sx[x].1, sy[y].1)),
                Vec2::new(xs[x + 1] - xs[x], ys[y] - ys[y + 1]),
                Vec2::new((xs[x] + xs[x + 1]) * 0.5, (ys[y] + ys[y + 1]) * 0.5),
            ));
        }
    }
    result
}

fn spawn(
    commands: &mut Commands,
    art: &BalloonArt,
    window_art: &WindowArt,
    state: &TalkWindowState,
    viewport: Vec2,
) {
    let text_width = (viewport.x - 64.).max(64.);
    let mut font = 18.;
    let labels = wrap(walk_glyphs(&state.label, art, 14., 0., 0.).0, text_width);
    let mut content = wrap(walk_glyphs(&state.text, art, font, 0.18, 1.2).0, text_width);
    let mut area = bounds(viewport, content.len(), labels.len(), font);
    if area.height() > viewport.y - 28. {
        font = (font * (viewport.y - 28.) / area.height()).clamp(14., 18.);
        content = wrap(walk_glyphs(&state.text, art, font, 0.14, 1.).0, text_width);
        area = bounds(viewport, content.len(), labels.len(), font);
    }
    let root = commands
        .spawn((
            TalkWindowRoot,
            Transform::IDENTITY,
            Visibility::default(),
            RenderLayers::layer(BALLOON_LAYER),
            ResponsiveDialogueMetrics {
                font_px: font,
                line_count: content.len(),
                bounds: area,
            },
        ))
        .id();
    for (rect, size, position) in panel(area) {
        let entity = commands
            .spawn((
                Sprite {
                    image: window_art.page.clone(),
                    rect: Some(rect),
                    custom_size: Some(size),
                    color: Color::WHITE.with_alpha(state.alpha),
                    ..default()
                },
                Transform::from_xyz(position.x, position.y, -0.1),
                TalkWindowPart { base: Color::WHITE },
                RenderLayers::layer(BALLOON_LAYER),
            ))
            .id();
        commands.entity(root).add_child(entity);
    }
    let label_top = area.max.y - 16.;
    let label_rect = Rect::from_corners(
        Vec2::new(area.min.x + 18., label_top - labels.len() as f32 * 20.),
        Vec2::new(area.max.x - 18., label_top),
    );
    let body_top = label_rect.min.y - 8.;
    let body_rect = Rect::from_corners(
        Vec2::new(area.min.x + 18., area.min.y + 14.),
        Vec2::new(area.max.x - 18., body_top),
    );
    spawn_text_glyphs(
        commands,
        root,
        art,
        state,
        &labels,
        &metrics(&labels, 20.),
        label_rect,
        14.,
        LABEL_COLOR,
        GlyphSlot::Label,
    );
    spawn_text_glyphs(
        commands,
        root,
        art,
        state,
        &content,
        &metrics(&content, font * 1.4),
        body_rect,
        font,
        CONTENT_COLOR,
        GlyphSlot::Content,
    );
    let end = commands
        .spawn((
            Sprite {
                image: window_art.end_mark.clone(),
                custom_size: Some(Vec2::new(10., 9.)),
                color: Color::WHITE.with_alpha(state.alpha),
                ..default()
            },
            Transform::from_xyz(area.max.x - 14., area.min.y + 12., 0.),
            TalkWindowPart { base: Color::WHITE },
            TalkEndMark,
            if state.end_icon {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
    commands.entity(root).add_child(end);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn render(
    commands: &mut Commands,
    embedded: bool,
    viewport: Vec2,
    cache: &mut Cache,
    state: &TalkWindowState,
    server: &AssetServer,
    art: Option<&BalloonArt>,
    window_art: Option<&WindowArt>,
    roots: &mut Query<(Entity, &mut Transform), With<TalkWindowRoot>>,
) -> bool {
    let responsive = embedded && CONTENT_FONT * canvas_scale(viewport.x, viewport.y) < 16.;
    if !responsive {
        if cache.active {
            for (root, _) in roots.iter_mut() {
                commands.entity(root).despawn();
            }
            cache.active = false;
            return true;
        }
        return false;
    }
    let changed = !cache.active
        || cache.viewport != viewport
        || cache.label_version != state.label_version
        || cache.text_version != state.text_version;
    cache.active = true;
    if !state.present {
        for (root, _) in roots.iter_mut() {
            commands.entity(root).despawn();
        }
        return true;
    }
    let (Some(art), Some(window_art)) = (art, window_art) else {
        return true;
    };
    match server.load_state(&window_art.page) {
        LoadState::Loaded => {}
        LoadState::Failed(error) => panic!("Talk panel source failed to load: {error}"),
        _ => return true,
    }
    if roots.is_empty() || changed {
        for (root, _) in roots.iter_mut() {
            commands.entity(root).despawn();
        }
        spawn(commands, art, window_art, state, viewport);
        cache.viewport = viewport;
        cache.label_version = state.label_version;
        cache.text_version = state.text_version;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn glyph(raw: usize, pen: f32) -> (GlyphSpot, f32, usize) {
        (
            GlyphSpot {
                ch: '字',
                size: 18.,
                visual: 1.,
                color: None,
                alpha: None,
            },
            pen,
            raw,
        )
    }
    #[test]
    fn soft_wrapping_preserves_source_typewriter_indices_and_hard_lines() {
        let original = vec![
            vec![glyph(0, 0.), glyph(1, 18.), glyph(2, 36.), glyph(3, 54.)],
            vec![glyph(5, 0.), glyph(6, 18.)],
        ];
        let rows = wrap(original, 36.);
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter()
                .flatten()
                .map(|(_, _, raw)| *raw)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 5, 6]
        );
        assert!(
            metrics(&rows, 25.2)
                .line_widths
                .iter()
                .all(|width| *width <= 36.)
        );
    }
    #[test]
    fn mobile_panel_keeps_readable_physical_text_inside_the_viewport() {
        let viewport = Vec2::new(390., 390.);
        let rect = bounds(viewport, 8, 1, 18.);
        assert!(rect.min.x > -viewport.x / 2. && rect.max.x < viewport.x / 2.);
        assert!(rect.min.y > -viewport.y / 2. && rect.max.y < viewport.y / 2.);
        assert!(
            panel(rect)
                .iter()
                .all(|(_, size, _)| size.x > 0. && size.y > 0.)
        );
        assert!(CONTENT_FONT * canvas_scale(viewport.x, viewport.y) < 16.);
    }
}
