use super::*;
use crate::source_shader::{state::SourcePassState, ProgramAbi};
use serde_json::json;

fn owner(path: i64) -> Value {
    json!({"package":{"bundle":"source","sha256":"a".repeat(64),"bytes":1},"serializedFile":"archive","pathId":path.to_string()})
}
fn content(file: &str) -> Value {
    json!({"file":file,"sha256":"b".repeat(64),"bytes":1})
}
fn fixture() -> (Value, SourceShaderCatalogue) {
    let mut shader = content("shaders/source.shader.json");
    shader["source"] = owner(51);
    shader["variants"] = content("shaders/source.variants.json");
    let document = json!({
        "sourceMaterial":{"schemaVersion":1,"source":owner(42),"shaderPointer":{"m_FileID":2,"m_PathID":51},
            "savedProperties":{"m_Floats":[["_Gain",2.0]],"m_Ints":[],
                "m_Colors":[["_Vector",{"r":0.1,"g":0.2,"b":0.3,"a":0.4}]],
                "m_TexEnvs":[["_Map",{"m_Texture":{"m_FileID":0,"m_PathID":0},
                    "m_Scale":{"x":2.0,"y":3.0},"m_Offset":{"x":0.25,"y":0.5}}]]},
            "validKeywords":["LOCAL"],"invalidKeywords":["REMOVED"],"disabledShaderPasses":[],"customRenderQueue":2996},
        "shaderProgram":shader,
        "textureSources":{"_Map":{"pointer":{"m_FileID":0,"m_PathID":0},"scale":{"x":2.0,"y":3.0},
            "offset":{"x":0.25,"y":0.5},"status":"unassigned"}}
    });
    let catalogue=serde_json::from_value(json!({"schemaVersion":1,"source":owner(51),
        "sourceDocument":content("shaders/source.shader.json"),"name":"arbitrary",
        "properties":[
            {"m_Name":"_Gain","m_Type":2,"m_DefValue[0]":1.0},
            {"m_Name":"_Other","m_Type":2,"m_DefValue[0]":0.75},
            {"m_Name":"_Vector","m_Type":1,"m_DefValue[0]":1.0,"m_DefValue[1]":1.0,"m_DefValue[2]":1.0,"m_DefValue[3]":1.0},
            {"m_Name":"_Map","m_Type":4}],"passes":[],"sourceErrors":[]})).unwrap();
    (document, catalogue)
}
fn field(name: &str, kind: &str) -> UniformField {
    serde_json::from_value(json!({"name":name,"type":kind,"offset":0,"bytes":16,"member":"moly_slot_0","array":null,"stages":["vertex"]})).unwrap()
}
#[test]
fn material_properties_and_shader_defaults_do_not_supply_unknown_globals() {
    let (document, catalogue) = fixture();
    let material = MaterialSnapshot::parse(&document).unwrap();
    let value = |name, kind| match material
        .uniform(&catalogue, &field(name, kind))
        .unwrap()
        .unwrap()
    {
        UniformValue::Float(v) => v,
        _ => panic!("wrong type"),
    };
    assert_eq!(value("_Gain", "float"), [2.0]);
    assert_eq!(value("_Other", "float"), [0.75]);
    assert_eq!(value("_Vector", "vec3"), [0.1, 0.2, 0.3]);
    assert_eq!(value("_Map_ST", "vec4"), [2.0, 3.0, 0.25, 0.5]);
    assert!(material
        .uniform(&catalogue, &field("_FrameWriter", "vec4"))
        .unwrap()
        .is_none());
    assert_eq!(
        material
            .uniform(&catalogue, &field("_Gain", "int"))
            .unwrap(),
        Some(UniformValue::Signed(vec![2]))
    );
    assert!(material
        .uniform(&catalogue, &field("_Gain", "vec2"))
        .is_err());
    assert!(material
        .uniform(&catalogue, &field("_Map_ST", "vec2"))
        .is_err());
    assert_eq!(material.keywords, ["LOCAL"]);
    assert_eq!(material.custom_render_queue, 2996);
}
#[test]
fn source_owner_and_property_sheet_integrity_precede_binding() {
    let (document, catalogue) = fixture();
    let material = MaterialSnapshot::parse(&document).unwrap();
    let mut wrong = catalogue.clone();
    wrong.source.path_id = "52".into();
    assert!(material.uniform(&wrong, &field("_Gain", "float")).is_err());
    let mut wrong = document.clone();
    wrong["sourceMaterial"]["shaderPointer"]["m_PathID"] = json!(52);
    assert!(MaterialSnapshot::parse(&wrong).is_err());
    let mut wrong = document.clone();
    wrong["sourceMaterial"]["savedProperties"]["m_Floats"]
        .as_array_mut()
        .unwrap()
        .push(json!(["_Gain", 3.0]));
    assert!(MaterialSnapshot::parse(&wrong).is_err());
    let mut wrong = document.clone();
    wrong["textureSources"]["_Map"]["scale"]["x"] = json!(1.0);
    assert!(MaterialSnapshot::parse(&wrong).is_err());
    let mut wrong = document.clone();
    wrong["sourceMaterial"]["validKeywords"] = json!(["LOCAL", "LOCAL"]);
    assert!(MaterialSnapshot::parse(&wrong).is_err());
}

