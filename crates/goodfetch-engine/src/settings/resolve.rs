//! The resolver (ADR 0028 as amended): every key the Run's Product honours
//! takes its built-in default, then the Operator policy's value, then user
//! and project configuration's, the Recipe's and the run's, each higher
//! layer winning only where no lower layer locked the value, within an
//! Operator policy's cap, a Recipe's exposed bounds and the key's own
//! authority: which layers may set it, and whether a Recipe or the run
//! may only narrow it. A pure function of its layers.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::time::Duration;

use goodfetch_protocol::catalog::{Authority, Product, ValueKind};
use goodfetch_protocol::settings::{
    ExplainedSetting, Exposure, RecipeManifest, RunOrigin, SettingLimit, SettingsExplanation,
    SettingsFile, SettingsLayer,
};
use serde_json::Value;

use super::error::SettingsError;
use super::keys::{self, Key};
use super::values::Values;

/// An Operator policy: what it sets, locks and caps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Policy {
    /// The values it sets.
    pub settings: Values,
    /// The values it sets and locks.
    pub lock: Values,
    /// Its caps on counts and durations.
    pub cap: Values,
    /// Its version, which the run report names.
    pub version: Option<String>,
}

impl Policy {
    /// Checks that each cap is on a count or a duration, the only kinds a
    /// cap bounds.
    ///
    /// # Errors
    ///
    /// `setting_invalid` for a cap on any other kind.
    pub fn check_caps(&self) -> Result<(), SettingsError> {
        for name in self.cap.applied.keys() {
            let capped = keys::find(name).map(|key| key.setting.kind);
            if !matches!(capped, Some(ValueKind::Count | ValueKind::Duration)) {
                return Err(SettingsError::Invalid {
                    key: (*name).to_owned(),
                    layer: Some(SettingsLayer::OperatorPolicy),
                    reason: "only a count or a duration takes a cap; lock it instead".to_owned(),
                });
            }
        }
        Ok(())
    }
}

/// A Recipe's layer: the settings it declares, locked unless exposed, and
/// those it exposes, with their bounds (ADR 0028).
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    name: String,
    declared: Values,
    expose: BTreeMap<&'static str, Exposure>,
}

impl Recipe {
    /// Reads `manifest`'s settings, on an engine of `engine`.
    ///
    /// # Errors
    ///
    /// `not_in_product` for a Recipe that names a Product `engine` lacks
    /// (ADR 0096), and the refusals of [`Values::read`] for its keys.
    pub fn read(manifest: &RecipeManifest, engine: Product) -> Result<Self, SettingsError> {
        if !engine.contains(manifest.product) {
            return Err(SettingsError::NotInProduct {
                key: manifest.name.clone(),
                needs: manifest.product,
            });
        }
        let declared = Values::read(SettingsLayer::Recipe, &manifest.settings, engine)?;
        let mut expose = BTreeMap::new();
        for (name, exposure) in &manifest.expose {
            let key = keys::find(name).ok_or_else(|| SettingsError::Unknown {
                key: name.clone(),
                layer: Some(SettingsLayer::Recipe),
                closest: keys::closest(name).map(str::to_owned),
            })?;
            if !engine.contains(key.product) {
                return Err(SettingsError::NotInProduct {
                    key: name.clone(),
                    needs: key.product,
                });
            }
            expose.insert(key.setting.name, exposure.clone());
        }
        Ok(Self {
            name: manifest.name.clone(),
            declared,
            expose,
        })
    }

    /// The Recipe's name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Every layer a Run's settings resolve through.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layers {
    /// The Operator policy.
    pub operator_policy: Policy,
    /// User configuration.
    pub user_configuration: Values,
    /// Project configuration.
    pub project_configuration: Values,
    /// The Recipe, when the Run is one's.
    pub recipe: Option<Recipe>,
    /// The run itself.
    pub run: Values,
}

