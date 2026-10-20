//! Settings (ADR 0028 as amended by ADRs 0083, 0087 and 0098): every engine
//! parameter resolves through five layers, from its built-in default
//! through the Operator policy, user and project configuration and a
//! Recipe to the run, with locks, caps, a Recipe's exposed bounds and each
//! key's authority, and every effective value can say which layer set it
//! and which locked it (`config explain`, the run report). Both Products
//! read one major's Operator policy and user configuration from Goodfetch
//! home, each applying its own keys and listing the other's.
//!
//! Its interface: [`Settings`] reads the machine's layers and resolves a
//! Run's [`RunSettings`]; [`resolve`] is the pure resolver over [`Layers`];
//! [`SettingsError`] is every refusal, with its Error code; the files'
//! reading and rewriting and the environment map's check are its other
//! entries.

mod environment;
mod error;
mod files;
mod keys;
mod resolve;
#[cfg(test)]
mod tests;
mod values;

use std::sync::Arc;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::{ProjectSettings, RecipeManifest, RunOrigin, SettingsLayer};
use serde_json::{Map, Value};

pub use environment::{check_environment, deployment_variable};
pub use error::SettingsError;
pub use files::{
    MachineLayers, OPERATOR_POLICY_FILE, PROJECT_CONFIGURATION_FILE, SettingsFiles,
    USER_CONFIGURATION_FILE, configuration, find_project, load_project, operator_policy,
    policy_version, rewrite,
};
pub use resolve::{Effective, Layers, Policy, Recipe, Resolved, Unapplied, resolve};
pub use values::Values;

use crate::tasks::TaskOwner;

/// The engine's settings: where its machine's layers are, and the Product
/// whose keys it applies.
#[derive(Clone)]
pub struct Settings(Arc<Inner>);

impl std::fmt::Debug for Settings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Settings")
            .field("files", &self.0.files)
            .field("product", &self.0.product)
            .finish_non_exhaustive()
    }
}

struct Inner {
    files: SettingsFiles,
    product: Product,
    owner: TaskOwner,
}

/// What a scope that opens gives the resolver beside the machine's layers.
#[derive(Clone, Debug, Default)]
pub struct Opening {
    /// The Product the Run needs.
    pub run: Option<Product>,
    /// The project configuration the Client sent.
    pub project: Option<ProjectSettings>,
    /// The Recipe, when the Run is one's.
    pub recipe: Option<RecipeManifest>,
    /// The run layer's values, nested by key.
    pub settings: Map<String, Value>,
    /// Where the run layer's values came from.
    pub origin: Option<RunOrigin>,
}

impl Settings {
    /// The settings of an engine of `product` whose layers are `files`,
    /// reading them on `owner`'s blocking policy.
    pub fn new(files: SettingsFiles, product: Product, owner: TaskOwner) -> Self {
        Self(Arc::new(Inner {
            files,
            product,
            owner,
        }))
    }

    /// Where its files are.
    pub fn files(&self) -> &SettingsFiles {
        &self.0.files
    }

    /// The Product whose keys it applies.
    pub fn product(&self) -> Product {
        self.0.product
    }

    /// Reads the machine's layers now.
    ///
    /// # Errors
    ///
    /// As [`SettingsFiles::load`].
    ///
    /// # Cancel safety
    ///
    /// Dropping the future leaves the read to finish on its own.
    pub async fn machine(&self) -> Result<MachineLayers, SettingsError> {
        let files = self.0.files.clone();
        let product = self.0.product;
        self.0.owner.blocking(move || files.load(product)).await
    }

    /// Resolves the settings of a scope that opens with `opening`, against
    /// the machine's layers as they are now; refused before the scope opens
    /// (ADR 0083).
    ///
    /// # Errors
    ///
    /// Every refusal of [`SettingsError`], the first found.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future resolves nothing.
    pub async fn open(
        &self,
        opening: Opening,
    ) -> Result<(RunSettings, MachineLayers), SettingsError> {
        let machine = self.machine().await?;
        let engine = self.0.product;
        let project = match &opening.project {
            Some(project) => Values::read(
                SettingsLayer::ProjectConfiguration,
                &project.settings,
                engine,
            )?,
            None => Values::default(),
        };
        let recipe = opening
            .recipe
            .as_ref()
            .map(|manifest| Recipe::read(manifest, engine))
            .transpose()?;
        let mut run = Values::read(SettingsLayer::Run, &opening.settings, engine)?;
        if let Some(origin) = opening.origin {
            run = run.from_origin(origin);
        }
        let layers = Layers {
            operator_policy: machine.operator_policy.clone(),
            user_configuration: machine.user_configuration.clone(),
            project_configuration: project,
            recipe,
            run,
        };
        let resolved = resolve(&layers, engine, opening.run.unwrap_or(engine))?;
        Ok((
            RunSettings {
                settings: self.clone(),
                resolved,
            },
            machine,
        ))
    }
}

/// One Run's resolved settings, which a Run already going reads again at
/// each admission point, so a tightened Operator policy cap reaches it
/// there (ADR 0028).
#[derive(Clone)]
pub struct RunSettings {
    settings: Settings,
    resolved: Resolved,
}

impl RunSettings {
    /// What the Run opened with.
    pub fn resolved(&self) -> &Resolved {
        &self.resolved
    }

    /// What the Run takes at an admission point: its values under the
    /// Operator policy's caps as they are now. A policy that no longer reads
    /// leaves the values it opened with, with a warning.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future changes nothing.
    pub async fn at_admission(&self) -> Resolved {
        match self.settings.machine().await {
            Ok(machine) => self.resolved.under_caps_of(&machine.operator_policy),
            Err(error) => {
                tracing::warn!(code = error.code(), "{error}; the Run keeps its settings");
                self.resolved.clone()
            }
        }
    }
}
