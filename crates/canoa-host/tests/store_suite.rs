//! The program_store contract suite, run on the good example stores, and on stores that each break one rule and must fail
//! the check that names it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use canoa::store::{
    ApplyOutcome, ApplyRequest, Check, FetchRequest, InstallPlan, InstallPrompter, InstallTools,
    LatestQuery, LatestReport, PlanRequest, ProgramQuery, ProgramReport, ProgramStore, StoreInfo,
    StoreSettings,
};
use canoa_host::store::contract::{Failure, run};
use canoa_host::store::echo::{EchoArchiveRecipe, EchoFixture, EchoRecipe};
use canoa_host::store::engine::Engine;
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};

fn standin() -> PathBuf {
    let exe = std::env::current_exe().expect("test path");
    let dir = exe.parent().expect("deps dir").parent().expect("debug dir");
    let name = if cfg!(windows) {
        "version_standin.exe"
    } else {
        "version_standin"
    };
    let path = dir.join("examples").join(name);
    assert!(
        path.is_file(),
        "build the examples first: cargo build -p canoa-host --examples ({})",
        path.display()
    );
    path
}

fn names(failures: &[Failure]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = failures.iter().map(|f| f.check).collect();
    out.dedup();
    out
}

#[test]
fn the_one_file_store_passes_every_check() {
    let fixture = EchoFixture {
        standin: standin(),
        archive: false,
    };
    let store: Arc<dyn ProgramStore> =
        Arc::new(Good(Engine::new(EchoRecipe, StoreSettings::default())));
    let failures = run(store, &fixture);
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn the_archive_store_with_an_extra_step_passes_every_check() {
    let fixture = EchoFixture {
        standin: standin(),
        archive: true,
    };
    let store: Arc<dyn ProgramStore> = Arc::new(GoodArchive(Engine::new(
        EchoArchiveRecipe,
        StoreSettings::default(),
    )));
    let failures = run(store, &fixture);
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Forwards a store's methods to an engine, and lets a defect change `apply`.
macro_rules! forwarding {
    ($name:ident, $recipe:ty) => {
        struct $name(Engine<$recipe>);

        impl ProgramStore for $name {
            fn describe(&self) -> Result<StoreInfo, ProviderError> {
                self.0.describe()
            }
            fn program(
                &self,
                q: ProgramQuery,
                s: Object<dyn ProcessSpawner>,
                t: Object<dyn InstallTools>,
            ) -> Result<ProgramReport, ProviderError> {
                self.0.program(&q, &s, &t)
            }
            fn latest(
                &self,
                q: LatestQuery,
                t: Object<dyn InstallTools>,
            ) -> Result<LatestReport, ProviderError> {
                self.0.latest(&q, &t)
            }
            fn plan(
                &self,
                r: PlanRequest,
                t: Object<dyn InstallTools>,
            ) -> Result<InstallPlan, ProviderError> {
                self.0.plan(&r, &t)
            }
            fn apply(
                &self,
                r: ApplyRequest,
                s: Object<dyn ProcessSpawner>,
                t: Object<dyn InstallTools>,
                p: Object<dyn InstallPrompter>,
            ) -> Result<ApplyOutcome, ProviderError> {
                self.0.apply(&r, &s, &t, &p)
            }
        }
    };
}

forwarding!(Good, EchoRecipe);
forwarding!(GoodArchive, EchoArchiveRecipe);

/// The one rule a bad store breaks.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "the registry defect is exercised only on Windows")
)]
#[derive(Clone, Copy)]
enum Defect {
    AppendsToProfile,
    WritesOutsideStaging,
    StartsAScript,
    SwallowsAMismatch,
    IgnoresCancel,
    WritesARegistryValue,
}

struct Bad {
    engine: Engine<EchoRecipe>,
    defect: Defect,
}

fn home_of(request: &ApplyRequest) -> PathBuf {
    let env = request.environment.expose();
    PathBuf::from(
        env.get("HOME")
            .or_else(|| env.get("USERPROFILE"))
            .expect("a home"),
    )
}

impl ProgramStore for Bad {
    fn describe(&self) -> Result<StoreInfo, ProviderError> {
        self.engine.describe()
    }

    fn program(
        &self,
        q: ProgramQuery,
        s: Object<dyn ProcessSpawner>,
        t: Object<dyn InstallTools>,
    ) -> Result<ProgramReport, ProviderError> {
        self.engine.program(&q, &s, &t)
    }

    fn latest(
        &self,
        q: LatestQuery,
        t: Object<dyn InstallTools>,
    ) -> Result<LatestReport, ProviderError> {
        self.engine.latest(&q, &t)
    }

    fn plan(
        &self,
        r: PlanRequest,
        t: Object<dyn InstallTools>,
    ) -> Result<InstallPlan, ProviderError> {
        self.engine.plan(&r, &t)
    }