/// One key's effective value.
#[derive(Clone, Debug, PartialEq)]
pub struct Effective {
    /// The value.
    pub value: Value,
    /// The layer that set it.
    pub layer: SettingsLayer,
    /// For the run layer, where it came from.
    pub origin: Option<RunOrigin>,
    /// The layer that locked it.
    pub locked_by: Option<SettingsLayer>,
    /// The Operator policy's cap on it.
    pub cap: Option<Value>,
    /// The smallest Product that honours it.
    pub product: Product,
    /// Whether its value weakens a guarantee or switches off a costly
    /// control.
    pub weakens: bool,
}

/// A key a configuration file holds that only the other Product honours,
/// listed, never applied (ADR 0087).
#[derive(Clone, Debug, PartialEq)]
pub struct Unapplied {
    /// The value its highest layer wrote.
    pub value: Value,
    /// That layer.
    pub layer: SettingsLayer,
    /// The Product that honours it.
    pub product: Product,
}

/// Every effective value of a Run, or of an engine before any Run.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    settings: BTreeMap<&'static str, Effective>,
    unapplied: BTreeMap<&'static str, Unapplied>,
    policy_version: Option<String>,
}

impl Resolved {
    /// The effective value of `key`, if the Run's Product honours it.
    pub fn get(&self, key: &str) -> Option<&Effective> {
        self.settings.get(key)
    }

    /// The effective value of `key`, `null` if the Run's Product does not
    /// honour it.
    pub fn value(&self, key: &str) -> &Value {
        self.settings
            .get(key)
            .map_or(&Value::Null, |effective| &effective.value)
    }

    /// `key`'s value as a count, `None` when it is not one.
    pub fn count(&self, key: &str) -> Option<u64> {
        self.value(key).as_u64()
    }

    /// `key`'s value as a duration, `None` when it is not one or cleared.
    pub fn duration(&self, key: &str) -> Option<Duration> {
        self.value(key).as_u64().map(Duration::from_millis)
    }

    /// `key`'s value as a boolean, `None` when it is not one.
    pub fn boolean(&self, key: &str) -> Option<bool> {
        self.value(key).as_bool()
    }

    /// `key`'s value as host names, each with its addresses, as
    /// `network.resolve` holds them (ADR 0059), from which a resolver's
    /// mapping is built; empty when it holds none.
    pub fn hosts(&self, key: &str) -> Vec<(String, Vec<IpAddr>)> {
        self.value(key)
            .as_object()
            .into_iter()
            .flatten()
            .map(|(host, addresses)| {
                let addresses = addresses
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|address| address.as_str()?.parse().ok())
                    .collect();
                (host.clone(), addresses)
            })
            .collect()
    }

    /// The version of the Operator policy it resolved under.
    pub fn policy_version(&self) -> Option<&str> {
        self.policy_version.as_deref()
    }

    /// Every setting whose value weakens a guarantee or switches off a
    /// costly control, with its value and layer, which the results it
    /// touches are marked with (ADR 0028).
    pub fn weakened_by(&self) -> Vec<SettingLimit> {
        self.settings
            .iter()
            .filter(|(_, effective)| effective.weakens)
            .map(|(key, effective)| SettingLimit {
                key: (*key).to_owned(),
                value: effective.value.clone(),
                layer: effective.layer,
            })
            .collect()
    }

    /// The same values held to `policy`'s caps, as a Run already going
    /// takes them at its next admission point (ADR 0028): a value above a
    /// tightened cap becomes the cap, set by the Operator policy.
    #[must_use]
    pub fn under_caps_of(&self, policy: &Policy) -> Self {
        let mut capped = self.clone();
        for (key, effective) in &mut capped.settings {
            let Some(cap) = policy.cap.get(key) else {
                effective.cap = None;
                continue;
            };
            effective.cap = Some(cap.clone());
            if let (Some(value), Some(bound)) = (effective.value.as_u64(), cap.as_u64())
                && value > bound
            {
                effective.value = cap.clone();
                effective.layer = SettingsLayer::OperatorPolicy;
                effective.origin = None;
            }
        }
        capped.policy_version.clone_from(&policy.version);
        capped
    }

    /// What `config explain` shows for the keys `prefixes` name (every key
    /// when empty), with the files read.
    pub fn explain(&self, prefixes: &[String], files: Vec<SettingsFile>) -> SettingsExplanation {
        let wanted = |key: &str| {
            prefixes.is_empty()
                || prefixes.iter().any(|prefix| {
                    key == prefix
                        || key
                            .strip_prefix(prefix.as_str())
                            .is_some_and(|rest| rest.starts_with('.'))
                })
        };
        let mut settings: Vec<ExplainedSetting> = self
            .settings
            .iter()
            .map(|(key, effective)| ExplainedSetting {
                key: (*key).to_owned(),
                products: honouring(effective.product),
                applied: true,
                value: effective.value.clone(),
                layer: effective.layer,
                origin: effective.origin,
                locked_by: effective.locked_by,
                cap: effective.cap.clone(),
                weakens: effective.weakens,
            })
            .chain(
                self.unapplied
                    .iter()
                    .map(|(key, unapplied)| ExplainedSetting {
                        key: (*key).to_owned(),
                        products: honouring(unapplied.product),
                        applied: false,
                        value: unapplied.value.clone(),
                        layer: unapplied.layer,
                        origin: None,
                        locked_by: None,
                        cap: None,
                        weakens: false,
                    }),
            )
            .filter(|setting| wanted(&setting.key))
            .collect();
        settings.sort_by(|left, right| left.key.cmp(&right.key));
        SettingsExplanation {
            policy_version: self.policy_version.clone(),
            files,
            settings,
        }
    }
}

