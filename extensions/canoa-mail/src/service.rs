//! The two providers this library offers: the `mail` session service (its tools, its control methods) and the `mail` section of
//! Settings.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use canoa::mail_backend::MailError;
use canoa::session_service::{
    Context, ControlCall, ControlReply, Outcome, Refusal, Resume, ServiceHost, ServiceInfo,
    SessionService, SkillFile, ToolCall,
};
use canoa::settings_section::{SectionInfo, SettingsSection};
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{Map as GuatiaoMap, Provider, Value as GuatiaoValue};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::accounts::{self, Failed, NewAccount};
use crate::config::{MailConfig, SERVICE};
use crate::mailboxes::{BackendCatalog, Mailboxes, Opened, open_account};
use crate::tools::{self, MailTools, SERVER_NAME, SKILL_NAME};

/// Changes to the service's table are made one at a time, so two accounts added together do not undo one another.
static WRITES: Mutex<()> = Mutex::new(());

/// The mail service: the person's own mailboxes, to an agent's sessions.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(SessionService, id = "mail", crate = ::coqui_acp_api::guatiao)]
pub struct MailService;

/// The mailboxes opened so far, by host: registries are made again as often as the host loads them, and a mailbox that holds a
/// token or a connection should outlive them.
fn opened() -> &'static Arc<Opened> {
    static OPENED: OnceLock<Arc<Opened>> = OnceLock::new();
    OPENED.get_or_init(Arc::default)
}

fn host_of(context: &Context) -> Result<Arc<dyn ServiceHost>, ProviderError> {
    canoa::session_service::host(context)
        .ok_or_else(|| ProviderError::from(MailError::Bad("this host is gone".to_owned())))
}

fn config_of(host: &dyn ServiceHost) -> Result<MailConfig, String> {
    MailConfig::parse(&host.config(SERVICE)?)
}

impl MailService {
    fn boxes(&self, context: &Context) -> Result<Mailboxes, String> {
        let host = host_of(context).map_err(|error| error.to_string())?;
        let config = config_of(&*host)?;
        Ok(Mailboxes::from_config(
            &config,
            host,
            &context.host,
            Arc::clone(opened()),
        ))
    }
}

impl SessionService for MailService {
    fn describe(&self) -> Result<ServiceInfo, ProviderError> {
        let mut info = ServiceInfo::new(SERVICE, "Mail");
        info.server = Some(SERVER_NAME.to_owned());
        info.tools = MailTools::infos();
        info.skill = Some(SKILL_NAME.to_owned());
        info.skill_files = tools::skill_files()
            .into_iter()
            .map(|(path, text)| SkillFile::new(path, text))
            .collect();
        Ok(info)
    }

    fn offered(&self, context: Context) -> Result<bool, ProviderError> {
        let Ok(host) = host_of(&context) else {
            return Ok(false);
        };
        Ok(config_of(&*host).is_ok_and(|config| config.offers_tools()))
    }

    fn call(&self, context: Context, call: ToolCall) -> Result<Outcome, ProviderError> {
        let args: Value = serde_json::from_str(&call.arguments).unwrap_or(Value::Null);
        Ok(match self.boxes(&context) {
            Ok(boxes) => MailTools::new(boxes).call(&call.tool, &args),
            Err(why) => failed_call(&why),
        })
    }

    fn resume(&self, context: Context, resume: Resume) -> Result<Outcome, ProviderError> {
        Ok(match self.boxes(&context) {
            Ok(boxes) => MailTools::new(boxes).resume(&resume.tool, &resume.state, resume.answer),
            Err(why) => failed_call(&why),
        })
    }

    fn control(&self, context: Context, call: ControlCall) -> Result<ControlReply, ProviderError> {
        let params: Value = if call.params.trim().is_empty() {
            Value::Null
        } else {
            match serde_json::from_str(&call.params) {
                Ok(params) => params,
                Err(_) => {
                    return Ok(ControlReply::refused(
                        Refusal::BadParams,
                        "the parameters are not JSON",
                    ));
                }
            }
        };
        let host = host_of(&context)?;
        Ok(match self.run(&host, &context, &call.method, &params) {
            Ok(result) => ControlReply::ok(result.to_string()),
            Err((why, message)) => ControlReply::refused(why, message),
        })
    }
}

type Refused = (Refusal, String);

fn failed_call(why: &str) -> Outcome {
    let text = tools::clean(why, 600, false);
    Outcome::done(json!({"content": [{"type": "text", "text": text}], "isError": true}).to_string())
}

fn refusal(failed: &Failed) -> Refused {
    (
        if failed.usage {
            Refusal::BadParams
        } else {
            Refusal::Refused
        },
        failed.why.clone(),
    )
}

