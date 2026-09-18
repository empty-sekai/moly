//! Optional developer model preview, sharing the normal importer/material path.
//! Fence previews use the source connection law and exported part references.
//! Optional line/single/corner/cross layouts exercise the same scene path.

use super::{layout_type, Direction, GridPosition, PlacementMock};

const FENCES: [&str; 7] = [
    "mysekai__fixture__mdl_non2002_fence_bamboo1",
    "mysekai__fixture__mdl_non2002_fence_green1",
    "mysekai__fixture__mdl_non2002_fence_iron1",
    "mysekai__fixture__mdl_non2002_fence_log1",
    "mysekai__fixture__mdl_non2002_fence_rock1",
    "mysekai__fixture__mdl_non2002_fence_shrine1",
    "mysekai__fixture__mdl_non2002_fence_wood1",
];

const ROADS: [&str; 26] = [
    "mysekai__fixture__mdl_non2001_road_brick1",
    "mysekai__fixture__mdl_non2001_road_concre1",
    "mysekai__fixture__mdl_non2001_road_gravel1",
    "mysekai__fixture__mdl_non2001_road_soil1",
    "mysekai__fixture__mdl_non2001_road_stone1",
    "mysekai__fixture__mdl_non2001_road_whitesand1",
    "mysekai__fixture__mdl_non2003_road_bg1",
    "mysekai__fixture__mdl_non2003_road_bgn1",
    "mysekai__fixture__mdl_non2003_road_bk1",
    "mysekai__fixture__mdl_non2003_road_bl1",
    "mysekai__fixture__mdl_non2003_road_br1",
    "mysekai__fixture__mdl_non2003_road_gn1",
    "mysekai__fixture__mdl_non2003_road_gr1",
    "mysekai__fixture__mdl_non2003_road_lbg1",
    "mysekai__fixture__mdl_non2003_road_lbl1",
    "mysekai__fixture__mdl_non2003_road_lbr1",
    "mysekai__fixture__mdl_non2003_road_lgr1",
    "mysekai__fixture__mdl_non2003_road_lor1",
    "mysekai__fixture__mdl_non2003_road_or1",
    "mysekai__fixture__mdl_non2003_road_pk1",
    "mysekai__fixture__mdl_non2003_road_ppr1",
    "mysekai__fixture__mdl_non2003_road_pr1",
    "mysekai__fixture__mdl_non2003_road_rd1",
    "mysekai__fixture__mdl_non2003_road_wh1",
    "mysekai__fixture__mdl_non2003_road_ygn1",
    "mysekai__fixture__mdl_non2003_road_yw1",
];

// The footprints and IDs come from the same extracted master as the editor.
// Only the preview position is product-owned; no model geometry is synthesized.
const SURFACES: [(&str, [i8; 3], i32); 16] = [
    (
        "mysekai__fixture__mdl_cst0005_canvas_large1board1",
        [5, 3, 1],
        441,
    ),
    (
        "mysekai__fixture__mdl_cst0005_canvas_large1stand1",
        [2, 1, 1],
        444,
    ),
    (
        "mysekai__fixture__mdl_cst0005_canvas_medium1board1",
        [3, 3, 1],
        440,
    ),
    (
        "mysekai__fixture__mdl_cst0005_canvas_medium1stand1",
        [1, 1, 1],
        443,
    ),
    (
        "mysekai__fixture__mdl_cst0005_canvas_small1board1",
        [1, 1, 1],
        439,
    ),
    (
        "mysekai__fixture__mdl_cst0005_canvas_small1stand1",
        [1, 1, 1],
        442,
    ),
    (
        "mysekai__fixture__mdl_env0001_fixture_tree1",
        [4, 5, 4],
        471,
    ),
    (
        "mysekai__fixture__mdl_ext0020_fixture_pond1",
        [6, 3, 7],
        1023,
    ),
    (
        "mysekai__fixture__mdl_non1002_after_conifer1",
        [3, 14, 3],
        123,
    ),
    (
        "mysekai__fixture__mdl_non1002_after_hardwood1",
        [3, 14, 3],
        122,
    ),
    (
        "mysekai__fixture__mdl_non1002_after_luxurytree1",
        [4, 14, 4],
        125,
    ),
    (
        "mysekai__fixture__mdl_non1002_after_palmtree1",
        [3, 14, 3],
        124,
    ),
    (
        "mysekai__fixture__mdl_non1004_fixture_shrub1",
        [2, 2, 2],
        542,
    ),
    (
        "mysekai__fixture__mdl_non1004_fixture_shrub2",
        [2, 2, 2],
        543,
    ),
    ("mysekai__fixture__mdl_non3001_block_cl1", [1, 1, 1], 595),
    ("mysekai__fixture__mdl_non3001_block_cl2", [2, 2, 2], 616),
];