/// The Products that honour a key of `product`'s mark.
fn honouring(product: Product) -> Vec<Product> {
    [Product::Collie, Product::Foxhound]
        .into_iter()
        .filter(|honours| honours.contains(product))
        .collect()
}

/// Resolves `layers` for a Run that needs `run` on an engine of `engine`.
///
/// # Errors
///
/// The first refusal, by key: `setting_locked` for a value a lower layer
/// locked, capped or reserved, or outside a Recipe's bounds;
/// `setting_not_applicable` for a key set in a layer that may not hold it,
/// or on a Run of a Product that does not honour it.
pub fn resolve(layers: &Layers, engine: Product, run: Product) -> Result<Resolved, SettingsError> {
    for (layer, values) in explicit_layers(layers) {
        for name in values.applied.keys() {
            let product = keys::find(name).map_or(Product::Collie, |key| key.product);
            if !run.contains(product) {
                return Err(SettingsError::NotApplicable {
                    key: (*name).to_owned(),
                    layer,
                    fix: format!(
                        "`{name}` applies to a {} Run only; leave it out of a {} Run",
                        product.name(),
                        run.name()
                    ),
                });
            }
        }
    }
    let mut settings = BTreeMap::new();
    for key in keys::all().filter(|key| engine.contains(key.product) && run.contains(key.product)) {
        settings.insert(key.setting.name, resolve_key(layers, key)?);
    }
    Ok(Resolved {
        settings,
        unapplied: unapplied(layers),
        policy_version: layers.operator_policy.version.clone(),
    })
}

/// The layers whose values are explicit for the Run: the Recipe's and the
/// run's.
fn explicit_layers(layers: &Layers) -> Vec<(SettingsLayer, &Values)> {
    let mut explicit = Vec::new();
    if let Some(recipe) = &layers.recipe {
        explicit.push((SettingsLayer::Recipe, &recipe.declared));
    }
    explicit.push((SettingsLayer::Run, &layers.run));
    explicit
}

fn unapplied(layers: &Layers) -> BTreeMap<&'static str, Unapplied> {
    let policy = &layers.operator_policy;
    let mut unapplied = BTreeMap::new();
    for (layer, values) in [
        (SettingsLayer::OperatorPolicy, &policy.settings),
        (SettingsLayer::OperatorPolicy, &policy.lock),
        (SettingsLayer::UserConfiguration, &layers.user_configuration),
        (
            SettingsLayer::ProjectConfiguration,
            &layers.project_configuration,
        ),
    ] {
        for (name, value) in &values.others {
            let product = keys::find(name).map_or(Product::Foxhound, |key| key.product);
            unapplied.insert(
                *name,
                Unapplied {
                    value: value.clone(),
                    layer,
                    product,
                },
            );
        }
    }
    unapplied
}

