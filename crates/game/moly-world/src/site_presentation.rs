//! Presentation ownership of detached draws and captured spatial sounds.
//! Simulation clocks and source-active state remain with their original owner.
use bevy::prelude::*;

#[derive(Component)]
struct HiddenPresentation(Visibility);

fn owner_site(world: &World, mut node: Entity) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    loop {
        if !seen.insert(node) { return None; }
        if let Some(owner) = world.get::<crate::fixture::residency::FixtureSiteOwner>(node) { return Some(owner.site_type.clone()); }
        if let Some(actor) = world.get::<crate::npc::NpcActions>(node) { return Some(actor.site_type.clone()); }
        if let Some(owner) = world.get::<crate::site::ResidentSiteRoot>(node) { return Some(owner.site_type.clone()); }
        if world.get::<crate::site::SiteRoot>(node).is_some() || world.get::<crate::player::PlayerControlled>(node).is_some() {
            return world.get_resource::<crate::site::SiteActive>().map(|s|s.site_type.clone());
        }
        node = world.get::<ChildOf>(node)?.parent();
    }
}

fn offset(world: &World, site: &str) -> Option<Vec3> {
    let active = world.get_resource::<crate::site::SiteActive>()?;
    let sites = world.get_resource::<crate::site::Sites>()?;
    Some(sites.placement(site)?.product_origin()-sites.placement(&active.site_type)?.product_origin())
}

/// The middleware captures the Director's position at Play3DSE. The position
/// stays fixed in its own site's frame as the camera changes to another site.
pub(crate) fn spatial_sound(world: &mut World, playback: Entity, owner: Entity, position: Vec3) {
    let site = owner_site(world,owner);
    let delta = site.as_deref().and_then(|s|offset(world,s)).unwrap_or_default();
    world.entity_mut(playback).insert((CapturedSound { site, position }, Transform::from_translation(position+delta),GlobalTransform::from_translation(position+delta)));
}
#[derive(Component)]
struct CapturedSound { site: Option<String>, position: Vec3 }

fn set_hidden(world: &mut World, draw: Entity, hidden: bool) {
    let Ok(mut entity)=world.get_entity_mut(draw) else { return; };
    if hidden {
        if !entity.contains::<HiddenPresentation>() {
            let prior=entity.get::<Visibility>().copied().unwrap_or(Visibility::Inherited);
            entity.insert(HiddenPresentation(prior));
        }
        entity.insert((Visibility::Hidden,crate::markers::SitePresentationHidden));
    } else if let Some(prior)=entity.take::<HiddenPresentation>() {
        entity.remove::<crate::markers::SitePresentationHidden>();
        entity.insert(prior.0);
    }
}

pub(crate) fn update(world: &mut World) {
    let Some(active)=world.get_resource::<crate::site::SiteActive>().map(|s|s.site_type.clone()) else { return; };
    let particles: Vec<_>=world.query::<(Entity,&crate::uber_particle::FixtureParticleLive)>().iter(world)
        .filter_map(|(draw,live)|live.0.anchor.map(|anchor|(draw,anchor))).collect();
    let mut hidden_draws=std::collections::HashMap::new();
    for (draw,anchor) in particles {
        let hidden=owner_site(world,anchor).is_some_and(|site|site!=active);
        hidden_draws.insert(draw,hidden);
        set_hidden(world,draw,hidden);
    }
    // Trails are separate draw entities; they follow the system's render owner.
    let trails: Vec<_>=world.query::<(Entity,&crate::source_particle::SourceParticle)>().iter(world)
        .filter_map(|(draw,source)|source.follows.map(|leader|(draw,leader))).collect();
    for (draw,leader) in trails {
        if let Some(&hidden)=hidden_draws.get(&leader) { set_hidden(world,draw,hidden); }
    }
    let sounds: Vec<_>=world.query::<(Entity,&CapturedSound)>().iter(world)
        .map(|(e,s)|(e,s.site.clone(),s.position)).collect();
    for (entity,site,position) in sounds {
        let delta=site.as_deref().and_then(|s|offset(world,s)).unwrap_or_default();
        if let Some(mut transform)=world.get_mut::<Transform>(entity) { transform.translation=position+delta; }
    }
    // Scene-root sounds already carry the real site offset. Fixture sounds
    // remain in local simulation space and receive only an audio-space offset.
    let sounds: Vec<_>=world.query_filtered::<Entity,With<crate::audio::SoundObject>>().iter(world).collect();
    for entity in sounds {
        let mut current=entity;
        let mut local=false;
        loop {
            if world.get::<crate::fixture::residency::FixtureSiteOwner>(current).is_some() || world.get::<crate::npc::NpcActions>(current).is_some() { local=true; break; }
            let Some(parent)=world.get::<ChildOf>(current) else { break; };
            current=parent.parent();
        }
        let delta=if local { owner_site(world,entity).as_deref().and_then(|s|offset(world,s)).unwrap_or_default() } else { Vec3::ZERO };
        world.entity_mut(entity).insert(crate::audio::SourcePositionOffset(delta));
    }
}
