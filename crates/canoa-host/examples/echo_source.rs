//! A Guatiao source serving one library, `source_echo`, whose provider of the same id answers each message with its text
//! after a prefix taken from the source's settings: `{"prefix": text}`, or `{"prefix_file": path}` read from the working
//! directory. The source entries the host's tests serve without a Python. This file is a `cdylib` and nothing else.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

#[path = "shared/echo.rs"]
mod echo;

use std::sync::Arc;

use coqui_acp_provider::guatiao::library::kind::{LibraryParts, ProviderParts};
use coqui_acp_provider::guatiao::library::{
    Candidate, Host, KindTable, LocalLibrary, Object, ProviderError, Source,
};
use coqui_acp_provider::guatiao::value::convert::TryAsRef;
use coqui_acp_provider::guatiao::{self, Map, Status};
use coqui_acp_provider::{
    AgentProvider, AgentProviderVtable, AgentSession, ResumeRequest, StartRequest,
};
use echo::Echo;

/// The provider the library offers.
struct SourceEcho {
    prefix: String,
}

impl AgentProvider for SourceEcho {
    fn start(&self, request: StartRequest) -> Result<Object<dyn AgentSession>, ProviderError> {
        Ok(Echo::new("source-0".to_owned(), &self.prefix, &request.prompt).into_object())
    }

    fn resume(&self, _request: ResumeRequest) -> Result<Object<dyn AgentSession>, ProviderError> {
        Err(ProviderError::new(
            Status::GUATIAO_ERR_NOT_FOUND,
            "this provider keeps no session",
        ))
    }
}

static TABLE: AgentProviderVtable = AgentProviderVtable::of::<SourceEcho>();

/// The one library, and its prefix.
struct Shelf {
    prefix: String,
}

impl Source for Shelf {
    fn list(&self, _host: Host) -> Result<Vec<Candidate>, ProviderError> {
        Ok(vec![Candidate::new(
            "source_echo",
            "1.0.0",
            vec!["agent_provider".into()],
            self.prefix.as_str(),
        )])
    }

    fn open(&self, _host: Host, id: &str) -> Result<LocalLibrary<'static>, ProviderError> {
        // The provider the table's `ctx` names, kept with it.
        let provider = Arc::new(SourceEcho {
            prefix: self.prefix.clone(),
        });
        let parts = ProviderParts::builder("source_echo")
            .kinds(&["agent_provider"])
            .table(KindTable::new(
                "agent_provider",
                (&TABLE as *const AgentProviderVtable).cast(),
                size_of::<AgentProviderVtable>(),
            ))
            .ctx(Arc::as_ptr(&provider).cast_mut().cast())
            .keep(provider)
            .build();
        Ok(LibraryParts::new(id, "1.0.0", vec![parts]).into())
    }
}

fn refused(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why)
}

/// Reads `{"prefix": text}` or `{"prefix_file": path}`.
fn start(_host: Host, config: &Map) -> Result<Shelf, ProviderError> {
    let text = |key: &str| {
        config
            .get(key)
            .map(|value| {
                TryAsRef::<str>::try_as_ref(value).ok_or_else(|| refused("a setting is not text"))
            })
            .transpose()
    };
    let prefix = match (text("prefix")?, text("prefix_file")?) {
        (Some(prefix), None) => prefix.to_owned(),
        (None, Some(file)) => std::fs::read_to_string(file)
            .map_err(|_| refused("prefix_file could not be read"))?
            .trim()
            .to_owned(),
        _ => return Err(refused("give prefix or prefix_file")),
    };
    Ok(Shelf { prefix })
}

guatiao::source!(serves = "echoes", make = start);
