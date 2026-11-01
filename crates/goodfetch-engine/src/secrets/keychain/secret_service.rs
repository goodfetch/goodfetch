//! The Secret Service on the session's D-Bus, Linux's OS keychain
//! (`gnome-keyring`, `KWallet`), spoken directly over zbus: a `plain`
//! session, which carries a value only on the user's own bus, then each
//! item found by the `service` and `username` attributes that libsecret's
//! `secret-tool` and keyring-core's stores write. A locked collection is
//! refused rather than prompted for, since no one may be there to answer.

use std::collections::HashMap;

use secrecy::{ExposeSecret as _, SecretString};

use zbus::{Connection, DBusError as _, OwnedObjectPath, Variant};

use super::super::{ConfigProblem, PartyText, ProviderFailure};
use super::no_keychain;

const DESTINATION: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE: &str = "org.freedesktop.Secret.Service";
const COLLECTION: &str = "org.freedesktop.Secret.Collection";
const ITEM: &str = "org.freedesktop.Secret.Item";

/// The object path that stands for "none": no prompt, no collection.
const NONE: &str = "/";

/// A session with the Secret Service of this user's session bus.
pub(super) struct SecretService {
    connection: Connection,
    session: OwnedObjectPath,
}

/// A Secret Service secret: its session, parameters, value and type.
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

impl SecretService {
    /// Opens a session with the session bus's Secret Service.
    ///
    /// # Errors
    ///
    /// The translation of the bus's failure, such as no Secret Service.
    pub(super) async fn open() -> Result<Self, ProviderFailure> {
        let connection = Connection::session().await.map_err(translate)?;
        let reply = connection
            .call_method(
                Some(DESTINATION),
                SERVICE_PATH,
                Some(SERVICE),
                "OpenSession",
                &("plain", Variant::from("")),
            )
            .await
            .map_err(translate)?;
        let (_, session): (zbus::OwnedVariant, OwnedObjectPath) =
            reply.body().deserialize().map_err(translate)?;
        Ok(Self {
            connection,
            session,
        })
    }

    /// The value of `service`'s `account`.
    ///
    /// # Errors
    ///
    /// `Missing` when no item matches, the translation of the bus's failure
    /// otherwise.
    pub(super) async fn read(
        &self,
        service: &str,
        account: &str,
    ) -> Result<SecretString, ProviderFailure> {
        let item = match self.items(service, account).await?.as_slice() {
            [] => return Err(ProviderFailure::Missing),
            [item] => item.clone(),
            _ => return Err(ambiguous()),
        };
        let reply = self
            .connection
            .call_method(
                Some(DESTINATION),
                item.as_str(),
                Some(ITEM),
                "GetSecret",
                &(&self.session,),
            )
            .await
            .map_err(translate)?;
        let (_, _, value, _): Secret = reply.body().deserialize().map_err(translate)?;
        String::from_utf8(value)
            .map(SecretString::from)
            .map_err(|_| not_text())
    }

