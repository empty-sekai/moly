//! Retained, responsive chrome. Only a visible page and the selected transcript
//! are materialized; scrolling never reconstructs the whole catalog.
use super::*;
const INK: Color = Color::srgb(0.91, 0.95, 0.98);
const MUTED: Color = Color::srgb(0.57, 0.67, 0.76);
const ACCENT: Color = Color::srgb(0.33, 0.84, 0.88);
const PANEL: Color = Color::srgb(0.070, 0.095, 0.14);
const EDGE: Color = Color::srgb(0.16, 0.23, 0.30);
const ROW_HEIGHT: f32 = 76.;
fn attach(c: &mut Commands, parent: Entity, node: Node) -> Entity {
    let id = c.spawn(node).id();
    c.entity(parent).add_child(id);
    id
}
fn column(c: &mut Commands, parent: Entity, gap: f32) -> Entity {
    attach(
        c,
        parent,
        Node {
            width: percent(100),
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(gap),
            ..default()
        },
    )
}
fn row(c: &mut Commands, parent: Entity, gap: f32) -> Entity {
    attach(
        c,
        parent,
        Node {
            width: percent(100),
            min_width: px(0),
            align_items: AlignItems::Center,
            column_gap: px(gap),
            flex_shrink: 0.,
            ..default()
        },
    )
}
fn text(
    c: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    value: impl Into<String>,
    size: f32,
    color: Color,
) -> Entity {
    let id = c
        .spawn((
            Text::new(value),
            TextFont {
                font: font.clone(),
                font_size: size,
                ..default()
            },
            TextColor(color),
            Node {
                min_width: px(0),
                flex_shrink: 0.,
                ..default()
            },
        ))
        .id();
    c.entity(parent).add_child(id);
    id
}
fn label(
    c: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    kind: UiLabel,
    size: f32,
    color: Color,
) -> Entity {
    let id = text(c, parent, font, "", size, color);
    c.entity(id).insert(kind);
    if matches!(kind, UiLabel::TransportTitle | UiLabel::Search) {
        c.entity(id).insert(TextLayout::new_with_no_wrap());
    }
    id
}
fn button(
    c: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    value: &str,
    action: LibraryAction,
    kind: ButtonKind,
) -> Entity {
    let id = c
        .spawn((
            Button,
            action,
            kind,
            Node {
                min_height: px(36),
                padding: UiRect::axes(px(12), px(7)),
                flex_shrink: 0.,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(9)),
                ..default()
            },
            BackgroundColor(PANEL),
            BorderColor::all(EDGE),
        ))
        .id();
    c.entity(parent).add_child(id);
    text(c, id, font, value, 14., INK);
    id
}
fn labeled_button(
    c: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    action: LibraryAction,
    kind: ButtonKind,
    marker: UiLabel,
) -> Entity {
    let id = button(c, parent, font, "", action, kind);
    label(c, id, font, marker, 14., INK);
    id
}
fn surface(c: &mut Commands, parent: Entity, node: Node, region: Region) -> Entity {
    let id = attach(c, parent, node);
    c.entity(id)
        .insert((region, BackgroundColor(PANEL), BorderColor::all(EDGE)));
    id
}
pub(crate) fn setup(mut c: Commands, mut fonts: ResMut<Assets<Font>>) {
    let font = fonts.add(Font::try_from_bytes(FONT.to_vec()).expect("bundled library font"));
    c.insert_resource(LibraryFont(font.clone()));
    let camera = c
        .spawn((
            Camera2d,
            Camera {
                order: 104,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            RenderLayers::none(),
            LibraryUiCamera,
        ))
        .id();
    let launcher = c
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(150),
                bottom: px(14),
                ..default()
            },
            UiTargetCamera(camera),
            GlobalZIndex(1001),
            Region::Launcher,
        ))
        .id();
    button(
        &mut c,
        launcher,
        &font,
        "对话与互动 F9",
        LibraryAction::Toggle,
        ButtonKind::Secondary,
    );
    let transport = c
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: px(18),
                top: px(18),
                display: Display::None,
                max_width: percent(96),
                align_items: AlignItems::Center,
                column_gap: px(8),
                padding: UiRect::all(px(10)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(14)),
                ..default()
            },
            Region::Transport,
            BackgroundColor(Color::srgba(0.035, 0.06, 0.10, 0.96)),
            BorderColor::all(EDGE),
            UiTargetCamera(camera),
            GlobalZIndex(1190),
        ))
        .id();
    let now = column(&mut c, transport, 3.);
    c.entity(now).insert(Node {
        width: px(190),
        min_width: px(0),
        overflow: Overflow::clip(),
        flex_direction: FlexDirection::Column,
        row_gap: px(3),
        ..default()
    });
    label(&mut c, now, &font, UiLabel::TransportState, 12., ACCENT);
    label(&mut c, now, &font, UiLabel::TransportTitle, 14., INK);
    button(
        &mut c,
        transport,
        &font,
        "停止",
        LibraryAction::Stop,
        ButtonKind::Danger,
    );
    button(
        &mut c,
        transport,
        &font,
        "返回原场景",
        LibraryAction::RestoreScene,
        ButtonKind::Secondary,
    );
    button(
        &mut c,
        transport,
        &font,
        "对话与互动 F9",
        LibraryAction::Return,
        ButtonKind::Secondary,
    );
    let overlay = c
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                display: Display::None,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::all(px(18)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.018, 0.03, 0.055, 0.80)),
            FocusPolicy::Block,
            Region::Overlay,
            UiTargetCamera(camera),
            GlobalZIndex(1200),
        ))
        .id();
    let shell = surface(
        &mut c,
        overlay,
        Node {
            width: percent(100),
            max_width: px(1360),
            height: percent(100),
            max_height: px(940),
            min_height: px(0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(22)),
            row_gap: px(12),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(20)),
            ..default()
        },
        Region::Shell,
    );
    c.entity(shell)
        .insert(BackgroundColor(Color::srgb(0.035, 0.052, 0.087)));
    let header = row(&mut c, shell, 14.);
    let heading = column(&mut c, header, 3.);
    let heading_text = text(&mut c, heading, &font, "Moly 对话与互动", 27., INK);
    c.entity(heading_text).insert(Region::HeaderTitle);
    let subtitle = text(
        &mut c,
        heading,
        &font,
        "重温每一句日常，也亲自走进家具与角色的演出。",
        13.,
        MUTED,
    );
    c.entity(subtitle).insert(Region::HeaderSubtitle);
    let resume = button(
        &mut c,
        header,
        &font,
        "继续观看",
        LibraryAction::Continue,
        ButtonKind::Secondary,
    );
    c.entity(resume).insert(Region::ResumeControl);
    button(
        &mut c,
        header,
        &font,
        "关闭 Esc",
        LibraryAction::Close,
        ButtonKind::Ghost,
    );
    let nav = row(&mut c, shell, 8.);
    c.entity(nav).insert(Region::Navigation);
    for tab in [
        LibraryTab::Conversations,
        LibraryTab::Furniture,
        LibraryTab::Performances,
    ] {
        button(
            &mut c,
            nav,
            &font,
            tab.title(),
            LibraryAction::Tab(tab),
            ButtonKind::Chip,
        );
    }
    let modes = row(&mut c, shell, 8.);
    text(&mut c, modes, &font, "体验方式", 12., ACCENT);
    button(
        &mut c,
        modes,
        &font,
        "当前场景",
        LibraryAction::SetMode(ExperienceMode::CurrentScene),
        ButtonKind::Chip,
    );
    button(
        &mut c,
        modes,
        &font,
        "独立体验",
        LibraryAction::SetMode(ExperienceMode::Independent),
        ButtonKind::Chip,
    );
    let mode_description = label(&mut c, modes, &font, UiLabel::ModeDescription, 12., MUTED);
    c.entity(mode_description).insert(Region::ModeDescription);
    label(&mut c, modes, &font, UiLabel::Source, 11., ACCENT);
    let search = surface(
        &mut c,
        shell,
        Node {
            width: percent(100),
            height: px(42),
            min_height: px(42),
            flex_shrink: 0.,
            align_items: AlignItems::Center,
            padding: UiRect::axes(px(14), px(4)),
            column_gap: px(12),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(10)),
            ..default()
        },
        Region::SearchBox,
    );
    c.entity(search)
        .insert((Button, LibraryAction::FocusSearch, ButtonKind::Secondary));
    text(&mut c, search, &font, "搜索", 13., ACCENT);
    let query = label(&mut c, search, &font, UiLabel::Search, 15., INK);
    c.entity(query).insert(Node {
        flex_grow: 1.,
        min_width: px(0),
        overflow: Overflow::clip(),
        height: px(23),
        ..default()
    });
    button(
        &mut c,
        search,
        &font,
        "清空",
        LibraryAction::ClearSearch,
        ButtonKind::Ghost,
    );
    let filters = row(&mut c, shell, 6.);
    c.entity(filters).insert((
        Region::Filters,
        Node {
            width: percent(100),
            column_gap: px(6),
            row_gap: px(6),
            flex_wrap: FlexWrap::Wrap,
            flex_shrink: 0.,
            align_items: AlignItems::Center,
            ..default()
        },
    ));
    button(
        &mut c,
        filters,
        &font,
        "场景内",
        LibraryAction::SetScope(Scope::Here),
        ButtonKind::Chip,
    );
    labeled_button(
        &mut c,
        filters,
        &font,
        LibraryAction::SetScope(Scope::Ready),
        ButtonKind::Chip,
        UiLabel::ReadyScope,
    );
    button(
        &mut c,
        filters,
        &font,
        "全部内容",
        LibraryAction::SetScope(Scope::All),
        ButtonKind::Chip,
    );
    let character = labeled_button(
        &mut c,
        filters,
        &font,
        LibraryAction::CharacterPicker,
        ButtonKind::Chip,
        UiLabel::Character,
    );
    c.entity(character).insert(Region::CharacterFilter);
    let special = labeled_button(
        &mut c,
        filters,
        &font,
        LibraryAction::SpecialFilter,
        ButtonKind::Chip,
        UiLabel::Special,
    );
    c.entity(special).insert(Region::SpecialFilter);
    let related = labeled_button(
        &mut c,
        filters,
        &font,
        LibraryAction::ClearRelated,
        ButtonKind::Chip,
        UiLabel::Related,
    );
    c.entity(related).insert(Region::RelatedFilter);
    let content = attach(
        &mut c,
        shell,
        Node {
            width: percent(100),
            flex_grow: 1.,
            min_height: px(0),
            column_gap: px(16),
            overflow: Overflow::clip(),
            ..default()
        },
    );
    c.entity(content).insert(Region::Content);
    let browser = column(&mut c, content, 8.);
    c.entity(browser).insert((
        Region::Browser,
        Node {
            width: percent(44),
            height: percent(100),
            min_height: px(0),
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(8),
            ..default()
        },
    ));
    label(&mut c, browser, &font, UiLabel::Count, 12., MUTED);
    let viewport = attach(
        &mut c,
        browser,
        Node {
            width: percent(100),
            flex_grow: 1.,
            min_height: px(0),
            overflow: Overflow::clip(),
            ..default()
        },
    );
    c.entity(viewport).insert((
        Region::ListViewport,
        RelativeCursorPosition::default(),
        ScrollPosition::default(),
    ));
    let items = column(&mut c, viewport, 8.);
    c.entity(items).insert(Region::ListItems);
    let pager = row(&mut c, browser, 8.);
    button(
        &mut c,
        pager,
        &font,
        "上一页",
        LibraryAction::PreviousPage,
        ButtonKind::Ghost,
    );
    let page = label(&mut c, pager, &font, UiLabel::Page, 12., MUTED);
    c.entity(page).insert(Node {
        flex_grow: 1.,
        ..default()
    });
    button(
        &mut c,
        pager,
        &font,
        "下一页",
        LibraryAction::NextPage,
        ButtonKind::Ghost,
    );
    let detail = surface(
        &mut c,
        content,
        Node {
            width: percent(56),
            height: percent(100),
            min_height: px(0),
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            padding: UiRect::all(px(18)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(14)),
            ..default()
        },
        Region::Detail,
    );
    let back = button(
        &mut c,
        detail,
        &font,
        "← 返回列表",
        LibraryAction::Back,
        ButtonKind::Ghost,
    );
    c.entity(back).insert(Region::Back);
    let scroll = attach(
        &mut c,
        detail,
        Node {
            width: percent(100),
            flex_grow: 1.,
            min_height: px(0),
            overflow: Overflow::scroll_y(),
            ..default()
        },
    );
    c.entity(scroll).insert((
        Region::DetailScroll,
        RelativeCursorPosition::default(),
        ScrollPosition::default(),
    ));
    let detail_body = column(&mut c, scroll, 13.);
    c.entity(detail_body).insert(Region::DetailBody);
    let instances = row(&mut c, detail, 6.);
    c.entity(instances).insert(Region::InstanceControls);
    button(
        &mut c,
        instances,
        &font,
        "←",
        LibraryAction::PreviousInstance,
        ButtonKind::Ghost,
    );
    let instance = label(&mut c, instances, &font, UiLabel::Instance, 12., MUTED);
    c.entity(instance).insert(Node {
        flex_grow: 1.,
        ..default()
    });
    button(
        &mut c,
        instances,
        &font,
        "→",
        LibraryAction::NextInstance,
        ButtonKind::Ghost,
    );
    label(&mut c, detail, &font, UiLabel::Availability, 13., MUTED);
    let playback = row(&mut c, detail, 8.);
    let play = labeled_button(
        &mut c,
        playback,
        &font,
        LibraryAction::Play,
        ButtonKind::Primary,
        UiLabel::Play,
    );
    c.entity(play).insert(Node {
        flex_grow: 1.,
        min_height: px(40),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        border: UiRect::all(px(1)),
        border_radius: BorderRadius::all(px(9)),
        ..default()
    });
    button(
        &mut c,
        playback,
        &font,
        "停止",
        LibraryAction::Stop,
        ButtonKind::Danger,
    );
    label(&mut c, shell, &font, UiLabel::Status, 12., ACCENT);
    let hint = text(
        &mut c,
        shell,
        &font,
        "Ctrl+F 搜索 · ↑↓ 选择 · Enter 体验 · Ctrl+1/2/3 切换 · 台词区可滚动",
        12.,
        MUTED,
    );
    c.entity(hint).insert(Region::FooterHint);
    let picker = surface(
        &mut c,
        overlay,
        Node {
            position_type: PositionType::Absolute,
            width: percent(88),
            max_width: px(620),
            height: percent(70),
            display: Display::None,
            flex_direction: FlexDirection::Column,
            row_gap: px(14),
            padding: UiRect::all(px(22)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(16)),
            ..default()
        },
        Region::CharacterPicker,
    );
    c.entity(picker)
        .insert((GlobalZIndex(1210), FocusPolicy::Block));
    let picker_header = row(&mut c, picker, 8.);
    let title = text(&mut c, picker_header, &font, "选择登场角色", 23., INK);
    c.entity(title).insert(Node {
        flex_grow: 1.,
        ..default()
    });
    button(
        &mut c,
        picker_header,
        &font,
        "返回",
        LibraryAction::DismissPicker,
        ButtonKind::Ghost,
    );
    let choices = attach(
        &mut c,
        picker,
        Node {
            width: percent(100),
            flex_grow: 1.,
            min_height: px(0),
            flex_wrap: FlexWrap::Wrap,
            align_content: AlignContent::FlexStart,
            column_gap: px(8),
            row_gap: px(8),
            overflow: Overflow::scroll_y(),
            ..default()
        },
    );
    c.entity(choices).insert((
        Region::CharacterChoices,
        RelativeCursorPosition::default(),
        ScrollPosition::default(),
    ));
}
fn character_color(catalog: &LibraryCatalog, unit: u32) -> Color {
    catalog
        .character_colors
        .get(&unit)
        .and_then(|color| bevy::color::Srgba::hex(color).ok())
        .map(Color::from)
        .unwrap_or(ACCENT)
}
fn artwork(
    c: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    image: Option<&Handle<Image>>,
    title: &str,
    size: f32,
    color: Color,
) {
    let node = attach(
        c,
        parent,
        Node {
            width: px(size),
            height: px(size),
            flex_shrink: 0.,
            border_radius: BorderRadius::all(px(10)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            overflow: Overflow::clip(),
            ..default()
        },
    );
    c.entity(node)
        .insert(BackgroundColor(Color::srgb(0.12, 0.20, 0.24)));
    if let Some(image) = image {
        c.entity(node).insert(ImageNode::new(image.clone()));
    } else {
        text(
            c,
            node,
            font,
            title.chars().take(1).collect::<String>(),
            size * 0.40,
            color,
        );
    }
}
fn populate_list(
    c: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    state: &ContentLibrary,
    catalog: &LibraryCatalog,
    world: &LibraryContext,
) {
    if state.filtered.is_empty() {
        text(
            c,
            parent,
            font,
            if !catalog.talks_ready {
                "正在整理内容…"
            } else {
                "这里还没有符合条件的内容"
            },
            19.,
            INK,
        );
        text(
            c,
            parent,
            font,
            "试试「全部内容」，或缩短关键词。未摆放的家具也可以查看图片和故事。",
            14.,
            MUTED,
        );
        button(
            c,
            parent,
            font,
            "浏览全部内容",
            LibraryAction::SetScope(Scope::All),
            ButtonKind::Secondary,
        );
        return;
    }
    for key in state
        .filtered
        .iter()
        .skip(state.offset)
        .take(state.page_size)
    {
        let item = c
            .spawn((
                Button,
                LibraryAction::Select(*key),
                ButtonKind::Row,
                Node {
                    width: percent(100),
                    height: px(ROW_HEIGHT - 8.),
                    min_height: px(ROW_HEIGHT - 8.),
                    flex_shrink: 0.,
                    padding: UiRect::all(px(9)),
                    column_gap: px(10),
                    align_items: AlignItems::Center,
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(11)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(EDGE),
            ))
            .id();
        c.entity(parent).add_child(item);
        let (title, meta, image, icon, color) = match key {
            EntryKey::Fixture(id) => {
                let Some(row) = catalog.fixture(*id) else {
                    continue;
                };
                let count = world.instances.get(id).map(Vec::len).unwrap_or(0);
                (
                    row.name.clone(),
                    format!(
                        "{} · {}",
                        row.action_label(),
                        if count == 0 {
                            "尚未摆放".into()
                        } else {
                            format!("已摆放 {count} 件")
                        }
                    ),
                    row.thumbnail.as_ref(),
                    "家".to_owned(),
                    ACCENT,
                )
            }
            _ => {
                let Some(row) = catalog.talk(*key) else {
                    continue;
                };
                let names = row
                    .units
                    .iter()
                    .map(|unit| catalog.character(*unit))
                    .collect::<Vec<_>>()
                    .join(" · ");
                let picture = row.fixture_ids.iter().find_map(|id| {
                    catalog
                        .fixture(*id)
                        .and_then(|fixture| fixture.thumbnail.as_ref())
                });
                let unit = row.units.first().copied().unwrap_or(0);
                (
                    row.title.clone(),
                    format!("{} · {}", row.kind(), names),
                    picture,
                    catalog.character(unit),
                    character_color(catalog, unit),
                )
            }
        };
        artwork(c, item, font, image, &icon, 46., color);
        let copy = column(c, item, 4.);
        c.entity(copy).insert(Node {
            flex_grow: 1.,
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            overflow: Overflow::clip(),
            ..default()
        });
        let title = text(c, copy, font, excerpt(&title, 25), 15., INK);
        c.entity(title).insert((
            TextLayout::new_with_no_wrap(),
            Node {
                height: px(22),
                overflow: Overflow::clip(),
                ..default()
            },
        ));
        let meta = text(c, copy, font, excerpt(&meta, 40), 12., MUTED);
        c.entity(meta).insert((
            TextLayout::new_with_no_wrap(),
            Node {
                height: px(18),
                overflow: Overflow::clip(),
                ..default()
            },
        ));
    }
}
fn populate_detail(
    c: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    key: Option<EntryKey>,
    catalog: &LibraryCatalog,
) {
    match key {
        Some(EntryKey::Fixture(id)) => {
            let Some(fixture) = catalog.fixture(id) else {
                return;
            };
            let hero = row(c, parent, 18.);
            artwork(
                c,
                hero,
                font,
                fixture.thumbnail.as_ref(),
                "家",
                112.,
                ACCENT,
            );
            let copy = column(c, hero, 7.);
            text(c, copy, font, &fixture.name, 23., INK);
            text(c, copy, font, fixture.action_label(), 13., ACCENT);
            if fixture.thumbnail.is_none() {
                text(c, parent, font, "这件家具暂时没有展示图片", 12., MUTED);
            }
            if !fixture.description.is_empty() && fixture.description != fixture.name {
                text(c, parent, font, &fixture.description, 16., INK);
            }
            text(
                c,
                parent,
                font,
                match fixture.action.as_str() {
                    "timeline" => "在场景中选中一件，体验角色与家具的完整互动。",
                    "loop" => "启动这件家具的持续互动。结束体验时会恢复原来的状态。",
                    "one_shot" => "欣赏一次完整的家具互动，也可以随时停止。",
                    _ => "这是一件陈设家具。相关的角色故事也值得一看。",
                },
                14.,
                MUTED,
            );
            let count = catalog
                .talks
                .iter()
                .filter(|talk| talk.fixture_ids.contains(&id))
                .count();
            button(
                c,
                parent,
                font,
                &format!("相关故事 · {count} 段 →"),
                LibraryAction::Related(id),
                ButtonKind::Secondary,
            );
        }
        Some(key) => {
            let Some(talk) = catalog.talk(key) else {
                return;
            };
            text(
                c,
                parent,
                font,
                format!("{} / {} 句台词", talk.kind(), talk.lines.len()),
                12.,
                ACCENT,
            );
            text(c, parent, font, excerpt(&talk.title, 32), 22., INK);
            let cast = row(c, parent, 7.);
            c.entity(cast).insert(Node {
                width: percent(100),
                flex_wrap: FlexWrap::Wrap,
                column_gap: px(7),
                row_gap: px(6),
                flex_shrink: 0.,
                ..default()
            });
            for unit in &talk.units {
                text(
                    c,
                    cast,
                    font,
                    catalog.character(*unit),
                    14.,
                    character_color(catalog, *unit),
                );
            }
            if !talk.fixture_ids.is_empty() {
                text(
                    c,
                    parent,
                    font,
                    talk.fixture_ids
                        .iter()
                        .map(|id| catalog.fixture_name(*id))
                        .collect::<Vec<_>>()
                        .join(" · "),
                    13.,
                    MUTED,
                );
            }
            if talk.lines.is_empty() {
                text(
                    c,
                    parent,
                    font,
                    "这段演出以动作或声音为主，没有可预览的台词。",
                    16.,
                    MUTED,
                );
            }
            for (index, line) in talk.lines.iter().enumerate() {
                let block = column(c, parent, 5.);
                text(
                    c,
                    block,
                    font,
                    format!("{:02}   {}", index + 1, line.speaker),
                    12.,
                    ACCENT,
                );
                text(c, block, font, &line.text, 16., INK);
            }
            text(
                c,
                parent,
                font,
                "台词按原剧本顺序展示 · 播放时将回到场景",
                12.,
                MUTED,
            );
        }
        None => {
            text(c, parent, font, "为你喜欢的故事，留一个位置。", 23., INK);
            text(
                c,
                parent,
                font,
                "从列表中挑选内容。可以搜索角色、家具名称，也可以用记得的一句台词找到那段故事。",
                16.,
                MUTED,
            );
        }
    }
}
#[derive(Default)]
pub(crate) struct ViewCache {
    list: Option<(u64, u64, u64, usize, usize)>,
    detail: Option<(Option<EntryKey>, u64)>,
    characters: u64,
}
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn refresh(
    mut c: Commands,
    mut state: ResMut<ContentLibrary>,
    catalog: Res<LibraryCatalog>,
    world: Res<LibraryContext>,
    font: Res<LibraryFont>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut regions: Query<(
        Entity,
        &Region,
        &mut Node,
        Option<&ComputedNode>,
        Option<&mut ScrollPosition>,
        Option<&mut TextFont>,
    )>,
    mut labels: Query<(&UiLabel, &mut Text)>,
    mut buttons: Query<(
        &LibraryAction,
        &ButtonKind,
        &Interaction,
        &mut BackgroundColor,
        &mut BorderColor,
    )>,
    mut cache: Local<ViewCache>,
) {
    let window = windows.iter().next();
    let narrow = window.is_some_and(|w| w.width() < 860.);
    let short = window.is_some_and(|w| w.height() < 560.);
    let selected_instances = state
        .selected
        .map(|key| context::instances_for(key, &catalog, &world))
        .unwrap_or_default();
    let available = context::selected_reason(&state, &catalog, &world);
    let playing = state.active.is_some() || state.pending.is_some();
    let mut list_root = None;
    let mut detail_root = None;
    let mut character_root = None;
    for (entity, region, mut node, computed, scroll, text_font) in &mut regions {
        let visible = match region {
            Region::Overlay => state.open,
            Region::Transport => state.watching,
            Region::Launcher => !state.open && !state.watching,
            Region::CharacterPicker => state.open && state.picker_open,
            Region::Browser => !narrow || !state.narrow_detail,
            Region::Navigation | Region::SearchBox | Region::Filters => {
                !narrow || !state.narrow_detail
            }
            Region::Detail => !narrow || state.narrow_detail,
            Region::Back => narrow,
            Region::HeaderSubtitle | Region::FooterHint | Region::ModeDescription => !short,
            Region::CharacterFilter => state.tab != LibraryTab::Furniture,
            Region::SpecialFilter => state.tab != LibraryTab::Conversations,
            Region::RelatedFilter => state.related_fixture.is_some(),
            Region::ResumeControl => state.active.as_ref().is_some_and(|a| a.started),
            Region::InstanceControls => selected_instances.len() > 1,
            _ => true,
        };
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        match region {
            Region::Overlay => node.padding = UiRect::all(px(if narrow || short { 8 } else { 18 })),
            Region::Shell => {
                node.padding = UiRect::all(px(if short {
                    10
                } else if narrow {
                    14
                } else {
                    22
                }));
                node.row_gap = px(if short { 5 } else { 12 });
            }
            Region::HeaderTitle => {
                if let Some(mut font) = text_font {
                    font.font_size = if short { 21. } else { 27. };
                }
            }
            Region::ModeDescription => {
                if let Some(mut text_font) = text_font {
                    text_font.font_size = if narrow { 11. } else { 12. };
                }
            }
            Region::SearchBox => {
                node.height = px(if short { 34 } else { 42 });
                node.min_height = node.height;
            }
            Region::Browser => node.width = percent(if narrow { 100. } else { 44. }),
            Region::Detail => {
                node.width = percent(if narrow { 100. } else { 56. });
                node.padding = UiRect::all(px(if short { 10 } else { 18 }));
            }
            Region::ListViewport if state.open && (!narrow || !state.narrow_detail) => {
                if let Some(computed) = computed {
                    let height = computed.size().y * computed.inverse_scale_factor();
                    if height > 40. {
                        state.page_size = ((height + 8.) / ROW_HEIGHT).floor().max(1.) as usize;
                    }
                }
            }
            Region::ListItems => list_root = Some(entity),
            Region::DetailBody => detail_root = Some(entity),
            Region::CharacterChoices => character_root = Some(entity),
            Region::DetailScroll => {
                if cache.detail != Some((state.selected, catalog.revision)) {
                    if let Some(mut scroll) = scroll {
                        scroll.y = 0.;
                    }
                }
            }
            _ => {}
        }
    }
    state.offset = state
        .offset
        .min(state.filtered.len().saturating_sub(state.page_size));
    let list_stamp = (
        state.revision,
        catalog.revision,
        world.revision,
        state.offset,
        state.page_size,
    );
    if state.open && cache.list != Some(list_stamp) {
        if let Some(parent) = list_root {
            c.entity(parent).despawn_children();
            populate_list(&mut c, parent, &font.0, &state, &catalog, &world);
        }
        cache.list = Some(list_stamp);
    }
    let detail_stamp = (state.selected, catalog.revision);
    if state.open && cache.detail != Some(detail_stamp) {
        if let Some(parent) = detail_root {
            c.entity(parent).despawn_children();
            populate_detail(&mut c, parent, &font.0, state.selected, &catalog);
        }
        cache.detail = Some(detail_stamp);
    }
    if cache.characters != catalog.revision {
        if let Some(parent) = character_root {
            c.entity(parent).despawn_children();
            button(
                &mut c,
                parent,
                &font.0,
                "所有角色",
                LibraryAction::SetCharacter(None),
                ButtonKind::Chip,
            );
            let mut units: Vec<_> = catalog.character_names.keys().copied().collect();
            units.sort_unstable();
            for unit in units {
                button(
                    &mut c,
                    parent,
                    &font.0,
                    &catalog.character(unit),
                    LibraryAction::SetCharacter(Some(unit)),
                    ButtonKind::Chip,
                );
            }
        }
        cache.characters = catalog.revision;
    }
    for (kind, mut text) in &mut labels {
        let value = match kind {
            UiLabel::Search => {
                if state.search.is_empty() && state.ime_preedit.is_empty() {
                    "角色、家具名称，或记得的一句台词… Ctrl+F".into()
                } else {
                    let chars: Vec<_> = state.search.chars().collect();
                    let cursor = state.search_cursor.min(chars.len());
                    let start = cursor.saturating_sub(if narrow { 16 } else { 45 });
                    let before: String = chars[start..cursor].iter().collect();
                    let after: String = chars[cursor..].iter().take(35).collect();
                    format!(
                        "{}{}{}{}{}",
                        if start > 0 { "…" } else { "" },
                        before,
                        state.ime_preedit,
                        if state.search_focus {
                            if state.select_all {
                                " [全选] "
                            } else {
                                "│"
                            }
                        } else {
                            ""
                        },
                        after
                    )
                }
            }
            UiLabel::Count => {
                if catalog.talks_ready {
                    format!(
                        "{} 项内容{}",
                        state.filtered.len(),
                        if catalog.data_issues.is_empty() {
                            ""
                        } else {
                            " · 部分数据未能载入"
                        }
                    )
                } else {
                    "正在整理内容…".into()
                }
            }
            UiLabel::Page => format!(
                "{} – {} / {}",
                if state.filtered.is_empty() {
                    0
                } else {
                    state.offset + 1
                },
                (state.offset + state.page_size).min(state.filtered.len()),
                state.filtered.len()
            ),
            UiLabel::Status => {
                if state.status.is_empty() {
                    if state.mode == ExperienceMode::Independent {
                        "选好内容后，Moly 会准备一处空场景和所需角色、家具。".into()
                    } else {
                        "只使用当前场景中已经在场的角色与家具。".into()
                    }
                } else {
                    state.status.clone()
                }
            }
            UiLabel::ModeDescription => {
                if state.mode == ExperienceMode::Independent {
                    "自动前往空场景，备齐这段内容需要的一切；退出后恢复原状。".into()
                } else {
                    "沿用眼前的角色、家具和位置，不改变场景。".into()
                }
            }
            UiLabel::Source => format!(
                "{} · {}",
                match catalog.source_region.as_str() {
                    "cn" => "国服来源",
                    "jp" => "日服来源",
                    _ => "未知来源",
                },
                catalog.source_version
            ),
            UiLabel::ReadyScope => {
                if state.mode == ExperienceMode::Independent {
                    "资源齐全".into()
                } else {
                    "可以体验".into()
                }
            }
            UiLabel::TransportTitle => state
                .active
                .as_ref()
                .map(|a| excerpt(&a.title, 14))
                .unwrap_or_else(|| "这段故事，随时再看".into()),
            UiLabel::TransportState => {
                if state
                    .active
                    .as_ref()
                    .is_some_and(|active| active.static_view)
                {
                    "正在查看 · 返回即恢复".into()
                } else if playing {
                    "正在体验 · 可随时停止".into()
                } else {
                    "播放已结束".into()
                }
            }
            UiLabel::Character => state
                .character
                .map(|unit| catalog.character(unit))
                .unwrap_or_else(|| "选择角色".into()),
            UiLabel::Special => {
                if state.tab == LibraryTab::Furniture {
                    "只看可互动".into()
                } else {
                    "只看家具演出".into()
                }
            }
            UiLabel::Related => state
                .related_fixture
                .map(|id| format!("{} ×", excerpt(&catalog.fixture_name(id), 14)))
                .unwrap_or_default(),
            UiLabel::Instance => {
                let i = selected_instances
                    .iter()
                    .position(|item| {
                        Some(item.target.uid.as_str()) == state.selected_uid.as_deref()
                    })
                    .unwrap_or(0);
                format!("场景中的第 {} 件 / {} 件", i + 1, selected_instances.len())
            }
            UiLabel::Availability => available.clone().unwrap_or_else(|| {
                if context::selected_instance(&state, &catalog, &world)
                    .is_some_and(|instance| instance.can_stage && !instance.ready)
                {
                    "将先安排附近的安全站位，再检查是否可以互动".into()
                } else if state.selected.is_some() {
                    "当前场景可以体验".into()
                } else {
                    String::new()
                }
            }),
            UiLabel::Play => {
                if state.pending.is_some() {
                    "正在准备…".into()
                } else if available.is_some() {
                    "暂时无法播放".into()
                } else if matches!(state.selected, Some(EntryKey::Fixture(id)) if state.mode == ExperienceMode::Independent
                    && catalog.fixture(id).is_some_and(|row| !row.interactive()))
                {
                    "在独立场景中查看".into()
                } else if matches!(state.selected, Some(EntryKey::Fixture(_))) {
                    "体验这件家具".into()
                } else if playing {
                    "切换到这段故事".into()
                } else {
                    "播放这段故事".into()
                }
            }
        };
        if **text != value {
            **text = value;
        }
    }
    for (action, kind, interaction, mut background, mut border) in &mut buttons {
        let selected = match action {
            LibraryAction::Tab(tab) => *tab == state.tab,
            LibraryAction::SetScope(scope) => *scope == state.scope,
            LibraryAction::SetMode(mode) => *mode == state.mode,
            LibraryAction::Select(key) => Some(*key) == state.selected,
            LibraryAction::SpecialFilter => state.special_only,
            LibraryAction::SetCharacter(unit) => *unit == state.character,
            LibraryAction::FocusSearch => state.search_focus,
            _ => false,
        };
        let disabled = match action {
            LibraryAction::Play => available.is_some() || state.pending.is_some(),
            LibraryAction::Stop | LibraryAction::RestoreScene => !playing,
            LibraryAction::PreviousPage => state.offset == 0,
            LibraryAction::NextPage => state.offset + state.page_size >= state.filtered.len(),
            _ => false,
        };
        background.0 = if disabled {
            Color::srgb(0.10, 0.13, 0.18)
        } else if *interaction == Interaction::Pressed {
            Color::srgb(0.14, 0.36, 0.42)
        } else if matches!(kind, ButtonKind::Primary) {
            if *interaction == Interaction::Hovered {
                Color::srgb(0.12, 0.48, 0.52)
            } else {
                Color::srgb(0.08, 0.38, 0.43)
            }
        } else if selected {
            Color::srgb(0.10, 0.25, 0.31)
        } else if *interaction == Interaction::Hovered {
            Color::srgb(0.14, 0.21, 0.29)
        } else {
            PANEL
        };
        *border = BorderColor::all(if selected {
            ACCENT
        } else if !disabled && matches!(kind, ButtonKind::Danger) {
            Color::srgb(0.40, 0.25, 0.29)
        } else {
            EDGE
        });
    }
}
