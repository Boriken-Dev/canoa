//! What the pane is told about the Office window it sits in: which application, and what is open there.
//!
//! Word, Excel and PowerPoint relate a chat to the active document (its full name, as the web add-in's `document.url` gives it);
//! Outlook relates it to the mail conversation of the open or selected message. The object model is read through `IDispatch` and
//! only for the two or three properties named here.

use windows::Win32::System::Com::IDispatch;

use crate::automation::{call_with_number, get_number, get_object, get_string};
use crate::relation::{Relation, conversation_relation, document_relation};

/// The Office application a window belongs to, and what is open in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    /// `word`, `excel`, `powerpoint` or `outlook`: what the app's `client=` says.
    pub client: &'static str,
    /// What the open item relates a chat to, when it has a relation.
    pub relation: Option<Relation>,
}

/// The application a program file is, by name: `WINWORD.EXE` is Word. `office` for any other.
#[must_use]
pub fn client_of_exe(exe: &str) -> &'static str {
    match exe.to_ascii_lowercase().as_str() {
        "winword.exe" => "word",
        "excel.exe" => "excel",
        "powerpnt.exe" => "powerpoint",
        "outlook.exe" => "outlook",
        _ => "office",
    }
}

/// The name of this process's program file.
#[must_use]
pub fn current_exe_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

/// What is open, asked of `application` (the object Office gave `OnConnection`) and, for Outlook, of `window` (the Explorer or
/// Inspector the ribbon button was clicked in). An object that cannot be read leaves the chat with no relation, never an error.
#[must_use]
pub fn current(
    client: &'static str,
    application: Option<&IDispatch>,
    window: Option<&IDispatch>,
) -> Context {
    let document = |property: &str| {
        let url = application.and_then(|app| full_name(app, property))?;
        document_relation(client, &url)
    };
    let relation = match client {
        "word" => document("ActiveDocument"),
        "excel" => document("ActiveWorkbook"),
        "powerpoint" => document("ActivePresentation"),
        "outlook" => window.and_then(mail_item).and_then(|item| {
            let id = get_string(&item, "ConversationID").ok()?;
            conversation_relation(&id, &get_string(&item, "Subject").unwrap_or_default())
        }),
        _ => None,
    };
    Context { client, relation }
}

/// The full name (a path, or an address for a document kept online) of the object `property` of `application` names.
fn full_name(application: &IDispatch, property: &str) -> Option<String> {
    get_string(&get_object(application, property).ok()?, "FullName").ok()
}

/// The message in an Inspector, or the first one selected in an Explorer.
fn mail_item(window: &IDispatch) -> Option<IDispatch> {
    if let Ok(item) = get_object(window, "CurrentItem") {
        return Some(item);
    }
    let selection = get_object(window, "Selection").ok()?;
    if get_number(&selection, "Count").ok()? < 1 {
        return None;
    }
    call_with_number(&selection, "Item", 1).ok()
}