    fn apply(
        &self,
        request: ApplyRequest,
        spawner: Object<dyn ProcessSpawner>,
        tools: Object<dyn InstallTools>,
        prompter: Object<dyn InstallPrompter>,
    ) -> Result<ApplyOutcome, ProviderError> {
        match self.defect {
            Defect::AppendsToProfile => {
                let rc = home_of(&request).join(".bashrc");
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(rc)
                    .expect("rc");
                writeln!(file, "export PATH=\"$HOME/.bad/bin:$PATH\"").expect("write");
            }
            Defect::WritesOutsideStaging => {
                std::fs::write(home_of(&request).join("elsewhere.txt"), b"x").expect("write");
            }
            Defect::StartsAScript => {
                let script = Path::new(&request.staging).join("run.cmd");
                let spec = coqui_acp_api::SpawnSpec::new(script.to_string_lossy())
                    .cwd(request.staging.clone());
                let _ = spawner.spawn(spec);
            }
            Defect::IgnoresCancel => {
                std::thread::sleep(std::time::Duration::from_secs(3));
                return Ok(ApplyOutcome::new(request.plan.version.clone(), None));
            }
            Defect::WritesARegistryValue => write_registry_value(),
            Defect::SwallowsAMismatch => {
                let result = self.engine.apply(&request, &spawner, &tools, &prompter);
                if let Err(error) = &result
                    && error.message().contains("checksum mismatch")
                {
                    let download = &request.plan.downloads[0];
                    let mut fetch = FetchRequest::new(
                        download.url.clone(),
                        "again",
                        Check::Unverified {
                            why: "the digest did not match".into(),
                        },
                        1 << 30,
                    );
                    fetch.executable = false;
                    let _ = tools.fetch(fetch);
                    return Ok(ApplyOutcome::new(request.plan.version.clone(), None));
                }
                return result;
            }
        }
        self.engine.apply(&request, &spawner, &tools, &prompter)
    }
}

#[cfg(windows)]
fn write_registry_value() {
    use windows_registry::CURRENT_USER;
    let key = CURRENT_USER.create("Environment").expect("the key");
    let nonce = std::process::id();
    key.set_string(format!("COQUI_STORE_CHECK_{nonce}"), "planted")
        .expect("a value");
}

#[cfg(not(windows))]
fn write_registry_value() {}

fn bad(defect: Defect) -> Vec<Failure> {
    let fixture = EchoFixture {
        standin: standin(),
        archive: false,
    };
    let store: Arc<dyn ProgramStore> = Arc::new(Bad {
        engine: Engine::new(EchoRecipe, StoreSettings::default()),
        defect,
    });
    run(store, &fixture)
}

#[test]
fn a_store_that_appends_to_a_profile_fails_apply_profile_file() {
    let failures = bad(Defect::AppendsToProfile);
    assert!(
        names(&failures).contains(&"apply.profile_file"),
        "{failures:#?}"
    );
}

#[test]
fn a_store_that_writes_outside_its_folder_fails_apply_outside_staging() {
    let failures = bad(Defect::WritesOutsideStaging);
    assert!(
        names(&failures).contains(&"apply.outside_staging"),
        "{failures:#?}"
    );
}

#[test]
fn a_store_that_starts_a_script_fails_apply_spawns() {
    let failures = bad(Defect::StartsAScript);
    assert!(names(&failures).contains(&"apply.spawns"), "{failures:#?}");
}

#[test]
fn a_store_that_swallows_a_mismatch_fails_apply_mismatch() {
    let failures = bad(Defect::SwallowsAMismatch);
    let found = names(&failures);
    assert!(found.contains(&"apply.mismatch"), "{failures:#?}");
    assert!(found.contains(&"apply.mismatch_no_retry"), "{failures:#?}");
}

#[test]
fn a_store_that_ignores_cancel_fails_apply_cancel() {
    let failures = bad(Defect::IgnoresCancel);
    assert!(names(&failures).contains(&"apply.cancel"), "{failures:#?}");
}

#[cfg(windows)]
#[test]
fn a_store_that_writes_the_registry_fails_apply_registry_and_the_value_is_put_back() {
    let failures = bad(Defect::WritesARegistryValue);
    assert!(
        names(&failures).contains(&"apply.registry"),
        "{failures:#?}"
    );
    use windows_registry::CURRENT_USER;
    let key = CURRENT_USER.open("Environment").expect("the key");
    let leftover = key
        .values()
        .expect("values")
        .any(|(name, _)| name.starts_with("COQUI_STORE_CHECK_"));
    assert!(
        !leftover,
        "the suite left a planted value in HKCU\\Environment"
    );
}
