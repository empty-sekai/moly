//! Serialized UI geometry and component data, decoded once at the asset boundary.

pub mod auto_layout;
pub mod clipping;
mod runtime;
pub use runtime::UiInstance;

use bevy::math::{Mat4, Quat, Vec2, Vec3};
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
        let mut rects: Vec<UiRect> = Vec::with_capacity(self.nodes.len());
        for (i, node) in self.nodes.iter().enumerate() {
            let parent = self.indices.parents[i]
                .map(|index| &rects[index])
                .unwrap_or(&root);
            let r = rect_overrides.get(&i).unwrap_or(&node.rect);
            let amin = Vec2::from_array(r.anchors_min);
            let amax = Vec2::from_array(r.anchors_max);
            let pivot = Vec2::from_array(r.pivot);
            let size = parent.size * (amax - amin) + Vec2::from_array(r.size_delta);
            let position = parent.size * (amin + (amax - amin) * pivot - parent.pivot)
                + Vec2::from_array(r.anchored_position);
            let local = Mat4::from_scale_rotation_translation(
                Vec3::from_array(r.local_scale),
                Quat::from_array(r.local_rotation),
                position.extend(0.),
            );
            rects.push(UiRect {
                size,
                pivot,
                world: parent.world * local,
                active: parent.active && visible.get(&i).copied().unwrap_or(node.active),
                clipping: clipping::UiClipping::default(),
            });
        }
        rects
    }
}
