// Copyright (c) ZeroC, Inc.

use tower_lsp::{
    lsp_types::{ConfigurationItem, DidChangeConfigurationParams, Url},
    Client,
};

#[derive(Default, Debug)]
pub struct SliceConfig {
    pub references: Option<Vec<String>>,
    pub root_uri: Option<Url>,
}

impl SliceConfig {
    pub async fn try_update_from_params(
        &mut self,
        params: &DidChangeConfigurationParams,
    ) -> tower_lsp::jsonrpc::Result<()> {
        self.references = Self::parse_reference_directories(params);
        Ok(())
    }

    pub async fn try_update_from_client(
        &mut self,
        client: &Client,
    ) -> tower_lsp::jsonrpc::Result<()> {
        self.references = Self::fetch_reference_directories(client).await?;
        Ok(())
    }

    // Fetch reference directories from the backend.
    async fn fetch_reference_directories(
        client: &Client,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<String>>> {
        let params = vec![ConfigurationItem {
            scope_uri: None,
            section: Some("slice.referenceDirectories".to_string()),
        }];

        Ok(client
            .configuration(params)
            .await?
            .first()
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect::<Vec<_>>()
            }))
    }

    // Parse reference directories from configuration parameters.
    fn parse_reference_directories(params: &DidChangeConfigurationParams) -> Option<Vec<String>> {
        params
            .settings
            .get("slice")
            .and_then(|v| v.get("referenceDirectories"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect::<Vec<String>>()
            })
    }

    // Convert reference directory strings into URLs.
    fn try_get_reference_urls(&self) -> Vec<Url> {
        // If no root_uri is set, return `None`, since this won't be able to resolve the reference urls.
        let Some(root_uri) = &self.root_uri else {
            return vec![];
        };

        // If no references are set, default to using the root_uri.
        let Some(references) = &self.references else {
            return vec![root_uri.clone()];
        };

        // Convert the root_uri to a file path. If it fails, return `None`.
        let Ok(root_path) = root_uri.to_file_path() else {
            return vec![];
        };

        // Convert reference directories to URLs or use root_uri if none are present
        let mut result_urls = Vec::new();
        for reference in references {
            match Url::from_file_path(root_path.join(reference)) {
                Ok(full_path) => result_urls.push(full_path),
                Err(_) => return vec![],
            }
        }
        result_urls
    }

    // Resolve reference URIs to file paths to be used by the Slice compiler.
    pub fn resolve_reference_paths(&self) -> Vec<String> {
        let reference_urls = self.try_get_reference_urls();

        // If no reference directories are set, use the root_uri as the reference directory.
        if reference_urls.is_empty() {
            return self
                .root_uri
                .as_ref()
                .and_then(|url| url.to_file_path().ok())
                .map(|path| vec![path.display().to_string()])
                .unwrap_or_default();
        }

        // Convert reference URLs to file paths
        reference_urls
            .iter()
            .filter_map(|uri| {
                let path = uri.to_file_path().ok()?;
                if path.is_absolute() {
                    Some(path.display().to_string())
                } else {
                    self.root_uri
                        .as_ref()?
                        .to_file_path()
                        .ok()
                        .map(|root_path| root_path.join(&path).display().to_string())
                }
            })
            .collect()
    }
}
