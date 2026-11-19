//! The `settings_section` kind: a page, or a group on a page, of the application's Settings.
//!
//! Which sections Settings has beyond the ones the application draws itself is not Canoa's to say: each is a provider of this kind,
//! the ones Canoa carries and the ones an extension adds alike. A provider describes itself (its id, its name, its icon, where it
//! sits and the form it asks for) and the application draws the form from the guatiao schema and the guatiao-intake form it gives,
//! with the renderer every such form is drawn with.
//!
//! A section either sits in the list of Settings under a heading of its own, or is a group inside another section (`under`).
//! Its values live in one of two places: in a table of canoa.cfg the host writes for it (`config`), or with the provider, which the
//! host asks to read and to write them (`provider`).
//!
//! The models are plain data, the same in every language.

use alloc::string::String;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, Status, ToValue, Value};
use guatiao_intake::Screen;

/// The name of the `settings_section` kind.
pub const KIND: &str = "settings_section";

/// The versions of the `settings_section` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// The heading a section sits under in Settings' list when it names none.
pub const DEFAULT_GROUP: &str = "Extensions";

/// The most bytes of values a section keeps, so a provider cannot make a section large.
pub const MAX_VALUES: usize = 64 * 1024;

/// An error with status `GUATIAO_ERR_BAD_VALUE`: the values do not fit the section's form.
#[must_use]
pub fn bad_value_error(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// An error with status `GUATIAO_ERR_INTERNAL`.
#[must_use]
pub fn internal_error(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why.as_ref())
}

/// Where a section's values are kept.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Stored {
    /// In the table `[settings.ID]` of the host's canoa.cfg, which the host writes after checking the values against the schema.
    Config,
    /// With the provider: the host calls [`SettingsSection::read`] and [`SettingsSection::write`].
    Provider,
}

/// What a section is, from [`SettingsSection::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SectionInfo {
    /// The id the section is found by and, for [`Stored::Config`], the key its values have in `[settings]`: a word of lower-case
    /// letters, digits and `_`, 1 to 32.
    pub id: String,
    /// Its name for people.
    pub title: String,
    /// A line saying what it holds, shown above its form.
    pub help: Option<String>,
    /// The name of one of the application's icons; a name it does not know draws the default.
    pub icon: Option<String>,
    /// The heading it sits under in the list of sections, for a section of its own; [`DEFAULT_GROUP`] when none.
    pub group: Option<String>,
    /// The id of an existing section this is a group in, instead of a section of its own.
    pub under: Option<String>,
    /// Words the search over Settings finds it by, besides its title.
    pub keywords: Option<String>,
    /// Where its values are kept.
    pub stored: Stored,
    /// The values it takes, as a guatiao schema of an object whose members are what its form asks for. A member the schema marks
    /// sensitive is never answered in clear.
    pub schema: Option<Value>,
    /// How the schema is shown, a guatiao-intake form document; none shows the form the schema implies.
    pub form: Option<Value>,
}

impl SectionInfo {
    /// A section of its own with no fields, kept in canoa.cfg.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            help: None,
            icon: None,
            group: None,
            under: None,
            keywords: None,
            stored: Stored::Config,
            schema: None,
            form: None,
        }
    }

    /// This section taking the values `S` describes, shown with the screen `S` declares.
    ///
    /// # Errors
    ///
    /// When `S`'s schema or form cannot be written.
    pub fn taking<S: Schema + Screen>(mut self) -> Result<Self, ProviderError> {
        let (schema, form) = crate::settings::describe::<S>()?;
        self.schema = Some(schema);
        self.form = Some(form);
        Ok(self)
    }

    /// Why this provider cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = |text: &str| {
            !text.is_empty()
                && text.len() <= 32
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        };
        if !word(&self.id) {
            return Some(alloc::format!(
                "the section id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the section {:?} has no title", self.id));
        }
        if let Some(under) = &self.under {
            if !word(under) {
                return Some(alloc::format!(
                    "the section {:?} sits under {under:?}, which is no section id",
                    self.id
                ));
            }
            if *under == self.id {
                return Some(alloc::format!(
                    "the section {:?} sits under itself",
                    self.id
                ));
            }
        }
        if self
            .group
            .as_deref()
            .is_some_and(|group| group.trim().is_empty())
        {
            return Some(alloc::format!(
                "the section {:?} has an empty group",
                self.id
            ));
        }
        crate::settings::problem(self.schema.as_ref(), self.form.as_ref())
    }
}

/// A page of Settings, or a group on one.
///
/// `describe` answers the same on every call with nothing run. `read` and `write` are asked only for a [`Stored::Provider`]
/// section; a [`Stored::Config`] section answers them with nothing.
#[coqui_acp_api::guatiao::kind(name = "settings_section", crate = ::coqui_acp_api::guatiao)]
pub trait SettingsSection: Send + Sync {
    /// What this section is and the form it asks for.
    fn describe(&self) -> Result<SectionInfo, ProviderError>;

    /// What the section holds now, as a JSON object keyed by the schema's member names. A sensitive member is left out.
    fn read(&self) -> Result<String, ProviderError>;

    /// Keeps `values`, a JSON object keyed by the schema's member names that the host has already checked against the schema. A
    /// sensitive member the person left empty is absent, so what was kept stays.
    fn write(&self, values: String) -> Result<(), ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    use guatiao_intake::Form;

    #[derive(ToValue, FromValue, Schema, Form)]
    #[allow(dead_code, reason = "only its description is read")]
    struct Values {
        /// Where it goes.
        #[schema(title = "Address")]
        url: String,
    }

    #[test]
    fn a_section_with_a_word_for_an_id_a_title_and_a_form_that_fits_its_schema_is_usable() {
        let info = SectionInfo::new("region", "Region")
            .taking::<Values>()
            .unwrap();
        assert_eq!(info.problem(), None);
        assert!(info.schema.is_some() && info.form.is_some());
        assert_eq!(SectionInfo::new("a", "A").problem(), None);
    }

    #[test]
    fn a_section_that_is_no_word_has_no_title_sits_under_itself_or_has_a_form_without_a_schema_is_refused()
     {
        for id in ["", "Region", "a b", &"a".repeat(33)] {
            assert!(SectionInfo::new(id, "T").problem().is_some(), "{id:?}");
        }
        assert!(SectionInfo::new("a", " ").problem().is_some());
        let mut info = SectionInfo::new("a", "A");
        info.under = Some("a".into());
        assert!(info.problem().is_some_and(|why| why.contains("itself")));
        info.under = Some("Bad Id".into());
        assert!(info.problem().is_some());
        info.under = Some("general".into());
        assert_eq!(info.problem(), None);
        info.group = Some(" ".into());
        assert!(info.problem().is_some());
        let mut info = SectionInfo::new("a", "A");
        info.form = Some(Value::from(true));
        assert!(info.problem().is_some_and(|why| why.contains("no schema")));
    }
}
