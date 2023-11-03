//! Provides a simple `SimpleEntityProvider` and an implementation using a local file system,
//! in the form a `EntityProvider`.
//!
//! # Examples
//!
//! ```
//! use cedar_local_agent::public::file::entity_provider::{ConfigBuilder, EntityProvider};
//!
//! let entity_provider = EntityProvider::new(
//!     ConfigBuilder::default()
//!         .schema_path("schema_path")
//!         .entities_path("entities_path")
//!         .build()
//!         .unwrap()
//! );
//! ```
use std::fmt::Debug;
use std::fs::File;
use std::io::Error;
use std::sync::Arc;

use async_trait::async_trait;
use cedar_policy::{Entities, Request, Schema, SchemaError};
use cedar_policy_core::entities::EntitiesError;
use derive_builder::Builder;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::public::{
    EntityProviderError, SimpleEntityProvider, UpdateProviderData, UpdateProviderDataError,
};

/// `ConfigBuilder` provides the data required to build the
/// `EntityProvider`.  Favor the builder to use this object, see example.
///
/// # Examples
/// ```
/// use cedar_local_agent::public::file::entity_provider::ConfigBuilder;
///
/// let config = ConfigBuilder::default()
///     .entities_path("entities_path".to_string())
///     .schema_path("schema_path".to_string())
///     .build()
///     .unwrap();
/// ```
#[derive(Default, Builder, Debug)]
#[builder(setter(into))]
pub struct Config {
    /// File path to the entities file
    #[builder(setter(into, strip_option), default)]
    pub entities_path: Option<String>,
    /// File path to the schema file
    #[builder(setter(into, strip_option), default)]
    pub schema_path: Option<String>,
}

/// `EntityProvider` structure implements the `SimpleEntityProvider` trait.
#[derive(Debug)]
pub struct EntityProvider {
    /// Entities path, stored to allow refreshing from disk.
    entities_path: Option<String>,
    /// Schema path, stored to allow refreshing from disk.
    schema_path: Option<String>,
    /// Entities can be updated through a back ground thread.
    entities: RwLock<Arc<Entities>>,
}

