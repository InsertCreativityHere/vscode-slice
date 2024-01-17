// Copyright (c) ZeroC, Inc.

use crate::{configuration_set::ConfigurationSet, slice_config::ServerConfig};
use tokio::sync::Mutex;
use tower_lsp::lsp_types::{DidChangeConfigurationParams, Url};

pub struct Session {
    /// This vector contains all of the configuration sets for the language server. Each element is a tuple containing
    /// `SliceConfig` and `CompilationState`. The `SliceConfig` is used to determine which configuration set to use when
    /// publishing diagnostics. The `CompilationState` is used to retrieve the diagnostics for a given file.
    pub configuration_sets: Mutex<Vec<ConfigurationSet>>,
    /// Configuration that affects the entire server.
    pub server_config: Mutex<ServerConfig>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            configuration_sets: Mutex::new(Vec::new()),
            server_config: Mutex::new(ServerConfig::default()),
        }
    }

    // Update the properties of the session from `InitializeParams`
    pub async fn update_from_initialize_params(
        &self,
        params: tower_lsp::lsp_types::InitializeParams,
    ) {
        let initialization_options = params.initialization_options;

        // This is the path to the built-in Slice files that are included with the extension. It should always
        // be present.
        let built_in_slice_path = initialization_options
            .as_ref()
            .and_then(|opts| opts.get("builtInSlicePath"))
            .and_then(|v| v.as_str().map(str::to_owned))
            .expect("builtInSlicePath not found in initialization options");

        // Use the root_uri if it exists temporarily as we cannot access configuration until
        // after initialization. Additionally, LSP may provide the windows path with escaping or a lowercase
        // drive letter. To fix this, we convert the path to a URL and then back to a path.
        let workspace_root_path = params
            .root_uri
            .and_then(|uri| uri.to_file_path().ok())
            .and_then(|path| Url::from_file_path(path).ok())
            .and_then(|uri| uri.to_file_path().ok())
            .map(|path| path.display().to_string())
            .expect("`root_uri` was not sent by the client, or was malformed");

        *self.server_config.lock().await = ServerConfig { workspace_root_path, built_in_slice_path };

        // Load any user configuration from the 'slice.configurations' option.
        let configuration_sets = initialization_options
            .as_ref()
            .and_then(|opts| opts.get("configuration"))
            .and_then(|v| v.as_array())
            .map(|arr| ConfigurationSet::parse_configuration_sets(arr))
            .unwrap_or_default();

        self.update_configurations(configuration_sets).await;
    }

    // Update the configuration sets from the `DidChangeConfigurationParams` notification.
    pub async fn update_configurations_from_params(&self, params: DidChangeConfigurationParams) {
        // Parse the configurations from the notification
        let configurations = params
            .settings
            .get("slice")
            .and_then(|v| v.get("configurations"))
            .and_then(|v| v.as_array())
            .map(|arr| ConfigurationSet::parse_configuration_sets(arr))
            .unwrap_or_default();

        // Update the configuration sets
        self.update_configurations(configurations).await;
    }

    // Update the configuration sets by replacing it with the new configurations. If there are no configuration sets
    // after updating, insert the default configuration set.
    async fn update_configurations(&self, mut configurations: Vec<ConfigurationSet>) {
        // Insert the default configuration set if needed
        if configurations.is_empty() {
            let default = ConfigurationSet::new();
            configurations.push(default);
        }

        let mut configuration_sets = self.configuration_sets.lock().await;
        *configuration_sets = configurations;
    }
}
