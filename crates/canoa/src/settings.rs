//! How a provider describes the values its form asks for: a guatiao schema, and beside it the guatiao-intake form that says how to
//! show it. The same description serves every kind whose provider asks the person for something (a profile item, a catalog source),
//! and the application draws it with the renderer it draws a speech provider's settings with.

use alloc::string::String;

use guatiao::library::ProviderError;
use guatiao::schema::SchemaRef;
use guatiao::{Alloc, Schema, Status, Value};
use guatiao_intake::{FormRef, Screen};

/// The schema and the form of the values `S` holds.
///
/// # Errors
///
/// When `S`'s schema or form cannot be written.
pub fn describe<S: Schema + Screen>() -> Result<(Value, Value), ProviderError> {
    let wrong = |why: &dyn core::fmt::Display| {
        ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &alloc::format!("{why}"))
    };
    let schema = S::schema().map_err(|why| wrong(&why))?;
    let form = S::form(Alloc::rust()).map_err(|why| wrong(&why))?;
    Ok((schema, form))
}

/// Why `schema` and `form` cannot be offered, or `None` when they can: a schema is a schema, a form needs one, and the form names
/// only fields the schema has.
#[must_use]
pub fn problem(schema: Option<&Value>, form: Option<&Value>) -> Option<String> {
    let Some(schema) = schema else {
        return form.map(|_| "it has a form but no schema for the form to show".into());
    };
    let Some(shown) = SchemaRef::new(schema) else {
        return Some("its schema is not a schema of an object".into());
    };
    let form = form?;
    let Some(form) = FormRef::new(form) else {
        return Some("its form is not a form".into());
    };
    guatiao_intake::check(shown, form)
        .err()
        .map(|why| alloc::format!("its form does not fit its schema: {why}"))
}
