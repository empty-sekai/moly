//! The NPC branch of the view's dither alpha as the game's own code
//! computed it: the cases with no player near this NPC and no camera input,
//! each with the alpha the view wrote. Generated; do not edit by hand.

use super::{npc_dither_alpha, use_dither, DitherNeighbor};

struct Other {
    unit: u32,
    root: [f32; 3],
    hips: [f32; 3],
}

struct Case {
    name: &'static str,
    unit: u32,
    root: [f32; 3],
    hips: [f32; 3],
    others: &'static [Other],
    talking: bool,
    photo: bool,
    fixture_action: bool,
    alpha: u32,
}

const CASES: &[Case] = &[
    Case {
        name: "alone_fps",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f800000,
    },
    Case {
        name: "npc_root_0.23",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3e6b851f), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3e6b851f), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f19999a,
    },
    Case {
        name: "npc_root_0.1",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3dcccccd), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3dcccccd), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3ebf7188,
    },
    Case {
        name: "npc_hips_near_root_far",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3f000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3e99999a), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f800000,
    },
    Case {
        name: "npc_root_near_hips_far",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3dcccccd), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3ef0a3d7), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f800000,
    },
    Case {
        name: "npc_hips_exactly_0.46",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3dcccccd), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3eeb851f), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f800000,
    },
    Case {
        name: "npc_root_exactly_0.46",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3eeb851f), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3e4ccccd), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f800000,
    },
    Case {
        name: "npc_two_first_wins",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3ecccccd), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3ecccccd), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
            Other { unit: 22, root: [f32::from_bits(0x3dcccccd), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3dcccccd), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f654976,
    },
    Case {
        name: "npc_first_far_second_near",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3f800000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3f800000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
            Other { unit: 22, root: [f32::from_bits(0x3dcccccd), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3dcccccd), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3ebf7188,
    },
    Case {
        name: "npc_3d_distance",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3e99999a), f32::from_bits(0x3e99999a), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3e99999a), f32::from_bits(0x3f99999a), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f7016fa,
    },
    Case {
        name: "npc_same_unit_skipped",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 17, root: [f32::from_bits(0x3dcccccd), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3dcccccd), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f800000,
    },
    Case {
        name: "npc_near_talking",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3e6b851f), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3e6b851f), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: true,
        photo: false,
        fixture_action: false,
        alpha: 0x3f800000,
    },
    Case {
        name: "npc_near_photo",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3e6b851f), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3e6b851f), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: true,
        fixture_action: false,
        alpha: 0x3f800000,
    },
    Case {
        name: "npc_near_fixture_action",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3e6b851f), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3e6b851f), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: true,
        alpha: 0x3f800000,
    },
    Case {
        name: "npc_near_no_agent",
        unit: 17,
        root: [f32::from_bits(0x00000000), f32::from_bits(0x00000000), f32::from_bits(0x00000000)],
        hips: [f32::from_bits(0x00000000), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)],
        others: &[
            Other { unit: 21, root: [f32::from_bits(0x3e6b851f), f32::from_bits(0x00000000), f32::from_bits(0x00000000)], hips: [f32::from_bits(0x3e6b851f), f32::from_bits(0x3f666666), f32::from_bits(0x00000000)] },
        ],
        talking: false,
        photo: false,
        fixture_action: false,
        alpha: 0x3f19999a,
    },
];

/// |a - b| as the view computes it: sqrt(dz^2 + (dx^2 + dy^2)) in f32.
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    (dz * dz + (dx * dx + dy * dy)).sqrt()
}

#[test]
fn npc_branch_matches_the_game() {
    for case in CASES {
        // The caller's list: every other NPC of another unit, in list order.
        let others: Vec<DitherNeighbor> = case
            .others
            .iter()
            .filter(|other| other.unit != case.unit)
            .map(|other| DitherNeighbor {
                hips_distance: distance(other.hips, case.hips),
                root_distance: distance(other.root, case.root),
            })
            .collect();
        let alpha = npc_dither_alpha(case.talking, case.photo, case.fixture_action, &others);
        assert_eq!(alpha.to_bits(), case.alpha, "{}: alpha {alpha}", case.name);
        assert_eq!(use_dither(alpha), f32::from_bits(case.alpha) <= 0.999, "{}", case.name);
    }
}
