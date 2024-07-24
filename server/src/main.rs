// Copyright (c) ZeroC, Inc.

use crate::diagnostic_ext::{clear_diagnostics, process_diagnostics, publish_diagnostics_for_set};
use crate::hover::get_hover_message;
use crate::jump_definition::get_definition_span;
use crate::notifications::{ShowNotification, ShowNotificationParams};
use crate::session::Session;
use crate::slice_config::compute_slice_options;
use std::ops::DerefMut;
use std::{collections::HashMap, path::Path};
use tokio::sync::Mutex;
use tower_lsp::{lsp_types::*, Client, LanguageServer, LspService, Server};
use utils::{convert_slice_path_to_uri, span_to_range, url_to_sanitized_file_path};

mod configuration_set;
mod diagnostic_ext;
mod hover;
mod jump_definition;
mod notifications;
mod session;
mod slice_config;
mod utils;

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

struct Backend {
    client: Client,
    session: Mutex<Session>,
}

impl Backend {
    pub fn new(client: tower_lsp::Client) -> Self {
        let session = Mutex::new(Session::default());
        Self { client, session }
    }

    fn capabilities() -> ServerCapabilities {
        let definition_provider = Some(OneOf::Left(true));
        let hover_provider = Some(HoverProviderCapability::Simple(true));

        let text_document_sync = Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::FULL),
                save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                    include_text: Some(false),
                })),
                ..Default::default()
            },
        ));

        let workspace = Some(WorkspaceServerCapabilities {
            workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                supported: Some(true),
                change_notifications: Some(OneOf::Left(true)),
            }),
            ..Default::default()
        });

        ServerCapabilities {
            text_document_sync,
            workspace,
            definition_provider,
            hover_provider,
            ..Default::default()
        }
    }

    async fn handle_file_change(&self, file_uri: &Url) {
        let Some(file_path) = url_to_sanitized_file_path(file_uri) else {
            self.client.log_message(MessageType::WARNING, format!("unsupported file path: '{file_uri}'")).await;
            return;
        };

        let mut session_guard = self.session.lock().await;
        let Session { configuration_sets, server_config } = session_guard.deref_mut();

        let mut publish_map = HashMap::new();
        let mut diagnostics = Vec::new();

        // Process each configuration set that contains the changed file
        for set in configuration_sets.iter_mut().filter(|set| {
            compute_slice_options(server_config, &set.slice_config)
                .references
                .into_iter()
                .any(|f| {
                    let key_path = Path::new(&f);
                    key_path == file_path || file_path.starts_with(key_path)
                })
        }) {
            // `trigger_compilation` compiles the configuration set's files and returns any diagnostics.
            diagnostics.extend(set.trigger_compilation(server_config));

            // Update publish_map with files to be updated
            publish_map.extend(
                set.compilation_data
                    .files
                    .keys()
                    .filter_map(convert_slice_path_to_uri)
                    .map(|uri| (uri, vec![])),
            );
        }

        // If there are multiple diagnostics for the same span, that have the same message, deduplicate them
        diagnostics.dedup_by(|d1, d2| d1.span() == d2.span() && d1.message() == d2.message());

        // Group the diagnostics by file since diagnostics are published per file and diagnostic.span contains the file URL
        // Process diagnostics and update publish_map. Any diagnostics that do not have a span are returned for further processing.
        let spanless_diagnostics = process_diagnostics(diagnostics, &mut publish_map);
        for diagnostic in spanless_diagnostics {
            show_popup(
                &self.client,
                diagnostic.message(),
                notifications::MessageType::Error,
            )
            .await;
        }

        // Publish the diagnostics for each file
        self.client
            .log_message(
                MessageType::INFO,
                "Publishing diagnostics for all configuration sets.",
            )
            .await;

        for (uri, lsp_diagnostics) in publish_map {
            self.client
                .publish_diagnostics(uri, lsp_diagnostics, None)
                .await;
        }
    } // TODO

    /// Triggers and compilation and publishes any diagnostics that are reported.
    /// It does this for all configuration sets.
    pub async fn compile_and_publish_diagnostics(&self) {
        let mut session_guard = self.session.lock().await;
        let Session { configuration_sets, server_config } = session_guard.deref_mut();

        self.client
            .log_message(
                MessageType::INFO,
                "Publishing diagnostics for all configuration sets.",
            )
            .await;
        for configuration_set in configuration_sets.iter_mut() {
            // Trigger a compilation and get any diagnostics that were reported during it.
            let diagnostics = configuration_set.trigger_compilation(server_config);
            // Publish those diagnostics.
            publish_diagnostics_for_set(&self.client, diagnostics, configuration_set).await;
        }
    } // TODO

    async fn warn(&self, message: impl std::fmt::Display) {
        self.client.log_message(MessageType::WARNING, message).await;
    }

    async fn log(&self, message: impl std::fmt::Display) {
        self.client.log_message(MessageType::LOG, message).await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(
        &self,
        params: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        let mut session_guard = self.session.lock().await;
        session_guard.update_from_initialize_params(params);

        let capabilities = Backend::capabilities();
        Ok(InitializeResult {
            capabilities,
            ..InitializeResult::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.log("Server initialized!").await;

        // Now that the server and client are fully initialized, it's safe to compile and publish any diagnostics.
        self.compile_and_publish_diagnostics().await;
    }

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        Ok(())
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        self.log("Detected change in server configuration.").await;

        // Explicit scope to ensure the session lock guard is dropped before we start compilation.
        {
            let mut session_guard = self.session.lock().await;

            // When the configuration changes, any of the files in the workspace could be impacted.
            // Therefore, we need to clear the diagnostics for all files and then re-publish them.
            self.log("  clearing diagnostics.").await;
            clear_diagnostics(&self.client, &session_guard.configuration_sets).await;

            // Update the stored configuration sets from the data provided in the client notification
            self.log(format!("Received json params:\n{params:?}")).await;
            session_guard.update_configurations_from_params(params);
            self.log(format!("Parsed config:\n{:?}", session_guard.configuration_sets)).await;
        }

        // Trigger a compilation and publish the diagnostics for all files
        self.compile_and_publish_diagnostics().await;
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        self.log(format!("GotoDef in file '{uri}' @ {pos:?}")).await;

        // Convert the URI to a file path and back to a URL to ensure that the URI is formatted correctly for Windows.
        let Some(file_path) = url_to_sanitized_file_path(&uri) else {
            self.warn(format!("  unsupported uri: '{uri}'")).await;
            return Ok(None);
        };

        // Find the configuration set that contains the file.
        let session_guard = self.session.lock().await;
        for configuration_set in &session_guard.configuration_sets {
            let files = &configuration_set.compilation_data.files;
            if let Some(file) = files.get(&file_path) {
                // Get the definition's span and convert it to a 'GotoDefinitionResponse'.
                let definition_span = get_definition_span(file, pos);
                match &definition_span {
                    Some(span) => self.log(format!("  definition found at: '{span:?}'")).await,
                    None => self.log(format!("  no definition at: '{pos:?}'")).await,
                }
                let response = definition_span.map(|span| {
                    let range = span_to_range(span);
                    GotoDefinitionResponse::Scalar(Location { uri, range })
                });
                return Ok(response);
            }
        }

        // Reaching here means the file wasn't in any of the configuration sets.
        self.log(format!("  file is not tracked: '{uri}'")).await;
        Ok(None)
    }

    async fn hover(&self, params: HoverParams) -> tower_lsp::jsonrpc::Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        self.log(format!("Hover in file '{uri}' @ {pos:?}")).await;

        // Convert the URI to a file path and back to a URL to ensure that the URI is formatted correctly for Windows.
        let Some(file_path) = url_to_sanitized_file_path(&uri) else {
            self.warn(format!("  unsupported uri: '{uri}'")).await;
            return Ok(None);
        };

        // Find the configuration set that contains the file and get the hover info.
        let session_guard = self.session.lock().await;
        for configuration_set in &session_guard.configuration_sets {
            let files = &configuration_set.compilation_data.files;
            if let Some(file) = files.get(&file_path) {
                let message = get_hover_message(file, pos);
                match &message {
                    Some(s) => self.log(format!("  emitting message: '{s}'")).await,
                    None => self.log("  no message found").await,
                }
                let response = message.map(|s| {
                    let contents = HoverContents::Scalar(MarkedString::String(s));
                    // TODO we should emit the range that the `slicec` gave us.
                    Hover { contents, range: None }
                });
                return Ok(response);
            }
        }

        // Reaching here means the file wasn't in any of the configuration sets.
        self.log(format!("  file is not tracked: '{uri}'")).await;
        Ok(None)
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = &params.text_document.uri;
        self.log(format!("File '{uri}' was opened")).await;
        self.handle_file_change(uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = &params.text_document.uri;
        self.log(format!("File '{uri}' was saved")).await;
        self.handle_file_change(uri).await;
    }
}

pub async fn show_popup(
    client: &Client,
    message: String,
    message_type: notifications::MessageType,
) {
    client
        .send_notification::<ShowNotification>(ShowNotificationParams {
            message,
            message_type,
        })
        .await;
}