#[test]
#[ignore = "requires a caller-supplied current source material corpus"]
fn supplied_source_material_binding_corpus() {
    let root = std::path::PathBuf::from(
        std::env::var_os("MOLY_SOURCE_SHADER_CORPUS").expect("MOLY_SOURCE_SHADER_CORPUS"),
    );
    fn load(path: impl AsRef<std::path::Path>) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }
    fn visit(value: &Value, materials: &mut BTreeMap<String, Value>) {
        match value {
            Value::Object(object) => {
                if object.contains_key("sourceMaterial") {
                    materials.insert(value["sourceMaterial"]["source"].to_string(), value.clone());
                }
                for value in object.values() {
                    visit(value, materials);
                }
            }
            Value::Array(values) => {
                for value in values {
                    visit(value, materials);
                }
            }
            _ => {}
        }
    }
    let mut materials = BTreeMap::new();
    for tier in std::fs::read_dir(&root)
        .unwrap()
        .map(|x| x.unwrap().path())
        .filter(|p| p.is_dir())
    {
        let fx = tier.join("fx");
        if !fx.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(fx)
            .unwrap()
            .map(|x| x.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
        {
            visit(&load(file), &mut materials);
        }
    }
    assert!(!materials.is_empty());
    let mut programs = 0;
    let mut bound = 0;
    let mut external = BTreeMap::<String, usize>::new();
    for document in materials.values() {
        let material = MaterialSnapshot::parse(document)
            .unwrap_or_else(|e| panic!("{}: {e}", document["name"]));
        let catalogue = SourceShaderCatalogue::parse(
            &std::fs::read(root.join(&material.shader.variants.file)).unwrap(),
        )
        .unwrap();
        material.matches(&catalogue).unwrap();
        for pass in &catalogue.passes {
            let matches: Vec<_> = pass
                .variants
                .iter()
                .filter(|v| {
                    v.reference.gpu_program_type == 4
                        && v.status == "converted"
                        && v.keywords
                            .iter()
                            .chain(&v.local_keywords)
                            .collect::<BTreeSet<_>>()
                            == material.keywords.iter().collect::<BTreeSet<_>>()
                })
                .collect();
            if matches.is_empty() {
                continue;
            }
            assert_eq!(matches.len(), 1, "ambiguous exact source program");
            let receipt = load(root.join(&matches[0].conversion.as_ref().unwrap().file));
            let abi: ProgramAbi = serde_json::from_value(receipt["abi"].clone()).unwrap();
            abi.validate().unwrap();
            let _state =
                SourcePassState::resolve(pass, |name| material.scalar_property(&catalogue, name))
                    .unwrap_or_else(|e| {
                        panic!("{} pass {}: {e}", document["name"], pass.pass_index)
                    });
            for field in &abi.uniforms {
                match material
                    .uniform(&catalogue, field)
                    .unwrap_or_else(|e| panic!("{} uniform {}: {e}", document["name"], field.name))
                {
                    Some(_) => bound += 1,
                    None => *external.entry(field.name.clone()).or_default() += 1,
                }
            }
            for field in &abi.textures {
                crate::source_shader::sampler::CompiledTextureSlot::resolve(
                    4,
                    receipt["source"].get("parameters"),
                    field,
                )
                .unwrap_or_else(|e| panic!("{} texture {}: {e}", document["name"], field.name));
            }
            programs += 1;
        }
    }
    assert!(programs > 0 && bound > 0 && !external.is_empty());
    println!("source material bindings: {} material identities, {programs} exact programs, {bound} material inputs; external writer requirements={external:?}",materials.len());
}