/// `ProviderError` thrown by the constructor of the `EntityProvider`.
#[derive(Error, Debug)]
pub enum ProviderError {
    /// Can't read from disk or find the file
    #[error("IO Error: {0}")]
    IOError(#[source] std::io::Error),
    /// Schema file is malformed in some way
    #[error("The Schema file failed to be parsed: {0}")]
    SchemaParseError(#[source] SchemaError),
    /// Entities file is malformed in some way
    #[error("The Entities failed to be parsed: {0}")]
    EntitiesError(#[source] EntitiesError),
    /// When the file system entity provider cannot update it's data
    #[error("The update provider failed to update the entities: {0}")]
    UpdateError(#[source] UpdateProviderDataError),
}

impl From<Error> for ProviderError {
    fn from(value: Error) -> Self {
        Self::IOError(value)
    }
}

impl From<SchemaError> for ProviderError {
    fn from(value: SchemaError) -> Self {
        Self::SchemaParseError(value)
    }
}

impl From<EntitiesError> for ProviderError {
    fn from(value: EntitiesError) -> Self {
        Self::EntitiesError(value)
    }
}

/// Implements the `EntityProvider`.
impl EntityProvider {
    /// Builds a new `EntityProvider`.
    ///
    /// # Examples
    ///
    /// ```
    /// use cedar_local_agent::public::file::entity_provider::{EntityProvider, ConfigBuilder};
    ///
    /// let entity_provider = EntityProvider::new(
    ///     ConfigBuilder::default()
    ///         .schema_path("schema_path")
    ///         .entities_path("entities_path")
    ///         .build()
    ///         .unwrap()
    /// );
    /// ```
    ///
    /// # Errors
    ///
    /// This constructor will return a `EntityProvider` error if the applicable
    /// entity or schema data is not a valid path or improperly formatted.
    pub fn new(configuration: Config) -> Result<Self, ProviderError> {
        let entities = if let Some(entities_path) = configuration.entities_path.as_ref() {
            let entities_file = File::open(entities_path)?;

            let entities = if let Some(schema_path) = configuration.schema_path.as_ref() {
                let schema_file = File::open(schema_path)?;
                let schema = Schema::from_file(schema_file)?;
                Entities::from_json_file(entities_file, Some(&schema))?
            } else {
                Entities::from_json_file(entities_file, None)?
            };

            entities
        } else {
            Entities::empty()
        };

        Ok(Self {
            entities_path: configuration.entities_path,
            schema_path: configuration.schema_path,
            entities: RwLock::new(Arc::new(entities)),
        })
    }
}

/// Default Entity Provider that has no entities
impl Default for EntityProvider {
    fn default() -> Self {
        Self {
            entities_path: None,
            schema_path: None,
            entities: RwLock::new(Arc::new(Entities::empty())),
        }
    }
}

/// Implements the update provider data trait
#[async_trait]
impl UpdateProviderData for EntityProvider {
    async fn update_provider_data(&self) -> Result<(), UpdateProviderDataError> {
        let entities = if let Some(entities_path) = self.entities_path.as_ref() {
            let entities_file = File::open(entities_path)
                .map_err(|e| UpdateProviderDataError::General(Box::new(e)))?;

            let entities = if let Some(schema_path) = self.schema_path.as_ref() {
                let schema_file = File::open(schema_path)
                    .map_err(|e| UpdateProviderDataError::General(Box::new(e)))?;
                let schema = Schema::from_file(schema_file)
                    .map_err(|e| UpdateProviderDataError::General(Box::new(e)))?;
                Entities::from_json_file(entities_file, Some(&schema))
                    .map_err(|e| UpdateProviderDataError::General(Box::new(e)))?
            } else {
                Entities::from_json_file(entities_file, None)
                    .map_err(|e| UpdateProviderDataError::General(Box::new(e)))?
            };

            entities
        } else {
            Entities::empty()
        };

        {
            let mut entities_data = self.entities.write().await;
            *entities_data = Arc::new(entities);
        }

        Ok(())
    }
}

/// The `EntityProvider` returns all the `Entities` read from disk.  The
/// cedar `Request` is unused for this use case.
#[async_trait]
impl SimpleEntityProvider for EntityProvider {
    /// Get Entities.
    async fn get_entities(&self, _: &Request) -> Result<Arc<Entities>, EntityProviderError> {
        Ok(self.entities.read().await.clone())
    }
}

#[cfg(test)]
mod test {
    use cedar_policy::{Context, Request};

    use crate::public::file::entity_provider::{ConfigBuilder, EntityProvider};
    use crate::public::{SimpleEntityProvider, UpdateProviderData};

    #[test]
    fn entity_provider_default_is_ok() {
        assert!(EntityProvider::default().entities_path.is_none());
        assert!(EntityProvider::default().schema_path.is_none());
    }

    #[test]
    fn entity_provider_is_ok() {
        assert!(EntityProvider::new(
            ConfigBuilder::default()
                .entities_path("tests/data/sweets.entities.json")
                .schema_path("tests/data/sweets.schema.cedar.json")
                .build()
                .unwrap(),
        )
        .is_ok());
    }

    #[test]
    fn entity_provider_is_ok_no_schema() {
        assert!(EntityProvider::new(
            ConfigBuilder::default()
                .entities_path("tests/data/sweets.entities.json")
                .build()
                .unwrap(),
        )
        .is_ok());
    }

    #[test]
    fn entity_provider_is_ok_no_input() {
        assert!(EntityProvider::new(ConfigBuilder::default().build().unwrap(),).is_ok());
    }

    #[tokio::test]
    async fn entity_provider_get_entities_ok_no_input() {
        let provider = EntityProvider::new(ConfigBuilder::default().build().unwrap());

        assert!(provider.is_ok());
        assert!(provider
            .unwrap()
            .get_entities(&Request::new(
                Some(r#"User::"Eric""#.parse().unwrap()),
                Some(r#"Action::"View""#.parse().unwrap()),
                Some(r#"Box::"10""#.parse().unwrap()),
                Context::empty(),
            ))
            .await
            .is_ok());
    }

    #[test]
    fn entity_provider_is_io_error_no_entities() {
        let error = EntityProvider::new(
            ConfigBuilder::default()
                .entities_path("not_a_file")
                .build()
                .unwrap(),
        );

        assert!(error.is_err());
        assert_eq!(
            error.err().unwrap().to_string(),
            "IO Error: No such file or directory (os error 2)"
        );
    }

    #[test]
    fn entity_provider_is_io_error_no_schema() {
        let error = EntityProvider::new(
            ConfigBuilder::default()
                .entities_path("tests/data/sweets.entities.json")
                .schema_path("not_a_file")
                .build()
                .unwrap(),
        );

        assert!(error.is_err());
        assert_eq!(
            error.err().unwrap().to_string(),
            "IO Error: No such file or directory (os error 2)"
        );
    }

    #[test]
    fn entity_provider_malformed_schema() {
        let error = EntityProvider::new(
            ConfigBuilder::default()
                .entities_path("tests/data/sweets.entities.json")
                .schema_path("tests/data/schema_bad.cedarschema.json")
                .build()
                .unwrap(),
        );

        assert!(error.is_err());
        assert_eq!(error.err().unwrap().to_string(), "The Schema file failed to be parsed: JSON Schema file could not be parsed: expected `,` or `}` at line 61 column 11");
    }

    #[test]
    fn entity_provider_malformed_entities() {
        let error = EntityProvider::new(
            ConfigBuilder::default()
                .entities_path("tests/data/malformed_entities.json")
                .build()
                .unwrap(),
        );

        assert!(error.is_err());
        assert_eq!(error.err().unwrap().to_string(), "The Entities failed to be parsed: entities deserialization error: invalid type: map, expected a sequence at line 1 column 1");
    }

    #[tokio::test]
    async fn entity_provider_update_is_ok() {
        let provider = EntityProvider::new(
            ConfigBuilder::default()
                .entities_path("tests/data/sweets.entities.json")
                .schema_path("tests/data/sweets.schema.cedar.json")
                .build()
                .unwrap(),
        );

        assert!(provider.is_ok());
        assert!(provider.unwrap().update_provider_data().await.is_ok());
    }

    #[tokio::test]
    async fn entity_provider_update_is_ok_no_schema() {
        let provider = EntityProvider::new(
            ConfigBuilder::default()
                .entities_path("tests/data/sweets.entities.json")
                .build()
                .unwrap(),
        );

        assert!(provider.is_ok());
        assert!(provider.unwrap().update_provider_data().await.is_ok());
    }

    #[tokio::test]
    async fn entity_provider_update_is_ok_no_input() {
        let provider = EntityProvider::new(ConfigBuilder::default().build().unwrap());

        assert!(provider.is_ok());
        assert!(provider.unwrap().update_provider_data().await.is_ok());
    }
}