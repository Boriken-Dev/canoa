//! The macro that makes a [`Driver`](crate::Driver) a provider of the kind.

/// Declares a provider of the [`Transport`](crate::Transport) kind that serves a [`Driver`](crate::Driver).
///
/// ```text
/// coqui_transport_guatiao::transport_provider! {
///     /// My transport, as a provider.
///     pub struct MyProvider for MyDriver, id = "my_transport";
/// }
/// ```
///
/// The struct holds the driver's settings. Its configuration is those settings, so a host reads their schema
/// from the offer and has every configuration checked against it before an instance is built; its `dial`,
/// `listen`, `describe` and `non_defaults` are [`serve`](crate::serve)'s over the driver, and the non-default
/// list is generated from the settings' description.
///
/// Invoke it where the driver's types live, or in the crate that registers it: the provider type is local to
/// that crate, so neither the transport's crate nor `coqui-transport` names guatiao. A `cdylib` that offers the
/// provider to hosts in any language calls `guatiao::providers!` on it; a host that links it calls
/// `guatiao::local_providers!`.
#[macro_export]
macro_rules! transport_provider {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident for $driver:ty, id = $id:literal;
    ) => {
        $(#[$meta])*
        #[derive(::core::fmt::Debug, $crate::guatiao::Provider)]
        #[provider(
            $crate::Transport,
            id = $id,
            config = $crate::settings::Settings<$driver>,
            new = $name::unconfigured,
            crate = $crate::guatiao
        )]
        $vis struct $name {
            settings: <$driver as $crate::Driver>::Settings,
        }

        impl $name {
            /// A provider configured as `settings` say.
            #[must_use]
            $vis fn new(settings: <$driver as $crate::Driver>::Settings) -> Self {
                Self { settings }
            }

            fn unconfigured() -> Self {
                Self::new(::core::default::Default::default())
            }
        }

        impl ::core::convert::From<$crate::settings::Settings<$driver>> for $name {
            fn from(settings: $crate::settings::Settings<$driver>) -> Self {
                Self::new(settings.0)
            }
        }

        impl $crate::Transport for $name {
            fn describe(
                &self,
            ) -> ::core::result::Result<$crate::TransportInfo, $crate::guatiao::library::ProviderError> {
                ::core::result::Result::Ok($crate::serve::describe::<$driver>(&self.settings))
            }

            fn dial(
                &self,
                request: $crate::DialRequest,
                sink: $crate::guatiao::library::Object<dyn $crate::ChannelSink>,
            ) -> ::core::result::Result<
                $crate::guatiao::library::Object<dyn $crate::Channel>,
                $crate::guatiao::library::ProviderError,
            > {
                $crate::serve::dial::<$driver>(&self.settings, &request, sink)
            }

            fn listen(
                &self,
                request: $crate::ListenRequest,
                acceptor: $crate::guatiao::library::Object<dyn $crate::TransportAcceptor>,
            ) -> ::core::result::Result<
                $crate::guatiao::library::Object<dyn $crate::TransportListener>,
                $crate::guatiao::library::ProviderError,
            > {
                $crate::serve::listen::<$driver>(&self.settings, &request, acceptor)
            }

            fn non_defaults(
                &self,
            ) -> ::core::result::Result<
                $crate::__Vec<$crate::NonDefault>,
                $crate::guatiao::library::ProviderError,
            > {
                ::core::result::Result::Ok($crate::serve::non_defaults::<$driver>(&self.settings))
            }
        }
    };
}
