use naga::back::glsl;
fn compile_glsl(source:&str)->Result<String,String> {
    let module=naga::front::wgsl::parse_str(source).map_err(|e|e.emit_to_string(source))?;
    let info=naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all())
        .validate(&module).map_err(|e|format!("{e:?}"))?;
    let options=glsl::Options {version:glsl::Version::Embedded {version:300,is_webgl:true},..Default::default()};
    let pipeline=glsl::PipelineOptions {shader_stage:naga::ShaderStage::Fragment,entry_point:"fragment".into(),multiview:None};
    let mut output=String::new();
    glsl::Writer::new(&mut output,&module,&info,&options,&pipeline,naga::proc::BoundsCheckPolicies::default())
        .map_err(|e|e.to_string())?.write().map_err(|e|e.to_string())?;
    Ok(output)
}
#[test]
fn actual_raw_depth_helper_compiles_to_non_comparison_glsl_texel_fetch() {
    let shader=include_str!("shaders/uber_particle.wgsl");
    let helper=shader.split("// BEGIN RAW DEPTH LOAD CONTRACT").nth(1).unwrap()
        .split("// END RAW DEPTH LOAD CONTRACT").next().unwrap();
    let source=format!("{helper}\n@group(0) @binding(0) var depth:texture_2d<f32>;\n@fragment fn fragment(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{{ return vec4<f32>(load_depth_at_pixel(depth,p.xy)); }}");
    let glsl=compile_glsl(&source).expect("The actual depth helper must compile through the WebGL2 backend");
    assert!(glsl.contains("texelFetch"),"{glsl}");
    assert!(!glsl.contains("Shadow"),"Raw depth must not become a comparison: {glsl}");
    assert!(!glsl.contains("textureLod("),"Raw depth must not be filtered: {glsl}");
}
#[test]
fn regression_unaliased_depth_load_is_rejected_by_the_same_glsl_compiler() {
    let failure=compile_glsl("@group(0) @binding(0) var depth:texture_depth_2d; @fragment fn fragment(@builtin(position) p:vec4<f32>)->@location(0) vec4<f32>{return vec4<f32>(textureLoad(depth,vec2<i32>(p.xy),0));}").unwrap_err();
    assert!(failure.contains("textureLoad")&&failure.contains("depth textures"),"{failure}");
}
#[test]
fn raw_depth_layout_is_unfilterable_and_missing_depth_discards() {
    use bevy::render::render_resource::*;
    let layout=super::raw_depth_layout();
    assert_eq!(layout.entries[0].ty,BindingType::Texture {sample_type:TextureSampleType::Float {filterable:false},view_dimension:TextureViewDimension::D2,multisampled:false});
    let shader=include_str!("shaders/uber_particle.wgsl");
    assert_eq!(shader.matches("if depth_available.x == 0u { discard; }").count(),2);
    assert!(!shader.contains("colour.a = 0.0;"));
}

#[test]
fn actual_snapshot_shader_writes_raw_depth_without_colour_or_comparison() {
    let source = include_str!("shaders/weather_depth_copy.wgsl");
    let glsl = compile_glsl(source).expect("Actual snapshot shader must compile for GLSL ES 300");
    assert!(glsl.contains("texelFetch"), "{glsl}");
    assert!(glsl.contains("gl_FragDepth"), "{glsl}");
    assert!(!glsl.contains("Shadow"), "Snapshot must not compare: {glsl}");
    assert!(!glsl.contains("textureLod("), "Snapshot must not filter: {glsl}");
}