#[cfg(not(target_arch = "wasm32"))]
fn selection() -> Option<String> {
    std::env::var("MOLY_FIXTURE_PREVIEW").ok()
}

// Optional gimmick examples use the same master dimensions and import path.
// Selecting a preview does not edit the user's default furniture layout.
const GIMMICKS: [(&str, [i8; 3], i32); 6] = [
    (
        "mysekai__fixture__mdl_ext0014_fixture_synthesizer1",
        [4, 2, 2],
        297,
    ),
    (
        "mysekai__fixture__mdl_ext0014_fixture_guitar1",
        [2, 4, 2],
        298,
    ),
    (
        "mysekai__fixture__mdl_ext0014_fixture_bass1",
        [2, 4, 2],
        299,
    ),
    (
        "mysekai__fixture__mdl_ext0014_fixture_drum1",
        [5, 3, 4],
        300,
    ),
    (
        "mysekai__fixture__mdl_chr0004_fixture_nenerobo1",
        [2, 3, 2],
        423,
    ),
    (
        "mysekai__fixture__mdl_ext0002_fixture_fridge1",
        [2, 4, 2],
        157,
    ),
];

#[cfg(target_arch = "wasm32")]
fn selection() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search)
        .ok()?
        .get("fixture_preview")
}

pub(super) fn append_preview(rows: &mut Vec<PlacementMock>) {
    let Some(selection) = selection() else {
        return;
    };
    let fields: Vec<_> = selection.split('-').collect();
    if matches!(fields[0], "surface" | "gimmick") {
        let catalog: &[(&str, [i8; 3], i32)] = if fields[0] == "surface" {
            &SURFACES
        } else {
            &GIMMICKS
        };
        let index = fields
            .get(1)
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&i| i < catalog.len() && fields.len() == 2)
            .unwrap_or_else(|| {
                panic!(
                    "家具预览参数无效：{selection:?}；支持 surface-0..{} 或 gimmick-0..{}",
                    SURFACES.len() - 1,
                    GIMMICKS.len() - 1
                )
            });
        let (package, [width, height, depth], id) = catalog[index];
        let min = GridPosition {
            x: -16,
            y: 0,
            z: 14,
        };
        rows.push(PlacementMock {
            texture_id: 1,
            package,
            min,
            max: GridPosition {
                x: min.x + width - 1,
                y: height - 1,
                z: min.z + depth - 1,
            },
            center_y: 0,
            layout: layout_type::FLOOR,
            direction: Direction::Front,
            fixture_id: id,
        });
        bevy::log::info!("家具预览：{package}，master fixture {id}");
        return;
    }
    let road = fields[0] == "road";
    let catalog: &[&str] = if road { &ROADS } else { &FENCES };
    let index = fields.get(1).and_then(|s| s.parse::<usize>().ok())
        .filter(|&i| (road || fields[0] == "fence") && i < catalog.len() && fields.len() <= 3)
        .unwrap_or_else(|| panic!("家具预览参数无效：{selection:?}；支持 fence-0..6 或 road-0..25[-single/corner/cross/square]"));
    let layout = fields.get(2).copied().unwrap_or("line");
    let cells: &[(i8, i8, Direction)] = match layout {
        "single" => &[(-14, 14, Direction::Front)],
        "line" => &[
            (-16, 14, Direction::Front),
            (-14, 14, Direction::Front),
            (-12, 14, Direction::Front),
        ],
        "corner" => &[
            (-16, 14, Direction::Front),
            (-14, 14, Direction::Front),
            (-14, 16, Direction::Left),
        ],
        "square" => &[
            (-16, 14, Direction::Front),
            (-14, 14, Direction::Front),
            (-16, 16, Direction::Front),
            (-14, 16, Direction::Front),
        ],
        "cross" => &[
            (-16, 14, Direction::Front),
            (-14, 14, Direction::Front),
            (-12, 14, Direction::Front),
            (-14, 16, Direction::Left),
            (-14, 12, Direction::Left),
        ],
        _ => panic!("未知家具预览布局：{layout}"),
    };
    for &(x, z, direction) in cells {
        // CN master: all fences occupy 2x2 floor cells; green hedge is one
        // cell high (fixture117), the other six are two cells high.
        rows.push(PlacementMock {
            texture_id: 1,
            package: catalog[index],
            min: GridPosition { x, y: 0, z },
            max: GridPosition {
                x: x + 1,
                y: if road || index == 1 { 0 } else { 1 },
                z: z + 1,
            },
            center_y: 0,
            layout: if road {
                layout_type::ROAD
            } else {
                layout_type::FLOOR
            },
            direction: if road { Direction::Front } else { direction },
            fixture_id: 0,
        });
    }
    bevy::log::info!(
        "家具预览：{} {layout}，{} 个实例；道路/围栏使用源邻接规则",
        catalog[index],
        cells.len()
    );
}
