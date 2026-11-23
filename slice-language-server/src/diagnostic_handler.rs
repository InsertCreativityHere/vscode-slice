// Copyright (c) ZeroC, Inc.

use crate::notifications::{self, ShowNotification, ShowNotificationParams};
use crate::slice_project::SliceProject;
use crate::utils::{convert_slice_path_to_uri, span_to_range};

use slicec::diagnostics::{AnnotatedDiagnostic, AnnotatedNote, DiagnosticLevel};
use std::collections::HashMap;
use tower_lsp::lsp_types::{DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString, Url};
use tower_lsp::Client;

/// Publishes diagnostics for all files in the provided project.
///
/// This function takes a client and a project, generates updated diagnostics,
/// and then publishes these diagnostics to the LSP client.
pub async fn publish_diagnostics_for_project(
    client_handle: &Client,
    diagnostics: Vec<AnnotatedDiagnostic>,
    project: &SliceProject,
) {
    // Initialize a map to hold diagnostics grouped by file (URL)
    let mut map = project
        .compilation_data
        .files
        .keys()
        .filter_map(|uri| Some((convert_slice_path_to_uri(uri)?, vec![])))
        .collect::<HashMap<Url, Vec<tower_lsp::lsp_types::Diagnostic>>>();

    // Process the diagnostics and populate the map.
    let popups = process_diagnostics(diagnostics, &mut map);
    for popup in popups {
        client_handle.send_notification::<ShowNotification>(popup).await;
    }

    // Publish the diagnostics for each file
    for (uri, lsp_diagnostics) in map {
        client_handle.publish_diagnostics(uri, lsp_diagnostics, None).await;
    }
}

/// Processes a list of diagnostics and updates the publish map with LSP-compatible diagnostics.
///
/// Allowed lints are dropped. Diagnostics that do not have a span are returned as popup notifications instead.
pub fn process_diagnostics(
    diagnostics: Vec<AnnotatedDiagnostic>,
    publish_map: &mut HashMap<Url, Vec<tower_lsp::lsp_types::Diagnostic>>,
) -> Vec<ShowNotificationParams> {
    let mut popups = Vec::new();
    for diagnostic in diagnostics {
        let (severity, message_type) = match diagnostic.level {
            DiagnosticLevel::Error => (DiagnosticSeverity::ERROR, notifications::MessageType::Error),
            DiagnosticLevel::Warning => (DiagnosticSeverity::WARNING, notifications::MessageType::Warning),
            DiagnosticLevel::Info => (DiagnosticSeverity::INFORMATION, notifications::MessageType::Info),
            DiagnosticLevel::Allowed => continue,
        };

        let Some(span) = diagnostic.snippet.as_ref().map(|snippet| snippet.span.clone()) else {
            popups.push(ShowNotificationParams {
                message: diagnostic.message,
                message_type,
            });
            continue;
        };
        let Some(uri) = convert_slice_path_to_uri(&span.file) else {
            continue;
        };

        let related_information = diagnostic
            .notes
            .iter()
            .filter_map(try_into_lsp_diagnostic_related_information)
            .collect();

        let lsp_diagnostic = tower_lsp::lsp_types::Diagnostic {
            range: span_to_range(span),
            severity: Some(severity),
            code: Some(NumberOrString::String(diagnostic.code)),
            code_description: None,
            source: Some("slicec".to_owned()),
            message: diagnostic.message,
            related_information: Some(related_information),
            tags: None,
            data: None,
        };
        publish_map.entry(uri).or_default().push(lsp_diagnostic);
    }
    popups
}

// A helper function that converts a slicec note into an lsp diagnostic related information
fn try_into_lsp_diagnostic_related_information(
    note: &AnnotatedNote,
) -> Option<tower_lsp::lsp_types::DiagnosticRelatedInformation> {
    let span = note.snippet.as_ref()?.span.clone();
    let uri = convert_slice_path_to_uri(&span.file)?;
    let range = span_to_range(span);

    Some(DiagnosticRelatedInformation {
        location: Location { uri, range },
        message: note.message.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::MessageType;
    use slicec::diagnostics::Snippet;
    use slicec::slice_file::Span;

    fn diagnostic(level: DiagnosticLevel, snippet: Option<Snippet>) -> AnnotatedDiagnostic {
        AnnotatedDiagnostic {
            message: "test message".to_owned(),
            level,
            code: "Test".to_owned(),
            snippet,
            notes: vec![],
            reported_by: vec![],
        }
    }

    #[test]
    fn lint_is_published_unless_allowed() {
        // Arrange
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Test.slice");
        let span = Span::new((3, 5).into(), (3, 23).into(), file);
        let snippet = Snippet {
            span,
            text: String::new(),
        };
        let diagnostics = vec![
            diagnostic(DiagnosticLevel::Warning, Some(snippet.clone())),
            diagnostic(DiagnosticLevel::Allowed, Some(snippet)),
        ];
        let mut publish_map = HashMap::new();

        // Act
        process_diagnostics(diagnostics, &mut publish_map);

        // Assert
        let uri = Url::from_file_path(file).unwrap();
        let severities = publish_map[&uri].iter().map(|d| d.severity).collect::<Vec<_>>();
        assert_eq!(severities, [Some(DiagnosticSeverity::WARNING)]);
    }

    #[test]
    fn diagnostic_without_span_becomes_popup_of_its_level() {
        // Arrange
        let diagnostics = vec![diagnostic(DiagnosticLevel::Warning, None)];
        let mut publish_map = HashMap::new();

        // Act
        let popups = process_diagnostics(diagnostics, &mut publish_map);

        // Assert
        let [popup] = popups.as_slice() else { panic!("expected one popup, got {popups:?}") };
        assert!(matches!(popup.message_type, MessageType::Warning));
    }
}
