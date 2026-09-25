//! Serialized UI geometry and component data, decoded once at the asset boundary.

pub mod auto_layout;
pub mod clipping;
mod runtime;
pub use runtime::UiInstance;

use bevy::math::{Mat4, Vec2, Vec4};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "UiPrefabSource")]
pub struct UiPrefab {
    pub version: u32,
    pub prefab: String,
    /// Where the prefab bytes came from. Documents extracted before the
    /// extractor tagged regions carry none of these fields.
    pub source: UiDocumentSource,
    pub nodes: Vec<UiNode>,
    indices: UiIndices,
}

/// The extractor's record of the serialized file a document was read from.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiDocumentSource {
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub client_version: Option<String>,
    #[serde(default)]
    pub unity_version: Option<String>,
    /// sha256 of the serialized file the prefab was read from.
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub resource_key: Option<String>,
}

/// A region-tagged UI root (its ui-manifest.json): one region and client
/// version, and one row per document the root carries with its sha256.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiRootManifest {
    pub version: u32,
    pub region: String,
    pub client_version: String,
    pub unity_version: String,
    #[serde(default)]
    pub failures: Vec<Value>,
    pub documents: Vec<UiRootDocument>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiRootDocument {
    pub document: String,
    pub document_sha256: String,
    pub kind: String,
    pub region: String,
    pub client_version: String,
    #[serde(default)]
    pub source_sha256: Option<String>,
    /// Node count of a prefab row, as the extractor wrote it.
    #[serde(default)]
    pub nodes: Option<usize>,
}

/// The identity a document was admitted with, for the load log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiDocumentIdentity {
    pub region: String,
    pub client_version: String,
    pub unity_version: String,
    pub document_sha256: String,
    pub source_sha256: Option<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl UiRootManifest {
    /// Parse a root's manifest and require it to be the region the runtime
    /// was built for, with no extraction failures recorded.
    pub fn parse(bytes: &str, region: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(bytes).map_err(|e| e.to_string())?;
        if manifest.version != 1 {
            return Err(format!("UI root manifest version {} is not 1", manifest.version));
        }
        if manifest.region != region {
            return Err(format!(
                "UI root manifest is region {}, the runtime is {region}",
                manifest.region
            ));
        }
        if !manifest.failures.is_empty() {
            return Err(format!("UI root manifest records {} failures", manifest.failures.len()));
        }
        Ok(manifest)
    }

    /// Admit one file of this root: it must have a manifest row of the given
    /// kind, of this region and client version, whose sha256 equals the
    /// loaded bytes.
    pub fn admit(
        &self,
        path: &str,
        kind: &str,
        bytes: &str,
    ) -> Result<(&UiRootDocument, UiDocumentIdentity), String> {
        let row = self
            .documents
            .iter()
            .find(|row| row.document == path)
            .ok_or_else(|| format!("UI root manifest has no row for {path}"))?;
        if row.kind != kind {
            return Err(format!("UI root manifest row {path} is {}, not {kind}", row.kind));
        }
        if row.region != self.region || row.client_version != self.client_version {
            return Err(format!(
                "UI root manifest row {path} is not {} {}",
                self.region, self.client_version
            ));
        }
        let document_sha256 = sha256_hex(bytes.as_bytes());
        if document_sha256 != row.document_sha256 {
            return Err(format!(
                "UI document {path} sha256 {document_sha256} differs from its manifest row {}",
                row.document_sha256
            ));
        }
        let identity = UiDocumentIdentity {
            region: self.region.clone(),
            client_version: self.client_version.clone(),
            unity_version: self.unity_version.clone(),
            document_sha256,
            source_sha256: row.source_sha256.clone(),
        };
        Ok((row, identity))
    }

    /// Admit a layout document of this root: it must have a prefab row of
    /// this region and client version whose sha256 equals the loaded bytes,
    /// and the document's own source record must agree with the row.
    pub fn admit_prefab(
        &self,
        path: &str,
        bytes: &str,
        doc: &UiPrefab,
    ) -> Result<UiDocumentIdentity, String> {
        let (row, identity) = self.admit(path, "prefab", bytes)?;
        let document_sha256 = identity.document_sha256;
        let source = &doc.source;
        if source.region.as_deref() != Some(self.region.as_str())
            || source.client_version.as_deref() != Some(self.client_version.as_str())
            || source.unity_version.as_deref() != Some(self.unity_version.as_str())
        {
            return Err(format!(
                "UI document {path} source is not {} {}",
                self.region, self.client_version
            ));
        }
        if row.nodes.is_some_and(|nodes| nodes != doc.nodes.len()) {
            return Err(format!(
                "UI document {path} has {} nodes, its manifest row {:?}",
                doc.nodes.len(),
                row.nodes
            ));
        }
        if row.source_sha256.is_some() && source.sha256 != row.source_sha256 {
            return Err(format!(
                "UI document {path} serialized-file sha256 differs from its manifest row"
            ));
        }
        Ok(UiDocumentIdentity {
            region: self.region.clone(),
            client_version: self.client_version.clone(),
            unity_version: self.unity_version.clone(),
            document_sha256,
            source_sha256: source.sha256.clone(),
        })
    }
}

