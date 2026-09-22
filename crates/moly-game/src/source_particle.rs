//! Source material preparation shared by live particle renderers. Loading is
//! independent of the simulation clock; program and texture owners stay intact.
use bevy::prelude::*;
use moly_assets::source_shader::{
    Result, SourceShaderError,
    loader::{SourceProgramAsset, SourceTextureAsset},
    state::SourcePassState,
};
use std::{collections::BTreeMap, sync::Arc};
fn require(value: bool, message: impl Into<String>) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(SourceShaderError(message.into()))
    }
}
use crate::source_particle_streams::ParticleStreams;
use bevy::render::extract_component::ExtractComponent;
use moly_assets::source_shader::{SourceShaderCatalogue, material::MaterialSnapshot};

#[derive(Clone)]
pub struct ParticlePass {
    pub program: Handle<SourceProgramAsset>,
    pub state: SourcePassState,
    pub effect: bool,
}

#[derive(Component, Clone, ExtractComponent)]
#[require(crate::source_color::EncodedColorOutput)]
pub struct SourceParticle {
    /// Material references are relative to their exporting archive, not weather.
    asset_root: String,
    pub material: Arc<MaterialSnapshot>,
    pub catalogue: Handle<SourceShaderCatalogue>,
    pub streams: ParticleStreams,
    pub passes: Vec<ParticlePass>,
    pub textures: BTreeMap<String, Handle<SourceTextureAsset>>,
    pub error: Option<String>,
    pub enabled: bool,
    pub readiness: Arc<std::sync::Mutex<ParticleReadiness>>,
    pub render_queue: i32,
    pub sorting_order: i32,
    pub sorting_fudge: f32,
}
#[derive(Clone, Debug, Default)]
pub enum ParticleReadiness {
    #[default]
    Pending,
    Ready,
    Failed(String),
}
impl SourceParticle {
    pub fn load(renderer: &serde_json::Value, server: &AssetServer) -> Result<Self> {
        Self::load_from(renderer, server, "phenomena")
    }

    pub(crate) fn load_from(renderer: &serde_json::Value, server: &AssetServer, asset_root: &str) -> Result<Self> {
        require(matches!(asset_root, "phenomena" | "fixture-particles-v2"), "unknown source particle archive")?;
        let material = Arc::new(MaterialSnapshot::parse(&renderer["material"])?);
        let streams = ParticleStreams::parse(renderer)?;
        require(
            renderer["sortingLayerId"] == 0 && renderer["maskInteraction"] == 0,
            "particle sorting layer or sprite mask requires its renderer owner",
        )?;
        let render_queue = renderer["material"]["renderQueue"]
            .as_i64()
            .ok_or_else(|| SourceShaderError("source material render queue is absent".into()))?
            as i32;
        let sorting_order = renderer["sortingOrder"]
            .as_i64()
            .ok_or_else(|| SourceShaderError("source sorting order is absent".into()))?
            as i32;
        let sorting_fudge = renderer["sortingFudge"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| SourceShaderError("source sorting fudge is absent".into()))?
            as f32;
        let catalogue = server.load(format!(
            "moly://{asset_root}/{}",
            material.shader.variants.file
        ));
        let textures = material
            .textures
            .iter()
            .filter_map(|(name, binding)| {
                binding.texture.as_ref().map(|texture| {
                    (
                        name.clone(),
                        server.load(format!("moly://{asset_root}/{}", texture.content.file)),
                    )
                })
            })
            .collect();
        Ok(Self {
            asset_root: asset_root.to_owned(),
            material,
            catalogue,
            streams,
            passes: Vec::new(),
            textures,
            error: None,
            enabled: false,
            readiness: Arc::new(std::sync::Mutex::new(ParticleReadiness::Pending)),
            render_queue,
            sorting_order,
            sorting_fudge,
        })
    }

