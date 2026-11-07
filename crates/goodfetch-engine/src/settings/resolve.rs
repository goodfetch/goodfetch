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

use goodfetch_protocol::catalog::{Authority, Product, Setting, ValueKind};
use goodfetch_protocol::foxhound::frontier::FIXED_FOR_A_RUN;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::settings::{
    ExplainedSetting, Exposure, RecipeManifest, RunOrigin, SettingLimit, SettingsExplanation,
    SettingsFile, SettingsLayer, WeakenedSetting,
};
use serde_json::Value;

use super::error::SettingsError;
use super::hosts;
use super::keys::{self, Key};
use super::values::{Given, Values};

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
    /// Its `[managed]` table, which marks it managed and names the install
    /// Limits each engine of its major writes to the Usage ledger.
    pub managed: Option<goodfetch_protocol::usage::Managed>,
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
            if !matches!(
                capped,
                Some(ValueKind::Count | ValueKind::CountUpTo(_) | ValueKind::Duration)
            ) {
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

/// The defaults an engine chose for its host in place of the catalog's,
/// such as the encrypted local file's `run_state.key_reference` on a host
/// with no OS keychain; still the default layer's, as `config explain`
/// shows them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HostDefaults(BTreeMap<&'static str, Value>);

impl HostDefaults {
    /// The same, with `value` in place of `setting`'s catalog default.
    #[must_use]
    pub fn with(mut self, setting: &Setting, value: Value) -> Self {
        self.0.insert(setting.name, value);
        self
    }

    /// The default of `setting` on this host.
    pub fn of(&self, setting: &Setting) -> Value {
        self.0
            .get(setting.name)
            .cloned()
            .unwrap_or_else(|| (setting.default)())
    }
}

/// Every layer a Run's settings resolve through.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layers {
    /// The defaults this host gives in place of the catalog's.
    pub host: HostDefaults,
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
    /// Whether a Client acting for a model gave it: in the run layer, or
    /// in the project configuration it sent (ADR 0092).
    pub for_model: bool,
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
    /// Whether a Recipe's layer took part.
    recipe_run: bool,
}

impl Resolved {
    /// `setting`'s effective value, with the layer that set it, when the
    /// Run's Product honours it.
    pub fn effective(&self, setting: &Setting) -> Option<&Effective> {
        self.settings.get(setting.name)
    }

    /// `setting`'s value as it is written, its catalog default where the
    /// Run's Product does not honour it.
    pub fn value(&self, setting: &Setting) -> &Value {
        self.settings
            .get(setting.name)
            .map_or_else(|| keys::default_of(setting), |effective| &effective.value)
    }

    /// `setting`'s value as `T`, its catalog default where the Run's
    /// Product does not honour it: `T` is the Rust type of its kind, such
    /// as `u64` for a count, and an `Option` of it exactly when the setting
    /// is clearable, `None` meaning cleared.
    ///
    /// # Panics
    ///
    /// When `T` is not the type of `setting`'s kind and clearability, a
    /// mistake of the reading code, never of a layer's values.
    pub fn get<'a, T: SettingValue<'a>>(&'a self, setting: &Setting) -> T {
        read(setting, self.value(setting))
    }

