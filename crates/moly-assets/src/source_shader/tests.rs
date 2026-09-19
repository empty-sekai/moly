use super::*;
use serde_json::json;

fn abi() -> ProgramAbi {
    serde_json::from_value(json!({
        "version":1,"bindGroup":0,"uniformBinding":0,"uniformBytes":96,
        "uniforms":[
            {"name":"vector","type":"vec3","array":null,"offset":0,"bytes":16,"member":"a","stages":["vertex"]},
            {"name":"signed","type":"int","array":null,"offset":16,"bytes":16,"member":"b","stages":["fragment"]},
            {"name":"matrix","type":"vec4","array":4,"offset":32,"bytes":64,"member":"c","stages":["vertex"]}
        ],"textures":[],"interfaces":{"vertex":[],"fragment":[]},
        "sourceClipSpace":"gl-minus-one-to-one","textureDataOrigin":"unity-lower-left",
        "arrayLayerRule":"clamp(floor(layer+0.5),0,layers-1)"
    })).unwrap()
}
#[test]
fn numeric_uniform_bytes_preserve_padding_sign_and_matrix_columns() {
    let packed = abi()
        .pack_uniforms(|field| {
            Ok(match field.name.as_str() {
                "vector" => UniformValue::Float(vec![1.0, -2.0, 3.5]),
                "signed" => UniformValue::Signed(vec![-7]),
                "matrix" => UniformValue::Float((1..=16).map(|i| i as f32).collect()),
                _ => unreachable!(),
            })
        })
        .unwrap();
    assert_eq!(&packed[0..4], &1.0f32.to_le_bytes());
    assert_eq!(&packed[4..8], &(-2.0f32).to_le_bytes());
    assert_eq!(&packed[12..16], &[0; 4]);
    assert_eq!(&packed[16..20], &(-7i32).to_le_bytes());
    assert_eq!(&packed[20..32], &[0; 12]);
    for i in 0..16 {
        assert_eq!(
            &packed[32 + i * 4..36 + i * 4],
            &((i + 1) as f32).to_le_bytes()
        );
    }
}
#[test]
fn uniform_inputs_are_not_defaulted_truncated_or_reinterpreted() {
    let layout = abi();
    assert!(layout
        .pack_uniforms(|_| Err(SourceShaderError("missing owner".into())))
        .unwrap_err()
        .0
        .contains("missing owner"));
    for value in [
        UniformValue::Float(vec![1.0]),
        UniformValue::Float(vec![f32::NAN, 1.0, 2.0]),
        UniformValue::Unsigned(vec![1, 2, 3]),
    ] {
        assert!(layout.pack_uniforms(|_| Ok(value.clone())).is_err());
    }
    let mut overlap = abi();
    overlap.uniforms[1].offset = 0;
    assert!(overlap.validate().is_err());
    let mut span = abi();
    span.uniform_bytes += 16;
    assert!(span.validate().is_err());
    let mut duplicate = abi();
    duplicate.uniforms[1].name = "vector".into();
    assert!(duplicate.validate().is_err());
}
#[test]
fn content_references_require_both_size_and_hash_and_cannot_escape() {
    let bytes = b"source bytes";
    let valid = ContentReference {
        file: "shaders/programs/code.glsl".into(),
        sha256: sha256(bytes),
        bytes: bytes.len() as u64,
    };
    valid.verify(bytes).unwrap();
    assert!(valid.verify(b"source bytex").is_err());
    assert!(valid.verify(b"source bytes!").is_err());
    for path in [
        "../outside",
        "/absolute",
        "https://remote",
        "a\\b",
        "a#label",
        "a//b",
        "a/./b",
        "a/%2e%2e/b",
    ] {
        assert!(
            ContentReference {
                file: path.into(),
                ..valid.clone()
            }
            .verify(bytes)
            .is_err(),
            "{path}"
        );
    }
}