    /// Stores `value` as `service`'s `account` in the default collection,
    /// in place of any value it had.
    ///
    /// # Errors
    ///
    /// `NotConfigured` with no default collection, a refusal when it is
    /// locked, the translation of the bus's failure otherwise.
    pub(super) async fn store(
        &self,
        service: &str,
        account: &str,
        value: &SecretString,
    ) -> Result<(), ProviderFailure> {
        let reply = self
            .call(SERVICE_PATH, SERVICE, "ReadAlias", &("default",))
            .await?;
        let collection: OwnedObjectPath = reply.body().deserialize().map_err(translate)?;
        if collection.as_str() == NONE {
            return Err(ProviderFailure::NotConfigured(ConfigProblem {
                fix: "The Secret Service has no default collection; create one (such as gnome-keyring's login keyring), then try again.".to_owned(),
            }));
        }
        let attributes = HashMap::from([("service", service), ("username", account)]);
        let properties = HashMap::from([
            (
                "org.freedesktop.Secret.Item.Label",
                Variant::from(format!("{service}/{account}")),
            ),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Variant::from(attributes),
            ),
        ]);
        let secret = (
            &self.session,
            Vec::<u8>::new(),
            value.expose_secret().as_bytes(),
            "text/plain",
        );
        let reply = self
            .call(
                collection.as_str(),
                COLLECTION,
                "CreateItem",
                &(properties, secret, true),
            )
            .await?;
        let (_, prompt): (OwnedObjectPath, OwnedObjectPath) =
            reply.body().deserialize().map_err(translate)?;
        prompted(&prompt)
    }

    /// Removes every item of `service`'s `account`.
    ///
    /// # Errors
    ///
    /// A refusal when one is locked, the translation of the bus's failure
    /// otherwise.
    pub(super) async fn remove(&self, service: &str, account: &str) -> Result<(), ProviderFailure> {
        for item in self.items(service, account).await? {
            let reply = self.call(item.as_str(), ITEM, "Delete", &()).await?;
            let prompt: OwnedObjectPath = reply.body().deserialize().map_err(translate)?;
            prompted(&prompt)?;
        }
        Ok(())
    }

    /// The unlocked items of `service`'s `account`; a locked one is refused.
    async fn items(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Vec<OwnedObjectPath>, ProviderFailure> {
        let attributes = HashMap::from([("service", service), ("username", account)]);
        let reply = self
            .call(SERVICE_PATH, SERVICE, "SearchItems", &(attributes,))
            .await?;
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            reply.body().deserialize().map_err(translate)?;
        if unlocked.is_empty() && !locked.is_empty() {
            return Err(locked_out());
        }
        Ok(unlocked)
    }

    async fn call<B>(
        &self,
        path: &str,
        interface: &str,
        method: &str,
        body: &B,
    ) -> Result<zbus::Message, ProviderFailure>
    where
        B: serde::Serialize + zbus::wire::DynamicType,
    {
        self.connection
            .call_method(Some(DESTINATION), path, Some(interface), method, body)
            .await
            .map_err(translate)
    }
}

/// Refused when the call needs a prompt, which no one may answer.
fn prompted(prompt: &OwnedObjectPath) -> Result<(), ProviderFailure> {
    if prompt.as_str() == NONE {
        Ok(())
    } else {
        Err(locked_out())
    }
}

fn locked_out() -> ProviderFailure {
    ProviderFailure::Refused {
        fix: "The OS keychain's collection is locked, or this program may not read the item; unlock it (on Linux, the login keyring), then try again.".to_owned(),
    }
}

fn ambiguous() -> ProviderFailure {
    ProviderFailure::Refused {
        fix: "Several keychain items match this service and account; remove all but one."
            .to_owned(),
    }
}

fn not_text() -> ProviderFailure {
    ProviderFailure::Refused {
        fix: "The keychain item holds no UTF-8 text; store the value as text.".to_owned(),
    }
}

/// What a failure of the bus or the Secret Service means for a
/// resolution: the translation table (ADR 0127), by the D-Bus error's name
/// where it has one.
pub(in crate::secrets) fn translate(error: zbus::Error) -> ProviderFailure {
    let name = match &error {
        zbus::Error::MethodError(name, ..) => Some(name.as_str().to_owned()),
        zbus::Error::FDO(error) => Some(error.name().as_str().to_owned()),
        zbus::Error::DBus(error) => Some(error.name().as_str().to_owned()),
        _ => None,
    };
    let unreachable = matches!(
        error,
        zbus::Error::InputOutput(_)
            | zbus::Error::Connection(..)
            | zbus::Error::Address(_)
            | zbus::Error::Handshake(_)
    );
    translate_named(name.as_deref(), unreachable, move || error.to_string())
}

/// The table itself: a failure the bus names `name`, or one with no name,
/// `unreachable` when the bus could not be reached; `text` gives the
/// party's own text, kept unread for a failure the table does not know.
pub(in crate::secrets) fn translate_named(
    name: Option<&str>,
    unreachable: bool,
    text: impl FnOnce() -> String,
) -> ProviderFailure {
    match name {
        Some(
            "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner",
        ) => no_keychain(),
        None if unreachable => no_keychain(),
        Some("org.freedesktop.Secret.Error.IsLocked") => locked_out(),
        Some("org.freedesktop.DBus.Error.AccessDenied") => ProviderFailure::Refused {
            fix:
                "The session bus refused this program the Secret Service; allow it, then try again."
                    .to_owned(),
        },
        Some(
            "org.freedesktop.Secret.Error.NoSuchObject"
            | "org.freedesktop.DBus.Error.UnknownObject"
            | "org.freedesktop.DBus.Error.UnknownMethod",
        ) => ProviderFailure::Missing,
        name => {
            tracing::warn!(
                provider = "keychain",
                failure = name.unwrap_or("unnamed"),
                "an unrecognized failure of the Secret Service"
            );
            ProviderFailure::Unrecognized(PartyText::new(text()))
        }
    }
}