    /// `setting`'s value as host names, each with its addresses, as
    /// `network.resolve` holds them (ADR 0059), from which a resolver's
    /// mapping is built; empty when it holds none.
    pub fn hosts(&self, setting: &Setting) -> Vec<(String, Vec<IpAddr>)> {
        self.value(setting)
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

    /// The settings of a scope opened beneath these, such as one Session
    /// of the Run: `own`, the scope's own values at the run layer, each
    /// taken over the value here unless a lower layer locked or capped it,
    /// and within its key's authority, so a Client acting for a model sets
    /// none a model may not (ADRs 0028, 0083 and 0092).
    ///
    /// # Errors
    ///
    /// `setting_locked` for a value a lower layer locked or capped, or that
    /// its key's authority refuses `own`; `setting_not_applicable` for a key
    /// these settings' Product does not honour.
    pub fn beneath(&self, own: &Values) -> Result<Self, SettingsError> {
        let mut beneath = self.clone();
        for (name, value) in &own.applied {
            let (Some(key), Some(effective)) = (keys::find(name), self.settings.get(name)) else {
                return Err(SettingsError::NotApplicable {
                    key: (*name).to_owned(),
                    layer: SettingsLayer::Run,
                    fix: format!("`{name}` does not apply to this Run's Product; leave it out"),
                });
            };
            let mut state = State {
                effective: effective.clone(),
                locked: effective
                    .locked_by
                    .map(|layer| limit(name, &effective.value, layer)),
                cap: effective
                    .cap
                    .as_ref()
                    .map(|cap| limit(name, cap, SettingsLayer::OperatorPolicy)),
                exposure: None,
            };
            take(
                &mut state,
                key,
                SettingsLayer::Run,
                value,
                Some(own.given()),
            )?;
            state.effective.weakens = key
                .setting
                .weakens
                .is_some_and(|weakens| weakens(&state.effective.value));
            beneath.settings.insert(name, state.effective);
        }
        Ok(beneath)
    }

    /// Whether the Run is a Recipe's.
    pub fn recipe_run(&self) -> bool {
        self.recipe_run
    }

    /// The version of the Operator policy it resolved under.
    pub fn policy_version(&self) -> Option<&str> {
        self.policy_version.as_deref()
    }

    /// Every setting whose value weakens a guarantee or switches off a
    /// costly control, with its value, layer and origin, which the results
    /// it touches are marked with (ADR 0028).
    pub fn weakened(&self) -> Vec<WeakenedSetting> {
        self.settings
            .iter()
            .filter(|(_, effective)| effective.weakens)
            .map(|(key, effective)| WeakenedSetting {
                key: (*key).to_owned(),
                value: effective.value.clone(),
                layer: effective.layer,
                origin: effective.origin,
            })
            .collect()
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
                user_settable: effective.locked_by.is_none() && user_settable(key),
                fixed_for_run: fixed_for_run(key),
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
                        user_settable: false,
                        fixed_for_run: fixed_for_run(key),
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

/// The Rust type a setting's values read as, by its kind: the reading of a
/// value the resolver let through, which a kind it reads always has.
pub trait SettingValue<'a>: Sized {
    /// Whether it reads a clearable setting, whose `null` it reads as
    /// cleared: only an `Option` does.
    const CLEARABLE: bool = false;

    /// Whether it reads values of `kind`.
    fn reads(kind: &ValueKind) -> bool;

    /// `value`, of a kind it reads, as this type.
    fn read(value: &'a Value) -> Option<Self>;
}

impl SettingValue<'_> for bool {
    fn reads(kind: &ValueKind) -> bool {
        matches!(kind, ValueKind::Boolean)
    }

    fn read(value: &Value) -> Option<Self> {
        value.as_bool()
    }
}

impl SettingValue<'_> for u64 {
    fn reads(kind: &ValueKind) -> bool {
        matches!(kind, ValueKind::Count | ValueKind::CountUpTo(_))
    }

    fn read(value: &Value) -> Option<Self> {
        value.as_u64()
    }
}

impl SettingValue<'_> for Duration {
    fn reads(kind: &ValueKind) -> bool {
        matches!(kind, ValueKind::Duration)
    }

    fn read(value: &Value) -> Option<Self> {
        value.as_u64().map(Duration::from_millis)
    }
}

impl<'a> SettingValue<'a> for &'a str {
    fn reads(kind: &ValueKind) -> bool {
        matches!(kind, ValueKind::Text | ValueKind::Choice(_))
    }

    fn read(value: &'a Value) -> Option<Self> {
        value.as_str()
    }
}

impl SettingValue<'_> for SecretReference {
    fn reads(kind: &ValueKind) -> bool {
        matches!(kind, ValueKind::SecretReference)
    }

    fn read(value: &Value) -> Option<Self> {
        SecretReference::parse(value.as_str()?).ok()
    }
}

impl<'a, T: SettingValue<'a>> SettingValue<'a> for Option<T> {
    const CLEARABLE: bool = true;

    fn reads(kind: &ValueKind) -> bool {
        !T::CLEARABLE && T::reads(kind)
    }

    fn read(value: &'a Value) -> Option<Self> {
        if value.is_null() {
            Some(None)
        } else {
            T::read(value).map(Some)
        }
    }
}

/// `setting`'s catalog default as `T`, as [`Resolved::get`] reads it, for
/// a scope that resolved no settings.
///
/// # Panics
///
/// As [`Resolved::get`].
pub fn catalog_default<T: SettingValue<'static>>(setting: &Setting) -> T {
    read(setting, keys::default_of(setting))
}