/// The value so far and what holds it.
struct State {
    effective: Effective,
    locked: Option<SettingLimit>,
    cap: Option<SettingLimit>,
    exposure: Option<Exposure>,
}

fn resolve_key(layers: &Layers, key: Key) -> Result<Effective, SettingsError> {
    let setting = key.setting;
    let name = setting.name;
    let policy = &layers.operator_policy;
    let default = (setting.default)();
    let mut state = State {
        effective: Effective {
            value: default.clone(),
            layer: SettingsLayer::Default,
            origin: None,
            locked_by: None,
            cap: None,
            product: key.product,
            weakens: false,
        },
        locked: None,
        cap: None,
        exposure: None,
    };
    if matches!(setting.authority, Authority::Fixed) {
        state.locked = Some(limit(name, &default, SettingsLayer::Default));
    }
    if let Some(cap) = policy.cap.get(name) {
        state.cap = Some(limit(name, cap, SettingsLayer::OperatorPolicy));
        state.effective.cap = Some(cap.clone());
    }
    let locked = policy.lock.get(name);
    let set = locked.or_else(|| policy.settings.get(name));
    if let Some(value) = set {
        take(&mut state, key, SettingsLayer::OperatorPolicy, value, None)?;
    }
    if locked.is_some() {
        state.locked = Some(limit(
            name,
            &state.effective.value,
            SettingsLayer::OperatorPolicy,
        ));
    }
    if let Some(value) = layers.user_configuration.get(name) {
        take(
            &mut state,
            key,
            SettingsLayer::UserConfiguration,
            value,
            None,
        )?;
    }
    if let Some(value) = layers.project_configuration.get(name) {
        take(
            &mut state,
            key,
            SettingsLayer::ProjectConfiguration,
            value,
            None,
        )?;
    }
    if let Some(recipe) = &layers.recipe {
        if let Some(value) = recipe.declared.get(name) {
            take(&mut state, key, SettingsLayer::Recipe, value, None)?;
        }
        match recipe.expose.get(name) {
            Some(exposure) => state.exposure = Some(exposure.clone()),
            None if recipe.declared.get(name).is_some() && state.locked.is_none() => {
                state.locked = Some(limit(name, &state.effective.value, SettingsLayer::Recipe));
            }
            None => {}
        }
    }
    if let Some(value) = layers.run.get(name) {
        take(
            &mut state,
            key,
            SettingsLayer::Run,
            value,
            layers.run.origin,
        )?;
    }
    if let (Some(cap), Some(value)) = (&state.cap, state.effective.value.as_u64())
        && state.effective.layer == SettingsLayer::Default
        && cap.value.as_u64().is_some_and(|bound| value > bound)
    {
        state.effective.value = cap.value.clone();
        state.effective.layer = SettingsLayer::OperatorPolicy;
    }
    state.effective.locked_by = state.locked.as_ref().map(|locked| locked.layer);
    state.effective.weakens = setting
        .weakens
        .is_some_and(|weakens| weakens(&state.effective.value));
    Ok(state.effective)
}

fn limit(key: &str, value: &Value, layer: SettingsLayer) -> SettingLimit {
    SettingLimit {
        key: key.to_owned(),
        value: value.clone(),
        layer,
    }
}