fn text<'a>(params: &'a Value, name: &str) -> Result<&'a str, Refused> {
    params
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| (Refusal::BadParams, format!("{name} is a string")))
}

impl MailService {
    /// Answers one of the four methods: `accounts`, `add`, `test`, `remove`.
    fn run(
        &self,
        host: &Arc<dyn ServiceHost>,
        context: &Context,
        method: &str,
        params: &Value,
    ) -> Result<Value, Refused> {
        let _one = WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let config = config_of(&**host).map_err(|why| (Refusal::Refused, why))?;
        match method {
            "accounts" => {
                let catalog = BackendCatalog::of(&**host);
                let rows: Vec<Value> = accounts::rows(&config, &catalog, &**host)
                    .iter()
                    .map(accounts::Row::json)
                    .collect();
                Ok(json!({
                    "accounts": rows,
                    "backends": accounts::backends_json(&catalog),
                    "tools": config.tools != Some(false),
                }))
            }
            "add" => {
                #[derive(Deserialize)]
                struct Params {
                    #[serde(default)]
                    name: Option<String>,
                    #[serde(default)]
                    backend: Option<String>,
                    #[serde(default)]
                    id: Option<String>,
                    #[serde(default)]
                    password_env: Option<String>,
                    #[serde(default)]
                    timeout_secs: Option<u64>,
                    #[serde(default)]
                    settings: Option<Map<String, Value>>,
                    #[serde(flatten)]
                    extra: Map<String, Value>,
                }
                let parsed: Params = serde_json::from_value(params.clone()).map_err(|_| {
                    (
                        Refusal::BadParams,
                        "name and backend are strings".to_owned(),
                    )
                })?;
                let name = parsed
                    .name
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| (Refusal::BadParams, "name is a string".to_owned()))?;
                let backend = parsed
                    .backend
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| (Refusal::BadParams, "backend is a string".to_owned()))?;
                let mut settings = parsed.settings.unwrap_or_default();
                for (k, v) in parsed.extra {
                    settings.entry(k).or_insert(v);
                }
                let new = NewAccount {
                    backend,
                    id: parsed.id,
                    password_env: parsed.password_env,
                    timeout_secs: parsed.timeout_secs,
                    settings,
                };
                let catalog = BackendCatalog::of(&**host);
                accounts::add(&**host, &config, &catalog, &name, &new)
                    .map_err(|failed| refusal(&failed))?;
                opened().forget(&context.host);
                let config = config_of(&**host).map_err(|why| (Refusal::Refused, why))?;
                let row = accounts::rows(&config, &catalog, &**host)
                    .into_iter()
                    .find(|row| row.name == name)
                    .map_or(Value::Null, |row| row.json());
                Ok(json!({"account": row}))
            }
            "test" => {
                let name = text(params, "name")?;
                let account = config.accounts.get(name).ok_or_else(|| {
                    (
                        Refusal::NotFound,
                        format!("no mail account is called {name}"),
                    )
                })?;
                let catalog = BackendCatalog::of(&**host);
                let outcome = open_account(&catalog, account, &**host)
                    .and_then(|backend| backend.folders().map_err(MailError::from));
                Ok(match outcome {
                    Ok(folders) => json!({"ok": true, "folders": folders.len()}),
                    Err(error) => json!({
                        "ok": false,
                        "problem": tools::clean(&error.to_string(), 600, false),
                    }),
                })
            }
            "remove" => {
                accounts::remove(&**host, &config, text(params, "name")?)
                    .map_err(|failed| refusal(&failed))?;
                opened().forget(&context.host);
                Ok(json!({"removed": true}))
            }
            _ => Err((
                Refusal::NotFound,
                "this service has no such method".to_owned(),
            )),
        }
    }
}

/// The page of Settings the mail accounts are on. Its accounts are the service's own methods, so it asks for no form.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(SettingsSection, id = "mail", crate = ::coqui_acp_api::guatiao)]
pub struct MailSection;

impl SettingsSection for MailSection {
    fn describe(&self) -> Result<SectionInfo, ProviderError> {
        let mut info = SectionInfo::new("mail", "Mail");
        info.group = Some("Agents".to_owned());
        info.icon = Some("mail".to_owned());
        info.keywords = Some(
            "email exchange outlook ews mailbox inbox messages password account add remove test sign in"
                .to_owned(),
        );
        Ok(info)
    }

    fn read(&self) -> Result<GuatiaoValue, ProviderError> {
        Ok(GuatiaoValue::from(GuatiaoMap::new()))
    }

    fn write(&self, _values: GuatiaoValue) -> Result<(), ProviderError> {
        Ok(())
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-mail",
    version = env!("CARGO_PKG_VERSION"),
    providers = [MailService, MailSection]
);
