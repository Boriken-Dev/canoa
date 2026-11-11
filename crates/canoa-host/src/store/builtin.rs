//! The stores built into this binary, registered like the built-in harness providers, and the macro that makes a provider of
//! a [`Recipe`](super::engine::Recipe).

/// A `program_store` provider `$name`, id `$id`, over the recipe `$recipe` (a type with `Default`).
macro_rules! store_provider {
    ($(#[$meta:meta])* $name:ident, $recipe:ty, $id:literal) => {
        $(#[$meta])*
        #[derive(::coqui_acp_api::guatiao::Provider, Debug)]
        #[provider(
            canoa::store::ProgramStore,
            id = $id,
            config = canoa::store::StoreSettings,
            new = $name::unconfigured,
            crate = ::coqui_acp_api::guatiao
        )]
        pub struct $name {
            engine: $crate::store::engine::Engine<$recipe>,
        }

        impl From<canoa::store::StoreSettings> for $name {
            fn from(settings: canoa::store::StoreSettings) -> Self {
                Self {
                    engine: $crate::store::engine::Engine::new(<$recipe>::default(), settings),
                }
            }
        }

        impl $name {
            fn unconfigured() -> Self {
                Self::from(canoa::store::StoreSettings::default())
            }
        }

        impl canoa::store::ProgramStore for $name {
            fn describe(&self) -> Result<canoa::store::StoreInfo, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.describe()
            }

            fn program(
                &self,
                query: canoa::store::ProgramQuery,
                spawner: ::coqui_acp_api::guatiao::library::Object<dyn ::coqui_acp_api::ProcessSpawner>,
                tools: ::coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallTools>,
            ) -> Result<canoa::store::ProgramReport, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.program(&query, &spawner, &tools)
            }

            fn latest(
                &self,
                query: canoa::store::LatestQuery,
                tools: ::coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallTools>,
            ) -> Result<canoa::store::LatestReport, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.latest(&query, &tools)
            }

            fn plan(
                &self,
                request: canoa::store::PlanRequest,
                tools: ::coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallTools>,
            ) -> Result<canoa::store::InstallPlan, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.plan(&request, &tools)
            }

            fn apply(
                &self,
                request: canoa::store::ApplyRequest,
                spawner: ::coqui_acp_api::guatiao::library::Object<dyn ::coqui_acp_api::ProcessSpawner>,
                tools: ::coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallTools>,
                prompter: ::coqui_acp_api::guatiao::library::Object<dyn canoa::store::InstallPrompter>,
            ) -> Result<canoa::store::ApplyOutcome, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.apply(&request, &spawner, &tools, &prompter)
            }
        }
    };
}

pub(crate) use store_provider;

store_provider!(
    /// The echo program's store, one file.
    EchoStore, super::echo::EchoRecipe, "echo_store"
);

store_provider!(
    /// The echo program's store, an archive with an extra step.
    EchoArchiveStore, super::echo::EchoArchiveRecipe, "echo_archive_store"
);
