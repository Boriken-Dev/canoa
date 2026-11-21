//! What the pane is told is open, read through `IDispatch` from stand-in Office objects: a Word application with an active document, an
//! Outlook message window and Explorer with a selection. No Office application is involved.

#![cfg(windows)]
#![allow(unsafe_code, non_snake_case)]

use std::collections::BTreeMap;

use canoa_office::context::{Context, client_of_exe, current};
use canoa_office::relation::{conversation_relation, document_relation};
use windows::Win32::Foundation::{DISP_E_MEMBERNOTFOUND, DISP_E_UNKNOWNNAME, E_NOTIMPL};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, DISPATCH_FLAGS, DISPPARAMS, EXCEPINFO, IDispatch,
    IDispatch_Impl, ITypeInfo,
};
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, GUID, PCWSTR, Result, implement};

enum Value {
    Text(&'static str),
    Number(i32),
    Object(IDispatch),
}

/// An object with named properties; `Item` is also a method that answers its `Object` whatever the argument.
#[implement(IDispatch)]
struct Fake {
    names: Vec<&'static str>,
    values: BTreeMap<&'static str, Value>,
}

fn fake(values: Vec<(&'static str, Value)>) -> IDispatch {
    Fake {
        names: values.iter().map(|(name, _)| *name).collect(),
        values: values.into_iter().collect(),
    }
    .into()
}

impl IDispatch_Impl for Fake_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> {
        Ok(0)
    }
    fn GetTypeInfo(&self, _: u32, _: u32) -> Result<ITypeInfo> {
        Err(E_NOTIMPL.into())
    }
    fn GetIDsOfNames(
        &self,
        _: *const GUID,
        names: *const PCWSTR,
        _: u32,
        _: u32,
        ids: *mut i32,
    ) -> Result<()> {
        // SAFETY: the caller passes one NUL-terminated name and room for one id.
        let name = unsafe { (*names).to_string().unwrap() };
        match self.names.iter().position(|known| *known == name) {
            Some(index) => {
                unsafe { ids.write(index as i32 + 1) };
                Ok(())
            }
            None => Err(DISP_E_UNKNOWNNAME.into()),
        }
    }
    fn Invoke(
        &self,
        id: i32,
        _: *const GUID,
        _: u32,
        _: DISPATCH_FLAGS,
        _: *const DISPPARAMS,
        result: *mut VARIANT,
        _: *mut EXCEPINFO,
        _: *mut u32,
    ) -> Result<()> {
        let Some(name) = self.names.get((id - 1) as usize) else {
            return Err(DISP_E_MEMBERNOTFOUND.into());
        };
        let value = match &self.values[name] {
            Value::Text(text) => VARIANT::from(BSTR::from(*text)),
            Value::Number(number) => VARIANT::from(*number),
            Value::Object(object) => VARIANT::from(object.clone()),
        };
        // SAFETY: a result slot the caller provided.
        unsafe { result.write(value) };
        Ok(())
    }
}

fn init() {
    // SAFETY: first COM call on this thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };
}

#[test]
fn the_applications_are_told_apart_by_their_program() {
    assert_eq!(client_of_exe("WINWORD.EXE"), "word");
    assert_eq!(client_of_exe("excel.exe"), "excel");
    assert_eq!(client_of_exe("POWERPNT.EXE"), "powerpoint");
    assert_eq!(client_of_exe("OUTLOOK.EXE"), "outlook");
    assert_eq!(client_of_exe("notepad.exe"), "office");
}

#[test]
fn word_excel_and_powerpoint_relate_the_chat_to_the_active_document() {
    init();
    for (client, property) in [
        ("word", "ActiveDocument"),
        ("excel", "ActiveWorkbook"),
        ("powerpoint", "ActivePresentation"),
    ] {
        let document = fake(vec![("FullName", Value::Text(r"D:\Papers\Plan.docx"))]);
        let application = fake(vec![(property, Value::Object(document))]);
        assert_eq!(
            current(client, Some(&application), None),
            Context {
                client,
                relation: document_relation(client, r"D:\Papers\Plan.docx")
            }
        );
    }
}

#[test]
fn a_document_that_cannot_be_read_leaves_the_chat_with_no_relation() {
    init();
    // No application, an application with no active document, and a document with no name (not saved yet).
    assert_eq!(current("word", None, None).relation, None);
    let application = fake(vec![("Name", Value::Text("Word"))]);
    assert_eq!(current("word", Some(&application), None).relation, None);
    let unsaved = fake(vec![("FullName", Value::Text(""))]);
    let application = fake(vec![("ActiveDocument", Value::Object(unsaved))]);
    assert_eq!(current("word", Some(&application), None).relation, None);
    // An application the add-in has no relation for.
    assert_eq!(current("office", Some(&application), None).relation, None);
}

#[test]
fn outlook_relates_the_chat_to_the_conversation_of_the_open_or_selected_message() {
    init();
    let message = || {
        fake(vec![
            ("ConversationID", Value::Text("AAQkADk3")),
            ("Subject", Value::Text("Re: Q3   plan")),
        ])
    };
    let expected = conversation_relation("AAQkADk3", "Re: Q3 plan");
    assert!(expected.is_some());

    // A message window (an Inspector) has the message as `CurrentItem`.
    let inspector = fake(vec![("CurrentItem", Value::Object(message()))]);
    assert_eq!(
        current("outlook", None, Some(&inspector)).relation,
        expected
    );

    // The main window (an Explorer) has a selection.
    let selection = fake(vec![
        ("Count", Value::Number(2)),
        ("Item", Value::Object(message())),
    ]);
    let explorer = fake(vec![("Selection", Value::Object(selection))]);
    assert_eq!(current("outlook", None, Some(&explorer)).relation, expected);

    // Nothing selected, or no window: no relation, and no error.
    let empty = fake(vec![
        ("Count", Value::Number(0)),
        ("Item", Value::Object(message())),
    ]);
    let explorer = fake(vec![("Selection", Value::Object(empty))]);
    assert_eq!(current("outlook", None, Some(&explorer)).relation, None);
    assert_eq!(current("outlook", None, None).relation, None);

    // A new message has no conversation yet.
    let draft = fake(vec![
        ("ConversationID", Value::Text("")),
        ("Subject", Value::Text("hello")),
    ]);
    let inspector = fake(vec![("CurrentItem", Value::Object(draft))]);
    assert_eq!(current("outlook", None, Some(&inspector)).relation, None);
}