/// `setting`'s value in `settings` as [`Resolved::get`] reads it, or its
/// catalog default when the scope resolved none, such as a Run started
/// with no settings.
///
/// # Panics
///
/// As [`Resolved::get`].
pub fn get_or_default<'a, T: SettingValue<'a>>(
    settings: Option<&'a Resolved>,
    setting: &Setting,
) -> T {
    settings.map_or_else(
        || read(setting, keys::default_of(setting)),
        |settings| settings.get(setting),
    )
}

/// `setting`'s catalog default as it is written.
pub fn catalog_value(setting: &Setting) -> &'static Value {
    keys::default_of(setting)
}

/// `value`, `setting`'s in a layer the resolver checked or its catalog
/// default, as `T`, as [`Resolved::get`] reads it.
///
/// # Panics
///
/// As [`Resolved::get`].
pub fn value_as<'a, T: SettingValue<'a>>(setting: &Setting, value: &'a Value) -> T {
    read(setting, value)
}

/// `value`, `setting`'s, as `T`.
fn read<'a, T: SettingValue<'a>>(setting: &Setting, value: &'a Value) -> T {
    let name = setting.name;
    assert!(
        T::reads(&setting.kind) && T::CLEARABLE == setting.clearable,
        "invariant: `{name}` ({:?}, clearable: {}) is read as `{}`",
        setting.kind,
        setting.clearable,
        std::any::type_name::<T>(),
    );
    T::read(value).unwrap_or_else(|| {
        panic!("invariant: the resolver lets only `{name}`'s kind through, not `{value}`")
    })
}

/// Whether user configuration may hold `key`, by its authority.
fn user_settable(key: &str) -> bool {
    keys::find(key).is_some_and(|found| {
        !matches!(
            found.setting.authority,
            Authority::OperatorPolicy | Authority::Fixed
        )
    })
}

