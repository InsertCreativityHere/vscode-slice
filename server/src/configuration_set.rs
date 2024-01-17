// Copyright (c) ZeroC, Inc.

use crate::slice_config;
use std::collections::HashMap;
use slice_config::{ServerConfig, SliceConfig, compute_slice_options};
use slicec::slice_options;
use slicec::{ast::Ast, diagnostics::Diagnostic, slice_file::SliceFile};
use slicec::compilation_state::CompilationState;

#[derive(Debug)]
pub struct CompilationData {
    pub ast: Ast,
    pub files: HashMap<String, SliceFile>,
}

impl Default for CompilationData {
    fn default() -> Self {
        Self {
            ast: Ast::create(),
            files: HashMap::default(),
        }
    }
}

unsafe impl Send for CompilationData {}
unsafe impl Sync for CompilationData {}

#[derive(Debug)]
pub struct ConfigurationSet {
    pub slice_config: SliceConfig,
    pub compilation_data: CompilationData,
    pub unpublished_diagnostics: Option<Vec<Diagnostic>>,
}

impl ConfigurationSet {
    /// Creates a new `ConfigurationSet`.
    pub fn new() -> Self {
        Self::create_and_compile(SliceConfig::default())
    }

    /// Parses a vector of `ConfigurationSet` from a JSON array, root path, and built-in path.
    pub fn parse_configuration_sets(config_array: &[serde_json::Value]) -> Vec<ConfigurationSet> {
        config_array
            .iter()
            .map(|value| ConfigurationSet::from_json(value))
            .collect::<Vec<_>>()
    }

    /// Constructs a `ConfigurationSet` from a JSON value.
    fn from_json(value: &serde_json::Value) -> Self {
        // Parse the paths and `include_built_in_types` into a `SliceConfig` struct.
        let slice_config = SliceConfig {
            slice_search_paths: parse_paths(value),
            include_built_in_slice_files: parse_include_built_in(value),
        };
        Self::create_and_compile(slice_config)
    }

    fn create_and_compile(slice_config: SliceConfig) -> Self {
        let mut configuration_set = Self {
            slice_config,
            compilation_data: CompilationData::default(),
            unpublished_diagnostics: None,
        };

        configuration_set.unpublished_diagnostics = Some(configuration_set.trigger_compilation());
        configuration_set
    }

    pub fn trigger_compilation(&mut self, server_config: &ServerConfig) -> Vec<Diagnostic> {
        // Perform the compilation.
        let slice_options = &compute_slice_options(server_config, &self.slice_config);
        let compilation_state = slicec::compile_from_options(slice_options, |_| {}, |_| {});
        let CompilationState { ast, diagnostics, files } = compilation_state;

        // Process the diagnostics (filter out allowed lints, and update diagnostic levels as necessary).
        let updated_diagnostics = diagnostics.into_updated(&ast, &files, slice_options);

        // Store the data we got from compiling, then return the diagnostics so they can be published.
        self.compilation_data = CompilationData { ast, files };
        updated_diagnostics
    }
}

/// Parses paths from a JSON value.
fn parse_paths(value: &serde_json::Value) -> Vec<String> {
    value
        .get("paths")
        .and_then(|v| v.as_array())
        .map(|dirs_array| {
            dirs_array
                .iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Determines whether to include built-in types from a JSON value.
fn parse_include_built_in(value: &serde_json::Value) -> bool {
    value
        .get("addWellKnownTypes")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}