    pub fn resolve(
        &mut self,
        server: &AssetServer,
        catalogues: &Assets<SourceShaderCatalogue>,
    ) -> Result<bool> {
        if !self.passes.is_empty() {
            return Ok(true);
        }
        if let bevy::asset::LoadState::Failed(error) = server.load_state(&self.catalogue) {
            return Err(SourceShaderError(format!(
                "source catalogue load failed: {error}"
            )));
        }
        let Some(catalogue) = catalogues.get(&self.catalogue) else {
            return Ok(false);
        };
        self.material.matches(catalogue)?;
        let subshaders = catalogue
            .passes
            .iter()
            .map(|p| p.sub_shader_index)
            .collect::<std::collections::BTreeSet<_>>();
        require(
            subshaders.len() == 1,
            "multiple source SubShaders require renderer capability selection",
        )?;
        let enabled = |pass: &&moly_assets::source_shader::SourcePass| {
            pass.pass_type == Some(0)
                && !pass
                    .name
                    .as_ref()
                    .is_some_and(|name| self.material.disabled_passes.contains(name))
        };
        // DrawObjectsPass queries these tags. Serialized UsePass/GrabPass entries
        // are not executable passes. An omitted LightMode is SRPDefaultUnlit.
        let forwards = catalogue
            .passes
            .iter()
            .filter(enabled)
            .filter(|p| {
                matches!(
                    p.light_mode.as_deref().unwrap_or("SRPDefaultUnlit"),
                    "SRPDefaultUnlit" | "UniversalForward" | "UniversalForwardOnly"
                )
            })
            .collect::<Vec<_>>();
        require(
            forwards.len() == 1,
            format!(
                "{} executable forward passes require renderer selection",
                forwards.len()
            ),
        )?;
        if self.render_queue == -1 {
            let tag = forwards[0]
                .sub_shader_tags
                .as_ref()
                .and_then(|tags| tags.as_object())
                .and_then(|tags| {
                    tags.iter()
                        .find(|(key, _)| key.eq_ignore_ascii_case("Queue"))
                })
                .and_then(|(_, value)| value.as_str());
            self.render_queue = match tag {
                Some("Transparent") => 3000,
                Some("Geometry") | None => 2000,
                other => {
                    return Err(SourceShaderError(format!(
                        "unconsumed source queue tag {other:?}"
                    )));
                }
            };
        }
        require(
            (2501..=5000).contains(&self.render_queue),
            "particle source queue is outside the transparent renderer range",
        )?;
        let effects = catalogue
            .passes
            .iter()
            .filter(enabled)
            .filter(|p| p.light_mode.as_deref() == Some("MysekaiEffect"))
            .collect::<Vec<_>>();
        require(
            effects.len() <= 1,
            "multiple effect passes require renderer selection",
        )?;
        let mut passes = Vec::new();
        for (pass, effect) in forwards
            .into_iter()
            .map(|p| (p, false))
            .chain(effects.into_iter().map(|p| (p, true)))
        {
            let (_, variant) = catalogue.select(
                pass.sub_shader_index,
                pass.pass_index,
                9,
                4,
                &self.material.keywords,
            )?;
            let state = SourcePassState::resolve(pass, |name| {
                self.material.scalar_property(catalogue, name)
            })?;
            passes.push(ParticlePass {
                program: server.load(format!(
                    "moly://{}/{}", self.asset_root,
                    variant.conversion.as_ref().expect("converted variant").file
                )),
                state,
                effect,
            });
        }
        self.passes = passes;
        Ok(true)
    }
}

pub fn resolve_particles(
    server: Res<AssetServer>,
    catalogues: Res<Assets<SourceShaderCatalogue>>,
    mut particles: Query<&mut SourceParticle>,
) {
    for mut particle in &mut particles {
        if particle.error.is_some() {
            continue;
        }
        let failed = particle
            .passes
            .iter()
            .find_map(|pass| match server.load_state(&pass.program) {
                bevy::asset::LoadState::Failed(error) => {
                    Some(format!("source program load failed: {error}"))
                }
                _ => None,
            })
            .or_else(|| {
                particle.textures.iter().find_map(|(name, texture)| {
                    match server.load_state(texture) {
                        bevy::asset::LoadState::Failed(error) => {
                            Some(format!("source texture {name} load failed: {error}"))
                        }
                        _ => None,
                    }
                })
            });
        if let Some(error) = failed {
            *particle.readiness.lock().unwrap() = ParticleReadiness::Failed(error.clone());
            error!(%error, "source particle dependency failed");
            particle.error = Some(error);
            continue;
        }
        if let Err(error) = particle.resolve(&server, &catalogues) {
            error!(%error, "source particle material preparation failed");
            *particle.readiness.lock().unwrap() = ParticleReadiness::Failed(error.to_string());
            particle.error = Some(error.to_string());
        }
    }
}
