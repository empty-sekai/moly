//! Optional comparison against a separately executed native writer. Private
//! inputs remain outside the source repository; this test consumes its receipt.
use super::*;

#[test]
#[ignore = "requires MOLY_SOURCE_CAMERA_NATIVE_VECTORS from current native replay"]
fn source_globals_match_current_native_writer() {
    let path = std::env::var("MOLY_SOURCE_CAMERA_NATIVE_VECTORS").unwrap();
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(data["externalCalls"].as_u64(), Some(0));
    assert_eq!(data["sourceSha256"].as_str().unwrap().len(), 64);
    let rows = data["rows"].as_array().unwrap();
    let mut compared = 0;
    for row in rows {
        if row["reversed"].as_bool().unwrap() {
            continue;
        }
        let camera = Projection::Perspective(PerspectiveProjection {
            near: row["near"].as_f64().unwrap() as f32,
            far: row["far"].as_f64().unwrap() as f32,
            ..default()
        });
        let actual = gles_z_buffer_params(&camera).unwrap().map(f32::to_bits);
        let expected: Vec<u32> = row["bits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap().try_into().unwrap())
            .collect();
        assert_eq!(actual.as_slice(), expected, "{row}");
        compared += 1;
    }
    assert!(
        compared >= 16,
        "an empty or reduced native corpus cannot pass"
    );
    println!("Current native GLES camera writer: {compared} bit-exact cases");
}
