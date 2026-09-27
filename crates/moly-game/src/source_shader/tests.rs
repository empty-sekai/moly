use super::*;
fn sampler() -> SourceSampler {
    SourceSampler {
        filter: 1,
        wrap_u: 0,
        wrap_v: 1,
        wrap_w: 2,
        anisotropy: 1,
        mip_bias: 0.0,
    }
}
#[test]
fn explicit_sampler_preserves_axis_modes_and_mip_filter() {
    let descriptor = sampler_descriptor(&sampler(), 4).unwrap();
    assert_eq!(descriptor.address_mode_u, AddressMode::Repeat);
    assert_eq!(descriptor.address_mode_v, AddressMode::ClampToEdge);
    assert_eq!(descriptor.address_mode_w, AddressMode::MirrorRepeat);
    assert_eq!(descriptor.mag_filter, FilterMode::Linear);
    assert_eq!(descriptor.mipmap_filter, FilterMode::Nearest);
    assert_eq!(
        (descriptor.lod_min_clamp, descriptor.lod_max_clamp),
        (0.0, 3.0)
    );
    let mut point = sampler();
    point.filter = 0;
    assert_eq!(
        sampler_descriptor(&point, 1).unwrap().min_filter,
        FilterMode::Nearest
    );
    let mut trilinear = sampler();
    trilinear.filter = 2;
    assert_eq!(
        sampler_descriptor(&trilinear, 2).unwrap().mipmap_filter,
        FilterMode::Linear
    );
}
#[test]
fn absent_sampler_capabilities_are_rejected_not_linear_clamped() {
    for change in [0, 1, 2, 3, 4] {
        let mut value = sampler();
        match change {
            0 => value.filter = 3,
            1 => value.wrap_u = 3,
            2 => value.anisotropy = 8,
            3 => value.mip_bias = 0.5,
            _ => value.mip_bias = f32::NAN,
        }
        assert!(sampler_descriptor(&value, 1).is_err());
    }
    assert!(sampler_descriptor(&sampler(), 0).is_err());
}