/// Takes `value` from `layer` for `key` unless something holds the key.
fn take(
    state: &mut State,
    key: Key,
    layer: SettingsLayer,
    value: &Value,
    origin: Option<RunOrigin>,
) -> Result<(), SettingsError> {
    let name = key.setting.name;
    let refused = |limit: SettingLimit, fix: String| SettingsError::Locked {
        key: name.to_owned(),
        layer,
        limit,
        fix,
    };
    let current = limit(name, &state.effective.value, state.effective.layer);
    authority(key, layer, value, &current, &refused)?;
    if let Some(locked) = &state.locked {
        let fix = match locked.layer {
            SettingsLayer::Default => format!("`{name}` is an honesty guarantee no layer changes"),
            SettingsLayer::Recipe => format!("the Recipe does not expose `{name}`"),
            _ => format!("ask the operator to change the Operator policy's lock on `{name}`"),
        };
        return Err(refused(locked.clone(), fix));
    }
    if let (Some(cap), Some(given)) = (&state.cap, value.as_u64())
        && cap.value.as_u64().is_some_and(|bound| given > bound)
    {
        return Err(refused(
            cap.clone(),
            format!("give `{name}` at most the Operator policy's cap"),
        ));
    }
    if layer == SettingsLayer::Run
        && let Some(exposure) = &state.exposure
        && !within(exposure, value)
    {
        let bounds = serde_json::to_value(exposure).unwrap_or(Value::Null);
        return Err(refused(
            limit(name, &bounds, SettingsLayer::Recipe),
            format!("give `{name}` a value within the Recipe's bounds"),
        ));
    }
    state.effective.value = value.clone();
    state.effective.layer = layer;
    state.effective.origin = origin;
    Ok(())
}

/// The key's own authority over `layer` (ADR 0028 and its amendments).
fn authority(
    key: Key,
    layer: SettingsLayer,
    value: &Value,
    current: &SettingLimit,
    refused: &dyn Fn(SettingLimit, String) -> SettingsError,
) -> Result<(), SettingsError> {
    let name = key.setting.name;
    let above_configuration = matches!(layer, SettingsLayer::Recipe | SettingsLayer::Run);
    let configuration_only = || {
        refused(
            current.clone(),
            format!(
                "`{name}` is taken only from an Operator policy and user and project configuration"
            ),
        )
    };
    match key.setting.authority {
        Authority::Fixed => Err(refused(
            current.clone(),
            format!("`{name}` is an honesty guarantee no layer changes"),
        )),
        Authority::Machine => match layer {
            SettingsLayer::OperatorPolicy | SettingsLayer::UserConfiguration => Ok(()),
            _ => Err(SettingsError::NotApplicable {
                key: name.to_owned(),
                layer,
                fix: format!("set `{name}` in user configuration or an Operator policy"),
            }),
        },
        Authority::OperatorPolicy if layer != SettingsLayer::OperatorPolicy => Err(refused(
            current.clone(),
            format!("`{name}` is set only in an Operator policy"),
        )),
        Authority::Configuration if above_configuration => Err(configuration_only()),
        Authority::ConfigurationExcept(allowed) if above_configuration && !allowed(value) => {
            Err(configuration_only())
        }
        Authority::NotByRecipe if layer == SettingsLayer::Recipe => Err(refused(
            current.clone(),
            format!("a Recipe never sets `{name}`; it may only expose it"),
        )),
        Authority::Narrowed if above_configuration && !narrows(key, value, &current.value) => {
            Err(refused(
                current.clone(),
                format!(
                    "a Recipe or a run only narrows `{name}`; widen it in configuration or an Operator policy"
                ),
            ))
        }
        Authority::Every
        | Authority::OperatorPolicy
        | Authority::Configuration
        | Authority::ConfigurationExcept(_)
        | Authority::NotByRecipe
        | Authority::Narrowed => Ok(()),
    }
}

/// Whether `higher` allows no more than `lower`; `null`, where a key can be
/// cleared, is no bound.
fn narrows(key: Key, higher: &Value, lower: &Value) -> bool {
    match (higher.is_null(), lower.is_null()) {
        (_, true) if key.setting.clearable => true,
        (true, false) => false,
        _ => key.setting.kind.narrows(higher, lower),
    }
}

/// Whether `value` is within a Recipe's exposed bounds.
fn within(exposure: &Exposure, value: &Value) -> bool {
    let number = value.as_u64();
    let above_minimum = exposure
        .minimum
        .is_none_or(|minimum| number.is_some_and(|number| number >= minimum));
    let below_maximum = exposure
        .maximum
        .is_none_or(|maximum| number.is_some_and(|number| number <= maximum));
    let listed = exposure.one_of.is_empty() || exposure.one_of.contains(value);
    above_minimum && below_maximum && listed
}