#[test]
fn legacy_float_to_int_follows_native_upload_not_bits_or_round_to_nearest() {
    // Current ARM64 GLES ApplyFloat, scalar int target, observed at the native
    // constant-buffer upload boundary. Source values are f32 before conversion.
    for (bits, expected) in [
        (0x00000000, 0),
        (0x80000000, 0),
        (0x3f000000, 0),
        (0x3f7d70a4, 0),
        (0x3fc00000, 1),
        (0x3ffeb852, 1),
        (0xbf000000, 0),
        (0xbfc00000, -1),
        (0xbffeb852, -1),
        (0x4effffff, 2147483520),
        (0x4f000000, i32::MAX),
        (0xcf000000, i32::MIN),
        (0xcf000001, i32::MIN),
        (0x7149f2ca, i32::MAX),
        (0xf149f2ca, i32::MIN),
    ] {
        let (mut document, catalogue) = fixture();
        document["sourceMaterial"]["savedProperties"]["m_Floats"][0][1] =
            json!(f32::from_bits(bits));
        let material = MaterialSnapshot::parse(&document).unwrap();
        assert_eq!(
            material
                .uniform(&catalogue, &field("_Gain", "int"))
                .unwrap(),
            Some(UniformValue::Signed(vec![expected])),
            "{bits:08x}"
        );
    }
}

#[test]
#[ignore = "requires independently captured current native scalar-upload results"]
fn supplied_native_scalar_upload_corpus() {
    let path = std::env::var_os("MOLY_NATIVE_SCALAR_CORPUS").expect("MOLY_NATIVE_SCALAR_CORPUS");
    let corpus: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let rows = corpus["cases"].as_array().unwrap();
    assert!(!rows.is_empty());
    let mut finite_cases = 0;
    for row in rows {
        let bits = u32::from_str_radix(row["inputBits"].as_str().unwrap(), 16).unwrap();
        let value = f32::from_bits(bits);
        // The public typed contract rejects nonfinite inputs rather than
        // allowing their native neutral/saturated outputs to hide bad assets.
        if !value.is_finite() {
            continue;
        }
        let (mut document, catalogue) = fixture();
        document["sourceMaterial"]["savedProperties"]["m_Floats"][0][1] = json!(value);
        let material = MaterialSnapshot::parse(&document).unwrap();
        assert_eq!(
            material
                .uniform(&catalogue, &field("_Gain", "int"))
                .unwrap(),
            Some(UniformValue::Signed(vec![
                row["signedOutput"].as_i64().unwrap() as i32
            ])),
            "{bits:08x}"
        );
        finite_cases += 1;
    }
    assert!(finite_cases > 0);
    println!("current native scalar upload: {finite_cases} finite cases agree exactly");
}

#[test]
fn inactive_nonfinite_source_components_are_preserved_not_neutralized() {
    let (mut document, catalogue) = fixture();
    document["sourceMaterial"]["savedProperties"]["m_Colors"][0][1]["a"] = json!("Infinity");
    let material = MaterialSnapshot::parse(&document).unwrap();
    assert_eq!(
        material.source_material["savedProperties"]["m_Colors"][0][1]["a"],
        "Infinity"
    );
    assert_eq!(
        material
            .uniform(&catalogue, &field("_Vector", "vec3"))
            .unwrap(),
        Some(UniformValue::Float(vec![0.1, 0.2, 0.3]))
    );
    assert!(material
        .uniform(&catalogue, &field("_Vector", "vec4"))
        .unwrap_err()
        .0
        .contains("nonfinite"));
    document["sourceMaterial"]["savedProperties"]["m_Colors"][0][1]["a"] = json!("unknown-number");
    assert!(MaterialSnapshot::parse(&document).is_err());
}