#[derive(Debug, Clone, Default)]
struct UiIndices {
    identities: HashMap<i64, usize>,
    suffixes: HashMap<String, (usize, usize)>,
    parents: Vec<Option<usize>>,
}

#[derive(Deserialize)]
struct UiPrefabSource {
    version: u32,
    prefab: String,
    #[serde(default)]
    source: UiDocumentSource,
    nodes: Vec<UiNode>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiNode {
    pub path: String,
    pub name: String,
    pub game_object_id: i64,
    pub transform_id: i64,
    pub parent_transform_id: i64,
    pub active: bool,
    pub rect: RectTransform,
    pub components: Vec<UiComponent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectTransform {
    pub anchors_min: [f32; 2],
    pub anchors_max: [f32; 2],
    pub anchored_position: [f32; 2],
    pub size_delta: [f32; 2],
    pub pivot: [f32; 2],
    pub local_scale: [f32; 3],
    pub local_rotation: [f32; 4],
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiComponent {
    pub path_id: i64,
    pub class: String,
    pub enabled: bool,
    #[serde(default)]
    pub fields: Value,
    /// Exact PPtr locations relative to fields, emitted before tuple-to-JSON
    /// conversion. None identifies an older/opaque component, not zero PPtrs.
    #[serde(default)]
    pub pointer_fields: Option<Vec<String>>,
    #[serde(default)]
    pub clip_material: Option<clipping::UiClipMaterial>,
    #[serde(default)]
    pub font: Option<UiTextFont>,
    pub sprite: Option<Value>,
    pub texture: Option<Value>,
    #[serde(default)]
    pub canvas_reference_pixels_per_unit: Option<f32>,
}

/// Source TMP font metadata. Bitmap glyph production is a separate concern.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiTextFont {
    #[serde(default)]
    pub source_face: Option<UiTextFace>,
    /// The serialized file and path id `m_fontAsset` resolves to.
    #[serde(default)]
    pub serialized_file: Option<String>,
    #[serde(default)]
    pub path_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiTextFace {
    pub point_size: f32,
    pub scale: f32,
    pub line_height: f32,
    pub ascent_line: f32,
    pub descent_line: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiRect {
    pub size: Vec2,
    pub pivot: Vec2,
    pub world: Mat4,
    pub active: bool,
    pub clipping: clipping::UiClipping,
}

impl UiRect {
    pub fn center(&self) -> Vec2 {
        self.world
            .transform_point3(((Vec2::splat(0.5) - self.pivot) * self.size).extend(0.))
            .truncate()
    }

    pub fn contains(&self, point: Vec2) -> bool {
        if !self.clipping.contains(point) {
            return false;
        }
        if self.world.determinant().abs() < f32::EPSILON {
            return false;
        }
        let local = self
            .world
            .inverse()
            .transform_point3(point.extend(0.))
            .truncate();
        let min = -self.size * self.pivot;
        local.cmpge(min).all() && local.cmple(min + self.size).all()
    }
}

impl TryFrom<UiPrefabSource> for UiPrefab {
    type Error = String;

    fn try_from(value: UiPrefabSource) -> Result<Self, Self::Error> {
        if value.version < 2 {
            return Err(
                "UI layout requires serialized activation and corrected image fields".into(),
            );
        }
        let mut ids = HashMap::new();
        let mut indices = UiIndices::default();
        for (i, node) in value.nodes.iter().enumerate() {
            let parent = ids.get(&node.parent_transform_id).copied();
            if i > 0 && parent.is_none() {
                return Err(format!("UI parent missing for {}", node.path));
            }
            if ids.insert(node.transform_id, i).is_some() {
                return Err(format!("duplicate UI transform {}", node.transform_id));
            }
            indices.parents.push(parent);
            for id in [node.game_object_id, node.transform_id]
                .into_iter()
                .chain(node.components.iter().map(|component| component.path_id))
            {
                indices.identities.entry(id).or_insert(i);
            }
            for start in std::iter::once(0).chain(node.path.match_indices('/').map(|(i, _)| i + 1))
            {
                let entry = indices
                    .suffixes
                    .entry(node.path[start..].to_owned())
                    .or_insert((i, 0));
                entry.1 += 1;
            }
            if node
                .rect
                .local_scale
                .iter()
                .chain(node.rect.local_rotation.iter())
                .chain(node.rect.anchored_position.iter())
                .chain(node.rect.size_delta.iter())
                .any(|n| !n.is_finite())
            {
                return Err(format!("non-finite UI geometry {}", node.path));
            }
        }
        Ok(Self {
            version: value.version,
            prefab: value.prefab,
            source: value.source,
            nodes: value.nodes,
            indices,
        })
    }
}

/// The size of a RectTransform's rect as the engine solves it: the anchor
/// references are the parent rect's corner (its pivot times minus its size)
/// plus the parent size scaled by each anchor, and the size is the size delta
/// plus the distance between them.
pub(crate) fn rect_size(parent_size: Vec2, parent_pivot: Vec2, rect: &RectTransform) -> Vec2 {
    let corner = parent_pivot * -parent_size;
    let reference_min = corner + parent_size * Vec2::from_array(rect.anchors_min);
    let reference_max = corner + parent_size * Vec2::from_array(rect.anchors_max);
    Vec2::from_array(rect.size_delta) + (reference_max - reference_min)
}

/// The engine's local position of a RectTransform in its parent (the
/// parent's pivot at the origin): `(refMin + anchoredPosition) +
/// (refMax - refMin) * pivot`, with `refMin`/`refMax` the anchors placed on
/// the parent rect (`parentPivot * -parentSize + parentSize * anchor`).
pub(crate) fn local_position(parent_size: Vec2, parent_pivot: Vec2, rect: &RectTransform) -> Vec2 {
    let corner = parent_pivot * -parent_size;
    let reference_min = corner + parent_size * Vec2::from_array(rect.anchors_min);
    let reference_max = corner + parent_size * Vec2::from_array(rect.anchors_max);
    (reference_min + Vec2::from_array(rect.anchored_position))
        + (reference_max - reference_min) * Vec2::from_array(rect.pivot)
}

/// One transform's local translation, rotation and scale as the engine
/// stores them.
#[derive(Clone, Copy)]
struct LocalTrs {
    position: [f32; 4],
    rotation: [f32; 4],
    scale: [f32; 3],
}

type Lanes = [f32; 4];

fn lanes_mul(a: Lanes, b: Lanes) -> Lanes {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

fn lanes_add(a: Lanes, b: Lanes) -> Lanes {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

fn lanes_scale(a: Lanes, s: f32) -> Lanes {
    [a[0] * s, a[1] * s, a[2] * s, a[3] * s]
}

/// The engine's rotation-and-scale columns of one transform: the rotation
/// matrix built from the quaternion with its vector constants and lane
/// order, each column times the matching scale component.
fn scaled_rotation_columns(trs: &LocalTrs) -> [Lanes; 3] {
    const C0: Lanes = [-2., 2., -2., 0.];
    const C1: Lanes = [-2., 2., 2., 0.];
    const C3: Lanes = [-2., -2., 2., 0.];
    const C5: Lanes = [2., -2., 2., 0.];
    const C6: Lanes = [2., -2., -2., 0.];
    const C18: Lanes = [2., 2., -2., 0.];
    let q = trs.rotation;
    let swapped = [q[1], q[0], q[3], q[2]];
    let rotated = [q[2], q[3], q[0], q[1]];
    let reversed = [q[3], q[2], q[1], q[0]];
    let column0 = lanes_add(
        lanes_add(lanes_mul(swapped, lanes_scale(C0, q[1])), lanes_mul(rotated, lanes_scale(C1, q[2]))),
        [1., 0., 0., 0.],
    );
    let column1 = lanes_add(
        lanes_add(lanes_mul(reversed, lanes_scale(C3, q[2])), lanes_mul(swapped, lanes_scale(C5, q[0]))),
        [0., 1., 0., 0.],
    );
    let column2 = lanes_add(
        lanes_add(lanes_mul(rotated, lanes_scale(C6, q[0])), lanes_mul(reversed, lanes_scale(C18, q[1]))),
        [0., 0., 1., 0.],
    );
    [
        lanes_scale(column0, trs.scale[0]),
        lanes_scale(column1, trs.scale[1]),
        lanes_scale(column2, trs.scale[2]),
    ]
}

/// `Transform::GetLocalToWorldMatrix` over a chain of transforms from the
/// node up to the top ancestor: the node's own columns and position, then
/// each ancestor applied to them in turn, `column' = P0 * column.x +
/// (P1 * column.y + P2 * column.z)` and `position' = parentPosition +
/// (P0 * position.x + (P1 * position.y + P2 * position.z))`, with `P` the
/// ancestor's rotation-and-scale columns. Positions therefore accumulate
/// from the node outwards, which a parent-world times local product does
/// not reproduce in float32.
fn local_to_world(chain: impl Iterator<Item = LocalTrs>) -> Mat4 {
    let mut chain = chain;
    let own = chain.next().expect("a transform chain has its node");
    let mut columns = scaled_rotation_columns(&own);
    let mut position = own.position;
    for ancestor in chain {
        let [p0, p1, p2] = scaled_rotation_columns(&ancestor);
        let apply = |v: Lanes| {
            lanes_add(lanes_scale(p0, v[0]), lanes_add(lanes_scale(p1, v[1]), lanes_scale(p2, v[2])))
        };
        columns = [apply(columns[0]), apply(columns[1]), apply(columns[2])];
        position = lanes_add(ancestor.position, apply(position));
    }
    let [c0, c1, c2] = columns;
    Mat4::from_cols(
        Vec4::new(c0[0], c0[1], c0[2], 0.),
        Vec4::new(c1[0], c1[1], c1[2], 0.),
        Vec4::new(c2[0], c2[1], c2[2], 0.),
        Vec4::new(position[0], position[1], position[2], 1.),
    )
}

impl UiPrefab {
    pub fn parse(bytes: &str) -> Result<Self, String> {
        serde_json::from_str(bytes).map_err(|e| e.to_string())
    }

    /// Image names inside a document are relative to the root it was read
    /// from. Rewrite them to full asset paths of that root, so documents of
    /// different roots never share an image name.
    pub fn rebase_images(&mut self, root: &str) {
        for node in &mut self.nodes {
            for component in &mut node.components {
                for source in [&mut component.sprite, &mut component.texture].into_iter().flatten() {
                    if let Some(image) = source.get_mut("image") {
                        if let Some(name) = image.as_str().filter(|name| !name.contains("://")) {
                            *image = Value::String(format!("{root}{name}"));
                        }
                    }
                }
            }
        }
    }

    pub fn find(&self, suffix: &str) -> Result<usize, String> {
        if let Some(id) = suffix.strip_prefix('@').and_then(|s| s.parse::<i64>().ok()) {
            return self
                .indices
                .identities
                .get(&id)
                .copied()
                .ok_or_else(|| format!("UI identity {id} missing in {}", self.prefab));
        }
        match self.indices.suffixes.get(suffix) {
            Some(&(index, 1)) => Ok(index),
            found => Err(format!(
                "UI path {suffix}: {} matches in {}",
                found.map_or(0, |(_, count)| *count),
                self.prefab
            )),
        }
    }

    /// The parent node of `index` inside this prefab (None for the root).
    pub fn parent(&self, index: usize) -> Option<usize> {
        self.indices.parents[index]
    }

    pub fn resolve(&self, canvas: Vec2, visible: &HashMap<usize, bool>) -> Vec<UiRect> {
        self.resolve_with(canvas, visible, &HashMap::new())
    }

    pub fn resolve_with(
        &self,
        canvas: Vec2,
        visible: &HashMap<usize, bool>,
        rect_overrides: &HashMap<usize, RectTransform>,
    ) -> Vec<UiRect> {
        let root = UiRect {
            size: canvas,
            pivot: Vec2::splat(0.5),
            world: Mat4::IDENTITY,
            active: true,
            clipping: clipping::UiClipping::default(),
        };
        // The document root sits in a full-canvas transform with identity
        // rotation and scale at the canvas centre; the frame is that
        // transform's local space.
        let top = LocalTrs { position: [0.; 4], rotation: [0., 0., 0., 1.], scale: [1.; 3] };
        let mut rects: Vec<UiRect> = Vec::with_capacity(self.nodes.len());
        let mut locals: Vec<LocalTrs> = Vec::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.iter().enumerate() {
            let parent = self.indices.parents[i]
                .map(|index| &rects[index])
                .unwrap_or(&root);
            let r = rect_overrides.get(&i).unwrap_or(&node.rect);
            let pivot = Vec2::from_array(r.pivot);
            let size = rect_size(parent.size, parent.pivot, r);
            let position = local_position(parent.size, parent.pivot, r);
            locals.push(LocalTrs {
                position: [position.x, position.y, 0., 0.],
                rotation: r.local_rotation,
                scale: r.local_scale,
            });
            let ancestors = std::iter::successors(Some(i), |&j| self.indices.parents[j]);
            let world = local_to_world(ancestors.map(|j| locals[j]).chain(std::iter::once(top)));
            rects.push(UiRect {
                size,
                pivot,
                world,
                active: parent.active && visible.get(&i).copied().unwrap_or(node.active),
                clipping: clipping::UiClipping::default(),
            });
        }
        rects
    }
}
