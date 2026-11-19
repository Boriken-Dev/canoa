/// Makes a `program_store` provider `$name`, with the id `$id`, of the recipe `$recipe` (a type with `Default`), run by
/// [`Engine`](crate::store::engine::Engine). A crate that packages one agent offers it from its library beside the agent's
/// other providers; the crate using it names `coqui-acp-api` and `canoa` as dependencies.
#[macro_export]
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

        impl From<$crate::store::StoreSettings> for $name {
            fn from(settings: $crate::store::StoreSettings) -> Self {
                Self {
                    engine: $crate::store::engine::Engine::new(<$recipe>::default(), settings),
                }
            }
        }

        impl $name {
            fn unconfigured() -> Self {
                Self::from($crate::store::StoreSettings::default())
            }
        }

        impl $crate::store::ProgramStore for $name {
            fn describe(&self) -> Result<$crate::store::StoreInfo, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.describe()
            }

            fn program(
                &self,
                query: $crate::store::ProgramQuery,
                spawner: ::coqui_acp_api::guatiao::library::Object<dyn ::coqui_acp_api::ProcessSpawner>,
                tools: ::coqui_acp_api::guatiao::library::Object<dyn $crate::store::InstallTools>,
            ) -> Result<$crate::store::ProgramReport, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.program(&query, &spawner, &tools)
            }

            fn latest(
                &self,
                query: $crate::store::LatestQuery,
                tools: ::coqui_acp_api::guatiao::library::Object<dyn $crate::store::InstallTools>,
            ) -> Result<$crate::store::LatestReport, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.latest(&query, &tools)
            }

            fn plan(
                &self,
                request: $crate::store::PlanRequest,
                tools: ::coqui_acp_api::guatiao::library::Object<dyn $crate::store::InstallTools>,
            ) -> Result<$crate::store::InstallPlan, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.plan(&request, &tools)
            }

            fn apply(
                &self,
                request: $crate::store::ApplyRequest,
                spawner: ::coqui_acp_api::guatiao::library::Object<dyn ::coqui_acp_api::ProcessSpawner>,
                tools: ::coqui_acp_api::guatiao::library::Object<dyn $crate::store::InstallTools>,
                prompter: ::coqui_acp_api::guatiao::library::Object<dyn $crate::store::InstallPrompter>,
            ) -> Result<$crate::store::ApplyOutcome, ::coqui_acp_api::guatiao::library::ProviderError> {
                self.engine.apply(&request, &spawner, &tools, &prompter)
            }
        }
    };
}
