// Copyright (c) ZeroC, Inc.

use slicec::{
    compilation_state::CompilationState,
    grammar::{
        Class, Commentable, CustomType, Entity, Enum, Enumerator, Exception, Field, Identifier,
        Interface, MessageComponent, Module, NamedSymbol, Operation, Parameter, Struct, Symbol,
        TypeAlias, TypeRef, TypeRefDefinition, Types,
    },
    slice_file::{Location, SliceFile, Span},
    visitor::Visitor,
};
use tower_lsp::lsp_types::{Position, Url};

pub fn get_definition_span(state: &CompilationState, uri: Url, position: Position) -> (Option<Span>, String) {
    let mut log_message = String::new();
    log_message.push_str(&format!("This is the uri that we're starting with: {uri:?}\n"));

    let file_path_part = uri.to_file_path().ok();
    log_message.push_str(&format!("    We make it a file path: {file_path_part:?}\n"));

    let file_path = file_path_part.and_then(|x| x.to_str().map(|y| y.to_owned()));
    log_message.push_str(&format!("    And finally stringify it: {file_path:?}\n"));

    let file = match &file_path {
        Some(path) => {
            log_message.push_str(&format!("\nStarting the search...\n"));
            match state.files.get(path) {
                Some(worked) => {
                    log_message.push_str(&format!("We found match!!! [search_path='{path:?}'] and [found_path='{}']\n", worked.relative_path));
                    Some(worked)
                }
                None => {
                    log_message.push_str(&format!("We didn't find a matching file...\n"));
                    None
                }
            }
        }
        None => {
            log_message.push_str(&format!("\nFile path didn't even exist... Nothing to look up\n"));
            None
        }
    };

    if let Some(thing) = file {
        log_message.push_str(&format!("\nJUMPING TO THE FILE HOORAY!!!\n"));

        // Convert position to row and column to 1 based
        let col = (position.character + 1) as usize;
        let row = (position.line + 1) as usize;
        let location = (row, col).into();

        let mut visitor = JumpVisitor::new(location);
        thing.visit_with(&mut visitor);

        (visitor.found_span, log_message)
    } else {
        log_message.push_str(&format!("\nWe are not jumping...\n"));
        (None, log_message)
    }
}

struct JumpVisitor {
    pub search_location: Location,
    pub found_span: Option<Span>,
}

impl JumpVisitor {
    pub fn new(search_location: Location) -> Self {
        JumpVisitor {
            search_location,
            found_span: None,
        }
    }

    // This function checks to see if the search location is within the span of the comment
    // and if it is, it checks to see if the comment contains a link to an entity.
    fn check_comment(&mut self, commentable: &dyn Commentable) {
        if let Some(comment) = commentable.comment() {
            comment
                .overview
                .as_ref()
                .map(|overview| self.check_message_links(&overview.message));
            comment
                .returns
                .iter()
                .for_each(|returns| self.check_message_links(&returns.message));
            comment
                .params
                .iter()
                .for_each(|params| self.check_message_links(&params.message));
            comment
                .see
                .iter()
                .for_each(|s| self.check_and_set_span(s.linked_entity(), s.span()));
            for throws in &comment.throws {
                self.check_message_links(&throws.message);
                self.check_and_set_span(throws.thrown_type(), throws.span());
            }
        }
    }

    // This function checks to see if the search location is within the span of the link
    fn check_message_links(&mut self, message: &Vec<MessageComponent>) {
        for m in message.iter() {
            if let MessageComponent::Link(l) = m {
                self.check_and_set_span(l.linked_entity(), l.span());
            }
        }
    }

    // This function checks to see if the search location is within the span of the entity
    // and if it is, it sets the found_span to the span of the entity
    fn check_and_set_span<T: Entity + ?Sized>(
        &mut self,
        linked_entity_result: Result<&T, &Identifier>,
        span: &Span,
    ) {
        if let Ok(entity) = linked_entity_result {
            if self.search_location.is_within(span) {
                self.found_span = Some(entity.raw_identifier().span().clone());
                return;
            };
        }
    }
}

impl Visitor for JumpVisitor {
    fn visit_file(&mut self, _: &SliceFile) {}

    fn visit_module(&mut self, _: &Module) {}

    fn visit_struct(&mut self, struct_def: &Struct) {
        self.check_comment(struct_def);
    }

    fn visit_class(&mut self, class_def: &Class) {
        self.check_comment(class_def);
        if let Some(base_ref) = &class_def.base {
            if self.search_location.is_within(&base_ref.span) {
                self.found_span = Some(base_ref.definition().raw_identifier().span().clone());
                return;
            }
        }
    }

    fn visit_exception(&mut self, exception_def: &Exception) {
        self.check_comment(exception_def);
        if let Some(base_ref) = &exception_def.base {
            if self.search_location.is_within(&base_ref.span) {
                self.found_span = Some(base_ref.definition().raw_identifier().span().clone());
                return;
            }
        }
    }

    fn visit_interface(&mut self, interface_def: &Interface) {
        self.check_comment(interface_def);
        for base_ref in interface_def.bases.iter() {
            if self.search_location.is_within(&base_ref.span) {
                self.found_span = Some(base_ref.definition().raw_identifier().span().clone());
                return;
            };
        }
    }

    fn visit_enum(&mut self, enum_def: &Enum) {
        self.check_comment(enum_def);
    }

    fn visit_operation(&mut self, operation_def: &Operation) {
        self.check_comment(operation_def);
        for base_ref in operation_def.exception_specification.iter() {
            if self.search_location.is_within(&base_ref.span) {
                self.found_span = Some(base_ref.definition().raw_identifier().span().clone());
                return;
            };
        }
    }

    fn visit_custom_type(&mut self, custom_type_def: &CustomType) {
        self.check_comment(custom_type_def);
    }

    fn visit_type_alias(&mut self, type_alias_def: &TypeAlias) {
        self.check_comment(type_alias_def);
    }

    fn visit_field(&mut self, field_def: &Field) {
        self.check_comment(field_def);
    }

    fn visit_parameter(&mut self, _: &Parameter) {}

    fn visit_enumerator(&mut self, enumerator_def: &Enumerator) {
        self.check_comment(enumerator_def);
    }

    fn visit_type_ref(&mut self, typeref_def: &TypeRef) {
        if self.search_location.is_within(typeref_def.span()) {
            let TypeRefDefinition::Patched(type_def) = &typeref_def.definition else {
                return;
            };
            let entity_def: Option<&dyn Entity> = match type_def.borrow().concrete_type() {
                Types::Struct(x) => Some(x),
                Types::Class(x) => Some(x),
                Types::Interface(x) => Some(x),
                Types::Enum(x) => Some(x),
                Types::CustomType(x) => Some(x),
                _ => None,
            };
            self.found_span = entity_def.map(|e| e.raw_identifier().span().clone());
        }
    }
}
