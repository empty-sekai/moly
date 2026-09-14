use super::*;
use crate::site::FloorGridLayout;
use bevy::ecs::system::RunSystemOnce;

fn floor() -> FloorGridLayout {
    FloorGridLayout {
        level: 1,
        layout_id: 1,
        width: 64,
        height: 16,
        depth: 64,
    }
}

fn rectangle(center: GridPosition, direction: Direction) -> EditableFixture {
    EditableFixture {
        uid: "synthetic-rectangle".into(),
        package: "mysekai__fixture__synthetic_rectangle".into(),
        texture_id: 1,
        fixture_id: 1,
        center,
        grid_size: Vector3Int { x: 1, y: 1, z: 2 },
        layout: layout_type::FLOOR,
        direction,
    }
}

#[test]
fn rotated_candidates_reject_grid_overflow_without_panicking() {
    let overflow = rectangle(GridPosition::new(127, 0, 0), Direction::Left);
    assert!(overflow.footprint().is_err());
    for (x, direction) in [
        (127, Direction::Left),
        (126, Direction::Left),
        (127, Direction::Front),
    ] {
        let candidate = rectangle(GridPosition::new(x, 0, 0), direction);
        if x == 126 || direction == Direction::Front {
            assert!(candidate.footprint().is_ok());
        }
        assert!(matches!(
            validation::put(&candidate, &[], Some(floor())),
            PutStatus::OutOfBounds { .. }
        ));
    }
    for direction in [
        Direction::Front,
        Direction::Left,
        Direction::Back,
        Direction::Right,
    ] {
        assert_eq!(
            validation::put(
                &rectangle(GridPosition::ZERO, direction),
                &[],
                Some(floor())
            ),
            PutStatus::Ok
        );
        for (x, z) in [(i8::MIN, 0), (i8::MAX, 0), (0, i8::MIN), (0, i8::MAX)] {
            for (width, depth) in [(1, 2), (2, 1), (3, 5)] {
                let mut candidate = rectangle(GridPosition::new(x, 0, z), direction);
                candidate.grid_size.x = width;
                candidate.grid_size.z = depth;
                assert!(matches!(
                    validation::put(&candidate, &[], Some(floor())),
                    PutStatus::OutOfBounds { .. }
                ));
            }
        }
    }
}

#[test]
fn malformed_draft_rows_return_layout_errors() {
    let valid = rectangle(GridPosition::ZERO, Direction::Front);
    let mut invalid = rectangle(GridPosition::new(127, 0, 0), Direction::Left);
    invalid.uid = "invalid-neighbor".into();
    assert!(matches!(
        validation::put(&valid, &[invalid.clone()], Some(floor())),
        PutStatus::OutOfBounds { .. }
    ));
    let areas = FixtureAreas {
        loaded: true,
        ..Default::default()
    };
    assert!(validation::save(&[invalid], Some(floor()), &areas)
        .unwrap_err()
        .starts_with("ErrorLayout(1)"));
    assert!(validation::save(&[valid], Some(floor()), &areas).is_ok());
}

#[test]
fn invalid_candidate_save_preserves_storage() {
    // The child owns its settings path, so this check cannot redirect storage
    // for another concurrently running unit test.
    if std::env::var_os("MOLY_EDITOR_TEST_CHILD").is_none() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let original = b"{\"unrelated\":{\"keep\":true}}";
        std::fs::write(&path, original).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "fixture_edit::tests::invalid_candidate_save_preserves_storage",
                "--nocapture",
            ])
            .env("MOLY_EDITOR_TEST_CHILD", "1")
            .env("MOLY_SETTINGS_FILE", &path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read(path).unwrap(), original);
        return;
    }

    let original = rectangle(GridPosition::ZERO, Direction::Front);
    let baseline = FixturePlacements::test_layout(&[original.clone()], floor());
    let mut session = EditSession {
        baseline: Some(baseline.clone()),
        rows: vec![original.clone()],
        phase: EditPhase::Browsing,
        revision: 1,
        ..Default::default()
    };
    let mut world = World::new();
    world.insert_resource(baseline.clone());
    world.insert_resource(FixtureAreas {
        loaded: true,
        ..Default::default()
    });
    world.init_resource::<EditView>();

    apply_command(
        &mut world,
        &mut session,
        EditCommand::SelectPlaced {
            uid: original.uid.clone(),
        },
    );
    apply_command(
        &mut world,
        &mut session,
        EditCommand::MoveTo {
            center: GridPosition::new(126, 0, 0),
        },
    );
    apply_command(&mut world, &mut session, EditCommand::Nudge { x: 1, z: 0 });
    apply_command(&mut world, &mut session, EditCommand::Rotate);
    world.insert_resource(session);
    world.run_system_once(publish_view).unwrap();
    assert!(matches!(
        world
            .resource::<EditView>()
            .selected
            .as_ref()
            .unwrap()
            .put_status,
        PutStatus::OutOfBounds { .. }
    ));
    assert!(!world.resource::<EditView>().can_save);
    let mut session = world.remove_resource::<EditSession>().unwrap();

    apply_command(&mut world, &mut session, EditCommand::Decide);
    apply_command(&mut world, &mut session, EditCommand::Save);
    assert!(session.selected.is_some());
    assert_eq!(session.rows, vec![original.clone()]);
    assert_eq!(
        world.resource::<FixturePlacements>().editor_rows(),
        vec![original.clone()]
    );

    apply_command(&mut world, &mut session, EditCommand::Cancel);
    assert!(session.selected.is_none());
    assert_eq!(session.rows, vec![original.clone()]);
    apply_command(
        &mut world,
        &mut session,
        EditCommand::SelectPlaced {
            uid: original.uid.clone(),
        },
    );
    apply_command(
        &mut world,
        &mut session,
        EditCommand::MoveTo {
            center: GridPosition::new(127, 0, 0),
        },
    );
    apply_command(&mut world, &mut session, EditCommand::Rotate);
    apply_command(
        &mut world,
        &mut session,
        EditCommand::MoveTo {
            center: GridPosition::ZERO,
        },
    );
    apply_command(&mut world, &mut session, EditCommand::Decide);
    assert!(session.selected.is_none());
    assert_eq!(session.rows[0].direction, Direction::Left);

    session.rows[0].center = GridPosition::new(127, 0, 0);
    apply_command(&mut world, &mut session, EditCommand::Save);
    assert!(session.feedback.contains("ErrorLayout(1)"));
    assert_eq!(session.rows[0].center.x, 127);
    assert_eq!(
        world.resource::<FixturePlacements>().editor_rows(),
        vec![original]
    );
}
