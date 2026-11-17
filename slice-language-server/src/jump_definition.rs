// Copyright (c) ZeroC, Inc.

use crate::utils::position_to_location;
use slicec::grammar::{
    Commentable, CustomType, Entity, Enum, Enumerator, Field, Identifier, Interface, Message, MessageComponent,
    NamedSymbol, Operation, Struct, Symbol, TypeAlias, TypeRef, TypeRefDefinition, Types,
};
use slicec::slice_file::{Location, SliceFile, Span};
use slicec::visitor::Visitor;
use tower_lsp::lsp_types::Position;

pub fn get_definition_span(file: &SliceFile, position: Position) -> Option<Span> {
    let mut visitor = JumpVisitor::new(position_to_location(position));
    file.visit_with(&mut visitor);

    visitor.found_span
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
            if let Some(overview) = &comment.overview {
                self.check_message_links(overview)
            }
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
        }
    }

    // This function checks to see if the search location is within the span of the link
    fn check_message_links(&mut self, message: &Message) {
        for component in &message.value {
            if let MessageComponent::Link(l) = component {
                self.check_and_set_span(l.linked_entity(), l.span());
            }
        }
    }

    // This function checks to see if the search location is within the span of the entity
    // and if it is, it sets the found_span to the span of the entity
    fn check_and_set_span<T: Entity + ?Sized>(&mut self, linked_entity_result: Result<&T, &Identifier>, span: &Span) {
        if let Ok(entity) = linked_entity_result {
            if self.search_location.is_within(span) {
                self.found_span = Some(entity.raw_identifier().span().clone());
            };
        }
    }
}

impl Visitor for JumpVisitor {
    fn visit_struct(&mut self, struct_def: &Struct) {
        self.check_comment(struct_def);
    }

    fn visit_interface(&mut self, interface_def: &Interface) {
        self.check_comment(interface_def);
        for base_ref in &interface_def.bases {
            if self.search_location.is_within(&base_ref.span) {
                let TypeRefDefinition::Patched(type_def) = &base_ref.definition else {
                    continue;
                };
                self.found_span = Some(type_def.borrow().raw_identifier().span().clone());
            };
        }
    }

    fn visit_enum(&mut self, enum_def: &Enum) {
        self.check_comment(enum_def);
    }

    fn visit_operation(&mut self, operation_def: &Operation) {
        self.check_comment(operation_def);
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
                Types::Enum(x) => Some(x),
                Types::CustomType(x) => Some(x),
                _ => None,
            };
            self.found_span = entity_def.map(|e| e.raw_identifier().span().clone());
        }
    }
}
