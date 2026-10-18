//! The gate models the home site's layout shows: the server's home gate and
//! the model packages a gate fixture may use, as the layout owner reads them.

use std::collections::HashSet;

use bevy::prelude::*;

/// The bundles a gate fixture may show (`MysekaiGateModel.AssetBundleName`
/// over the gate, unit skin and common skin tables), as fixture packages.
#[derive(Resource, Default, Debug)]
pub(crate) struct GateModelPackages(pub(crate) HashSet<String>);

/// The home gate's model as the server document states it
/// (`userMysekaiGates`: the gate set at the home site and its skin), which
/// the home site's layout shows on its gate rows. `package` is none when the
/// server document or the gate tables are not available, or when the
/// fixture index lacks the model; the layout's own row stands then.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub(crate) struct HomeGateModel {
    pub(crate) package: Option<String>,
    /// The fixture masters whose type is gate.
    pub(crate) masters: HashSet<i32>,
}
