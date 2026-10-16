//! Runtime RoadCell / RoadNativeArrayCache rules, not the editor property block.
//! Neighbor order follows RoadCell.NeighbourData: L,R,U,D,LU,RU,LD,RD.

#[derive(Debug, Clone, Copy)]
pub struct Quarter {
    pub connections: [f32; 4], // X+, X-, Y+, Y-
    pub alpha_clip: bool,
    pub alpha_tiling_offset: [f32; 4],
}

pub const NEIGHBOR_OFFSETS: [(i8, i8); 8] = [
    (-1, 0), (1, 0), (0, 1), (0, -1), (-1, 1), (1, 1), (-1, -1), (1, -1),
];

/// RoadView cell types: min, min+Z, min+X, min+X+Z.
pub const CELL_OFFSETS: [(i8, i8); 4] = [(0, 0), (0, 1), (1, 0), (1, 1)];

pub fn quarter(neighbors: [bool; 8], material_index: usize) -> Quarter {
    let [l, r, u, d, lu, ru, ld, rd] = neighbors;
    // Material indices RD, LD, RU, LU. TextureTilingOffsetLibrary.Get's
    // fill axes rotate with each authored quadrant's UV orientation.
    let (x_fill, y_fill, diagonal) = match material_index {
        0 => (d, r, rd), 1 => (l, d, ld), 2 => (r, u, ru), 3 => (u, l, lu),
        _ => panic!("Road material quadrant must be 0..3"),
    };
    let offset = match (x_fill, y_fill) {
        (false, false) => [0.0, 0.0], (true, false) => [0.0, 0.5],
        (false, true) => [0.5, 0.5], (true, true) => [0.5, 0.0],
    };
    Quarter {
        connections: [r as u8 as f32, l as u8 as f32, u as u8 as f32, d as u8 as f32],
        alpha_clip: !(x_fill && y_fill && diagonal),
        alpha_tiling_offset: [0.5, 0.5, offset[0], offset[1]],
    }
}
