//! Late-bound calls on Office's objects: a property by name, a method with arguments. The object model is reached only through
//! `IDispatch`, so nothing here needs Office's type libraries, and it runs on any object that answers `IDispatch`.

#![allow(unsafe_code)]

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::System::Com::{
    DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPPARAMS, EXCEPINFO, IDispatch,
};
use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VariantClear};
use windows::core::{GUID, HSTRING, PCWSTR, Result};

fn id_of(object: &IDispatch, name: &str) -> Result<i32> {
    let wide = HSTRING::from(name);
    let names = [PCWSTR(wide.as_ptr())];
    let mut id = 0_i32;
    // SAFETY: one NUL-terminated name that outlives the call, and one id to write.
    unsafe { object.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0, &mut id)? };
    Ok(id)
}

fn invoke(
    object: &IDispatch,
    name: &str,
    flags: DISPATCH_FLAGS,
    arguments: &mut [VARIANT],
) -> Result<VARIANT> {
    let id = id_of(object, name)?;
    // Arguments go in reverse order, as `IDispatch::Invoke` takes them.
    arguments.reverse();
    let parameters = DISPPARAMS {
        rgvarg: if arguments.is_empty() {
            core::ptr::null_mut()
        } else {
            arguments.as_mut_ptr()
        },
        rgdispidNamedArgs: core::ptr::null_mut(),
        cArgs: u32::try_from(arguments.len()).unwrap_or(0),
        cNamedArgs: 0,
    };
    let mut result = VARIANT::default();
    let mut exception = EXCEPINFO::default();
    // SAFETY: `parameters` points to `arguments`, which outlives the call; the result and exception are live locals.
    let called = unsafe {
        object.Invoke(
            id,
            &GUID::zeroed(),
            0,
            flags,
            &parameters,
            Some(&mut result),
            Some(&mut exception),
            None,
        )
    };
    arguments.reverse();
    called?;
    Ok(result)
}

/// The property `name` of `object`.
///
/// # Errors
///
/// When the object has no such property or the call fails.
pub fn get(object: &IDispatch, name: &str) -> Result<VARIANT> {
    invoke(object, name, DISPATCH_PROPERTYGET, &mut [])
}

/// The property `name`, as an object.
///
/// # Errors
///
/// When the property is not there or is not an object.
pub fn get_object(object: &IDispatch, name: &str) -> Result<IDispatch> {
    IDispatch::try_from(&get(object, name)?)
}

/// The property `name`, as text.
///
/// # Errors
///
/// When the property is not there or is not text.
pub fn get_string(object: &IDispatch, name: &str) -> Result<String> {
    let mut value = get(object, name)?;
    // SAFETY: the tag says which member of the union is live; the text is copied before the variant is cleared.
    let text = unsafe {
        let inner = &value.Anonymous.Anonymous;
        (inner.vt == VT_BSTR).then(|| inner.Anonymous.bstrVal.to_string())
    };
    // SAFETY: a variant this function owns, cleared once.
    unsafe {
        let _ = VariantClear(&mut value);
    }
    text.ok_or_else(|| E_INVALIDARG.into())
}

/// The method `name` of `object` called with one whole-number argument, as an object.
///
/// # Errors
///
/// When the method is not there, fails, or returns something that is not an object.
pub fn call_with_number(object: &IDispatch, name: &str, argument: i32) -> Result<IDispatch> {
    let mut arguments = [VARIANT::from(argument)];
    let result = invoke(
        object,
        name,
        DISPATCH_METHOD | DISPATCH_PROPERTYGET,
        &mut arguments,
    )?;
    IDispatch::try_from(&result)
}

/// The property `name`, as a whole number.
///
/// # Errors
///
/// When the property is not there or is not a number.
pub fn get_number(object: &IDispatch, name: &str) -> Result<i32> {
    let mut value = get(object, name)?;
    let number = i32::try_from(&value);
    // SAFETY: a variant this function owns, cleared once.
    unsafe {
        let _ = VariantClear(&mut value);
    }
    number
}