fn catalogue() -> SourceShaderCatalogue {
    SourceShaderCatalogue::parse(&serde_json::to_vec(&json!({
        "schemaVersion":1,"source":{"package":{"sha256":"source"},"serializedFile":"file","pathId":"9007199254740993"},
        "name":"display only","sourceDocument":{"file":"shaders/source.shader.json","sha256":"a".repeat(64),"bytes":1},
        "properties":[],"sourceErrors":[],"passes":[{
            "subShaderIndex":0,"passIndex":2,"name":"pass","lightMode":"UniversalForward","passType":0,
            "renderState":{},"serializedState":{},"programBlocks":{},"variants":[{
                "reference":{"subshader":0,"pass":2,"platform":9,"record":23,"gpuProgramType":4,"programBlock":"progVertex","parameterRecord":17},
                "status":"converted","identity":"b".repeat(64),"keywords":["A"],"localKeywords":["B"],
                "conversion":{"file":"shaders/compiled/converted.program.json","sha256":"c".repeat(64),"bytes":1}
            }]
        }]
    })).unwrap()).unwrap()
}
#[test]
fn program_selection_uses_source_address_and_exact_keywords_not_names_or_code() {
    let mut c = catalogue();
    let keywords = vec!["B".into(), "A".into()];
    assert_eq!(
        c.select(0, 2, 9, 4, &keywords).unwrap().1.reference.record,
        23
    );
    c.name = Some("a completely different display name".into());
    assert!(c.select(0, 2, 9, 4, &keywords).is_ok());
    assert!(c.select(0, 2, 9, 4, &["A".into()]).is_err());
    assert!(c
        .select(0, 2, 9, 4, &["A".into(), "B".into(), "UNKNOWN".into()])
        .is_err());
    assert!(c.select(0, 0, 9, 4, &keywords).is_err());
    assert!(c.select(0, 2, 22, 4, &keywords).is_err());
    // Equal code identities never make two surviving owners unambiguous.
    let mut other = c.passes[0].variants[0].clone();
    other.reference.record += 1;
    c.passes[0].variants.push(other);
    assert!(c.select(0, 2, 9, 4, &keywords).is_err());
}

/// Replay a caller-supplied formal extraction without distributing game data.
/// Every converted receipt/texture is checked; unavailable variants retain their
/// source reason. This is an asset-contract gate, not a renderer equivalence gate.
#[test]
#[ignore = "requires a caller-supplied source shader corpus"]
fn supplied_source_shader_corpus() {
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(
        std::env::var_os("MOLY_SOURCE_SHADER_CORPUS").expect("set MOLY_SOURCE_SHADER_CORPUS"),
    );
    let read = |reference: &ContentReference| {
        let bytes = fs::read(root.join(&reference.file)).unwrap();
        reference.verify(&bytes).unwrap();
        bytes
    };
    let (mut catalogues, mut conversions, mut textures) = (0, 0, 0);
    for path in fs::read_dir(root.join("shaders"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".variants.json"))
    {
        let catalogue = SourceShaderCatalogue::parse(&fs::read(&path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let _: Value = serde_json::from_slice(&read(&catalogue.source_document)).unwrap();
        for pass in &catalogue.passes {
            for variant in pass.variants.iter().filter(|v| v.status == "converted") {
                let reference = variant.conversion.as_ref().unwrap();
                let receipt = ProgramReceipt::parse(&read(reference))
                    .unwrap_or_else(|e| panic!("{}: {e}", reference.file));
                receipt.matches(&catalogue, variant).unwrap();
                for stage in [&receipt.stages.vertex, &receipt.stages.fragment] {
                    read(&stage.shader);
                    if let Some(adapter) = &stage.renderer {
                        read(&adapter.shader);
                    }
                }
                read(&receipt.source.program.code);
                conversions += 1;
            }
        }
        catalogues += 1;
    }
    for path in fs::read_dir(root.join("shader-textures"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".texture.json"))
    {
        let texture = texture::SourceTexture::parse(&fs::read(&path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        texture
            .verify_pixels(&read(texture.pixels.as_ref().unwrap()))
            .unwrap();
        read(texture.encoded.as_ref().unwrap());
        textures += 1;
    }
    assert!(
        catalogues > 0 && conversions > 0 && textures > 0,
        "empty source corpus is not a positive control"
    );
    eprintln!("source shader corpus: {catalogues} shader identities, {conversions} converted programs, {textures} complete textures");
}
