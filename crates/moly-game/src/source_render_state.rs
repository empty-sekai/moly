//! Backend translation of source-resolved Unity pass state. No material-family defaults.
//! Camera depth is reversed Z, so only ordered depth comparisons are reversed;
//! equality, blend arithmetic and the source's independent alpha factors are unchanged.
use bevy::render::render_resource::*;
use moly_assets::material_passes::SourceRenderState;

pub(crate) fn apply(pipeline: &mut RenderPipelineDescriptor, state: SourceRenderState) {
    pipeline.primitive.cull_mode = match state.cull {
        0 => None, 1 => Some(Face::Front), 2 => Some(Face::Back),
        _ => unreachable!("validated source CullMode"),
    };
    if let Some(depth) = &mut pipeline.depth_stencil {
        depth.depth_compare = reversed_compare(state.depth_test);
        // Disabled depth testing disables depth writes on the source graphics path.
        depth.depth_write_enabled = state.depth_write && state.depth_test != 0;
    }
    if let Some(fragment) = &mut pipeline.fragment {
        for target in fragment.targets.iter_mut().flatten() {
            target.write_mask = color_mask(state.color_mask);
            target.blend = Some(BlendState {
                color: BlendComponent { src_factor: factor(state.src_color), dst_factor: factor(state.dst_color), operation: operation(state.color_op) },
                alpha: BlendComponent { src_factor: factor(state.src_alpha), dst_factor: factor(state.dst_alpha), operation: operation(state.alpha_op) },
            });
        }
    }
}
fn factor(value: u8) -> BlendFactor {
    match value {
        0 => BlendFactor::Zero, 1 => BlendFactor::One,
        2 => BlendFactor::Dst, 3 => BlendFactor::Src,
        4 => BlendFactor::OneMinusDst, 5 => BlendFactor::SrcAlpha,
        6 => BlendFactor::OneMinusSrc, 7 => BlendFactor::DstAlpha,
        8 => BlendFactor::OneMinusDstAlpha, 9 => BlendFactor::SrcAlphaSaturated,
        10 => BlendFactor::OneMinusSrcAlpha,
        _ => unreachable!("validated source BlendMode"),
    }
}
fn operation(value: u8) -> BlendOperation {
    match value {
        0 => BlendOperation::Add, 1 => BlendOperation::Subtract,
        2 => BlendOperation::ReverseSubtract, 3 => BlendOperation::Min,
        4 => BlendOperation::Max, _ => unreachable!("validated source BlendOp"),
    }
}
fn reversed_compare(value: u8) -> CompareFunction {
    match value {
        0 | 8 => CompareFunction::Always, 1 => CompareFunction::Never,
        2 => CompareFunction::Greater, 3 => CompareFunction::Equal,
        4 => CompareFunction::GreaterEqual, 5 => CompareFunction::Less,
        6 => CompareFunction::NotEqual, 7 => CompareFunction::LessEqual,
        _ => unreachable!("validated source CompareFunction"),
    }
}
fn color_mask(value: u8) -> ColorWrites {
    // Unity ColorWriteMask is A=1,B=2,G=4,R=8, not wgpu's R=1,G=2,B=4,A=8.
    let mut mask=ColorWrites::empty();
    for (bit, channel) in [(8,ColorWrites::RED),(4,ColorWrites::GREEN),(2,ColorWrites::BLUE),(1,ColorWrites::ALPHA)] {
        if value & bit != 0 { mask |= channel; }
    }
    mask
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_effect_rgb_mask_does_not_write_alpha() {
        assert_eq!(color_mask(14), ColorWrites::RED|ColorWrites::GREEN|ColorWrites::BLUE);
        assert_eq!(color_mask(8),ColorWrites::RED);
        assert_eq!(color_mask(1),ColorWrites::ALPHA);
        assert_eq!(color_mask(15),ColorWrites::ALL);
    }
    #[test]
    fn source_less_equal_is_reverse_z_greater_equal() {
        assert_eq!(reversed_compare(4),CompareFunction::GreaterEqual);
        assert_eq!(reversed_compare(2),CompareFunction::Greater);
        assert_eq!(reversed_compare(7),CompareFunction::LessEqual);
        assert_eq!(reversed_compare(3),CompareFunction::Equal);
    }
    #[test]
    fn alpha_factors_are_not_replaced_with_over() {
        // The JP Uber effect pass references _BlendSrc/_BlendDst for alpha too.
        assert_eq!(factor(5),BlendFactor::SrcAlpha);
        assert_eq!(factor(10),BlendFactor::OneMinusSrcAlpha);
        assert_eq!(factor(1),BlendFactor::One);
    }
}