/// Whether `key` is fixed for a Run's life (ADRs 0102 and 0120).
fn fixed_for_run(key: &str) -> bool {
    FIXED_FOR_A_RUN.iter().any(|fixed| fixed.name == key)
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
        recipe_run: layers.recipe.is_some(),
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
    let default = layers.host.of(setting);
    let mut state = State {
        effective: Effective {
            value: default.clone(),
            layer: SettingsLayer::Default,
            origin: None,
            for_model: false,
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
            Some(layers.project_configuration.given()),
        )?;
    }
    if let Some(recipe) = &layers.recipe {
        if let Some(value) = recipe.declared.get(name) {
            recipe_may_set(policy, setting, &state.effective)?;
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
            Some(layers.run.given()),
        )?;
    }
    if let Some(cap) = &state.cap
        && state.effective.layer == SettingsLayer::Default
        && cap
            .value
            .as_u64()
            .is_some_and(|bound| above(&state.effective.value, bound))
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

/// Refuses a Recipe's value for `setting`, whose effective value below the
/// Recipe is `below`, where its authority wants an allowance `policy` does
/// not give.
fn recipe_may_set(
    policy: &Policy,
    setting: &Setting,
    below: &Effective,
) -> Result<(), SettingsError> {
    let Authority::NotByModelRecipeAllowedBy(allowance) = setting.authority else {
        return Ok(());
    };
    if allows(policy, allowance) {
        return Ok(());
    }
    let name = setting.name;
    Err(SettingsError::Locked {
        key: name.to_owned(),
        layer: SettingsLayer::Recipe,
        limit: limit(name, &below.value, below.layer),
        fix: format!(
            "a Recipe sets `{name}` only where the Operator policy sets `{allowance}` to true"
        ),
    })
}

/// Whether `policy` sets the boolean `allowance` to `true`, locked or not.
fn allows(policy: &Policy, allowance: &str) -> bool {
    policy
        .lock
        .get(allowance)
        .or_else(|| policy.settings.get(allowance))
        == Some(&Value::Bool(true))
}

fn limit(key: &str, value: &Value, layer: SettingsLayer) -> SettingLimit {
    SettingLimit {
        key: key.to_owned(),
        value: value.clone(),
        layer,
    }
}

/// Whether `value`, a count or a duration, or `null` for no bound, is above
/// `bound`.
fn above(value: &Value, bound: u64) -> bool {
    value.is_null() || value.as_u64().is_some_and(|value| value > bound)
}

/// Takes `value` from `layer` for `key`, as `given` for the run layer and
/// the project configuration a Client sent,
/// unless something holds the key: a lock, an Operator policy's cap the
/// value is past (a clear to no bound is past every cap), a Recipe's
/// bounds or the key's authority.
fn take(
    state: &mut State,
    key: Key,
    layer: SettingsLayer,
    value: &Value,
    given: Option<Given>,
) -> Result<(), SettingsError> {
    let given = given.unwrap_or_default();
    let name = key.setting.name;
    let refused = |limit: SettingLimit, fix: String| SettingsError::Locked {
        key: name.to_owned(),
        layer,
        limit,
        fix,
    };
    let current = limit(name, &state.effective.value, state.effective.layer);
    authority(key, layer, value, given.for_model, &current, &refused)?;
    if let Some(locked) = &state.locked {
        let fix = match locked.layer {
            SettingsLayer::Default => format!("`{name}` is an honesty guarantee no layer changes"),
            SettingsLayer::Recipe => format!("the Recipe does not expose `{name}`"),
            _ => format!("ask the operator to change the Operator policy's lock on `{name}`"),
        };
        return Err(refused(locked.clone(), fix));
    }
    if let Some(cap) = &state.cap
        && cap.value.as_u64().is_some_and(|bound| above(value, bound))
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
    state.effective.origin = given.origin;
    state.effective.for_model = given.for_model;
    Ok(())
}

/// The key's own authority over `layer` (ADR 0028 and its amendments);
/// `for_model` when a Client acting for a model gave `value`.
fn authority(
    key: Key,
    layer: SettingsLayer,
    value: &Value,
    for_model: bool,
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
        Authority::Configuration | Authority::ConfigurationProjectLimited(_)
            if above_configuration =>
        {
            Err(configuration_only())
        }
        Authority::ConfigurationExcept(allowed) if above_configuration && !allowed(value) => {
            Err(configuration_only())
        }
        Authority::ConfigurationProjectLimited(allowed)
            if layer == SettingsLayer::ProjectConfiguration && !allowed(value) =>
        {
            Err(refused(
                current.clone(),
                format!(
                    "a project's configuration gives `{name}` without what only the machine's own layers set, such as a Proxy provider's `price`, which a currency Limit trusts: set the provider's entry, its `price` included, in user configuration or an Operator policy"
                ),
            ))
        }
        Authority::NotByRecipe | Authority::NotByRecipeOrModel
            if layer == SettingsLayer::Recipe =>
        {
            Err(refused(
                current.clone(),
                format!("a Recipe never sets `{name}`; it may only expose it"),
            ))
        }
        Authority::NotByRecipeOrModel
        | Authority::NotByModel
        | Authority::NotByModelRecipeAllowedBy(_)
            if for_model =>
        {
            Err(refused(
                current.clone(),
                format!(
                    "a Client acting for a model never sets `{name}`: set it in the program's own code, in configuration or in an Operator policy"
                ),
            ))
        }
        Authority::Narrowed if above_configuration && !narrows(key, value, &current.value) => {
            Err(refused(
                current.clone(),
                format!(
                    "a Recipe or a run only narrows `{name}`; widen it in configuration or an Operator policy"
                ),
            ))
        }
        Authority::HostsNarrowedByRecipeOrModel(list)
            if (layer == SettingsLayer::Recipe || for_model)
                && !hosts::narrows(list, value, &current.value) =>
        {
            Err(refused(
                current.clone(),
                format!(
                    "a Recipe or a Client acting for a model only narrows `{name}`: list no host the layers below do not cover in `egress.allowed_domains`, and leave none they list uncovered in `egress.blocked_domains`; widen it in the program's own code, in configuration or in an Operator policy"
                ),
            ))
        }
        Authority::Every
        | Authority::HostsNarrowedByRecipeOrModel(_)
        | Authority::OperatorPolicy
        | Authority::Configuration
        | Authority::ConfigurationExcept(_)
        | Authority::ConfigurationProjectLimited(_)
        | Authority::NotByRecipe
        | Authority::NotByRecipeOrModel
        | Authority::NotByModel
        | Authority::NotByModelRecipeAllowedBy(_)
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
