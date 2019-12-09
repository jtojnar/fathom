//! Elaboration from the surface syntax into the core syntax.
//!
//! Performs the following:
//!
//! - name resolution
//! - desugaring
//! - pattern compilation (TODO)
//! - bidirectional type checking (TODO)
//! - unification (TODO)

use codespan::{FileId, Span};
use codespan_reporting::diagnostic::{Diagnostic, Severity};
use num_bigint::BigInt;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{core, diagnostics, surface};

/// Elaborate a module in the surface syntax into the core syntax.
pub fn elaborate_module(
    surface_module: &surface::Module,
    report: &mut dyn FnMut(Diagnostic),
) -> core::Module {
    let item_context = ItemContext::new(surface_module.file_id);
    core::Module {
        file_id: surface_module.file_id,
        doc: surface_module.doc.clone(),
        items: elaborate_items(item_context, &surface_module.items, report),
    }
}

/// Contextual information to be used when elaborating items.
pub struct ItemContext {
    /// The file where these items are defined (for error reporting).
    file_id: FileId,
    /// Labels that have previously been used for items, along with the span
    /// where they were introduced (for error reporting).
    items: HashMap<core::Label, (Span, core::Value)>,
}

impl ItemContext {
    /// Create a new item context.
    pub fn new(file_id: FileId) -> ItemContext {
        ItemContext {
            file_id,
            items: HashMap::new(),
        }
    }

    /// Create a field context based on this item context.
    pub fn field_context(&self) -> FieldContext<'_> {
        FieldContext::new(self.file_id, &self.items)
    }

    /// Create a term context based on this item context.
    pub fn term_context(&self) -> TermContext<'_> {
        TermContext::new(self.file_id, &self.items)
    }
}

/// Elaborate items in the surface syntax into items in the core syntax.
pub fn elaborate_items(
    mut context: ItemContext,
    surface_items: &[surface::Item],
    report: &mut dyn FnMut(Diagnostic),
) -> Vec<core::Item> {
    let mut core_items = Vec::new();

    for item in surface_items.iter() {
        use std::collections::hash_map::Entry;

        match item {
            surface::Item::Alias(alias) => {
                let label = core::Label(alias.name.1.clone());
                let (core_term, ty) = match &alias.ty {
                    Some(surface_ty) => {
                        let context = context.term_context();
                        let core_ty = elaborate_universe(&context, surface_ty, report);
                        let ty = core::semantics::eval(&core_ty);
                        let core_term = check_term(&context, &alias.term, &ty, report);
                        (core::Term::Ann(Arc::new(core_term), Arc::new(core_ty)), ty)
                    }
                    None => synth_term(&context.term_context(), &alias.term, report),
                };

                match context.items.entry(label) {
                    Entry::Vacant(entry) => {
                        let item = core::Alias {
                            span: alias.span,
                            doc: alias.doc.clone(),
                            name: entry.key().clone(),
                            term: core_term,
                        };

                        core_items.push(core::Item::Alias(item));
                        entry.insert((alias.span, ty));
                    }
                    Entry::Occupied(entry) => report(diagnostics::item_redefinition(
                        Severity::Error,
                        context.file_id,
                        entry.key(),
                        alias.span,
                        entry.get().0,
                    )),
                }
            }
            surface::Item::Struct(struct_ty) => {
                let label = core::Label(struct_ty.name.1.clone());
                let field_context = context.field_context();
                let core_fields =
                    elaborate_struct_ty_fields(field_context, &struct_ty.fields, report);

                match context.items.entry(label) {
                    Entry::Vacant(entry) => {
                        let item = core::StructType {
                            span: struct_ty.span,
                            doc: struct_ty.doc.clone(),
                            name: entry.key().clone(),
                            fields: core_fields,
                        };

                        core_items.push(core::Item::Struct(item));
                        entry.insert((
                            struct_ty.span,
                            core::Value::Universe(core::Universe::Format),
                        ));
                    }
                    Entry::Occupied(entry) => report(diagnostics::item_redefinition(
                        Severity::Error,
                        context.file_id,
                        entry.key(),
                        struct_ty.span,
                        entry.get().0,
                    )),
                }
            }
        }
    }

    core_items
}

/// Contextual information to be used when elaborating structure type fields.
pub struct FieldContext<'items> {
    /// The file where these fields are defined (for error reporting).
    file_id: FileId,
    /// Previously elaborated items.
    items: &'items HashMap<core::Label, (Span, core::Value)>,
    /// Labels that have previously been used for fields, along with the span
    /// where they were introduced (for error reporting).
    fields: HashMap<core::Label, Span>,
}

impl<'items> FieldContext<'items> {
    /// Create a new field context.
    pub fn new(
        file_id: FileId,
        items: &'items HashMap<core::Label, (Span, core::Value)>,
    ) -> FieldContext<'items> {
        FieldContext {
            file_id,
            fields: HashMap::new(),
            items,
        }
    }

    /// Create a term context based on this field context.
    pub fn term_context(&self) -> TermContext<'_> {
        TermContext::new(self.file_id, self.items)
    }
}

/// Elaborate structure type fields in the surface syntax into structure type
/// fields in the core syntax.
pub fn elaborate_struct_ty_fields(
    mut context: FieldContext<'_>,
    surface_fields: &[surface::TypeField],
    report: &mut dyn FnMut(Diagnostic),
) -> Vec<core::TypeField> {
    let mut core_fields = Vec::with_capacity(surface_fields.len());

    for field in surface_fields {
        use std::collections::hash_map::Entry;

        let label = core::Label(field.name.1.clone());
        let field_span = Span::merge(field.name.0, field.term.span());
        let ty = check_term(
            &context.term_context(),
            &field.term,
            &core::Value::Universe(core::Universe::Format),
            report,
        );

        match context.fields.entry(label) {
            Entry::Vacant(entry) => {
                core_fields.push(core::TypeField {
                    doc: field.doc.clone(),
                    start: field_span.start(),
                    name: entry.key().clone(),
                    term: ty,
                });

                entry.insert(field_span);
            }
            Entry::Occupied(entry) => report(diagnostics::field_redeclaration(
                Severity::Error,
                context.file_id,
                entry.key(),
                field_span,
                *entry.get(),
            )),
        }
    }

    core_fields
}

/// Contextual information to be used when elaborating terms.
pub struct TermContext<'items> {
    /// The file where this term is located (for error reporting).
    file_id: FileId,
    /// Previously elaborated items.
    items: &'items HashMap<core::Label, (Span, core::Value)>,
}

impl<'items> TermContext<'items> {
    /// Create a new term context.
    pub fn new(
        file_id: FileId,
        items: &'items HashMap<core::Label, (Span, core::Value)>,
    ) -> TermContext<'items> {
        TermContext { file_id, items }
    }
}

/// Check that a surface term is a type or kind, and elaborate it into the core syntax.
pub fn elaborate_universe(
    context: &TermContext<'_>,
    surface_term: &surface::Term,
    report: &mut dyn FnMut(Diagnostic),
) -> core::Term {
    match surface_term {
        surface::Term::Name(span, name)
            if !context.items.contains_key("Type") && name.as_str() == "Type" =>
        {
            core::Term::Universe(*span, core::Universe::Type)
        }
        surface::Term::Name(span, name)
            if !context.items.contains_key("Format") && name.as_str() == "Format" =>
        {
            core::Term::Universe(*span, core::Universe::Format)
        }
        surface::Term::Name(span, name)
            if !context.items.contains_key("Kind") && name.as_str() == "Kind" =>
        {
            core::Term::Universe(*span, core::Universe::Kind)
        }
        surface_term => match synth_term(context, surface_term, report) {
            (core_term, core::Value::Universe(_)) | (core_term, core::Value::Error) => core_term,
            (_, ty) => {
                let span = surface_term.span();
                report(diagnostics::universe_mismatch(
                    Severity::Error,
                    context.file_id,
                    span,
                    &ty,
                ));
                core::Term::Error(span)
            }
        },
    }
}

/// Check a surface term against the given type, and elaborate it into the core syntax.
pub fn check_term(
    context: &TermContext<'_>,
    surface_term: &surface::Term,
    expected_ty: &core::Value,
    report: &mut dyn FnMut(Diagnostic),
) -> core::Term {
    match (surface_term, expected_ty) {
        (surface::Term::Error(span), _) => core::Term::Error(*span),
        (surface_term, core::Value::Error) => core::Term::Error(surface_term.span()),
        (surface::Term::Paren(_, surface_term), expected_ty) => {
            check_term(context, surface_term, expected_ty, report)
        }
        (surface::Term::NumberLiteral(span, literal), _) => {
            let error = |report: &mut dyn FnMut(Diagnostic)| {
                report(diagnostics::error::numeric_literal_not_supported(
                    context.file_id,
                    *span,
                    expected_ty,
                ));
                core::Term::Error(surface_term.span())
            };
            match expected_ty {
                core::Value::Neutral(core::Head::Item(label), elims) if elims.is_empty() => {
                    match label.0.as_str() {
                        "Int" => match literal.parse_big_int(context.file_id, report) {
                            Some(value) => core::Term::Constant(*span, core::Constant::Int(value)),
                            None => core::Term::Error(*span),
                        },
                        "F32" => match literal.parse_float(context.file_id, report) {
                            Some(value) => core::Term::Constant(*span, core::Constant::F32(value)),
                            None => core::Term::Error(*span),
                        },
                        "F64" => match literal.parse_float(context.file_id, report) {
                            Some(value) => core::Term::Constant(*span, core::Constant::F64(value)),
                            None => core::Term::Error(*span),
                        },
                        _ => error(report),
                    }
                }
                _ => error(report),
            }
        }
        (surface::Term::If(span, surface_head, surface_if_true, surface_if_false), _) => {
            let bool_ty =
                core::Value::Neutral(core::Head::Item(core::Label("Bool".to_owned())), Vec::new());
            let head = check_term(context, surface_head, &bool_ty, report);
            let if_true = check_term(context, surface_if_true, expected_ty, report);
            let if_false = check_term(context, surface_if_false, expected_ty, report);

            core::Term::BoolElim(*span, Arc::new(head), Arc::new(if_true), Arc::new(if_false))
        }
        (surface::Term::Match(span, surface_head, surface_branches), _) => {
            let (head, head_ty) = synth_term(context, surface_head, report);
            let error = |report: &mut dyn FnMut(Diagnostic)| {
                report(diagnostics::error::unsupported_pattern_ty(
                    context.file_id,
                    surface_head.span(),
                    &head_ty,
                ));
                core::Term::Error(*span)
            };

            match &head_ty {
                core::Value::Neutral(core::Head::Item(label), elims) if elims.is_empty() => {
                    match label.0.as_str() {
                        "Bool" => {
                            let (if_true, if_false) =
                                check_bool_branches(context, surface_branches, expected_ty, report);
                            core::Term::BoolElim(*span, Arc::new(head), if_true, if_false)
                        }
                        "Int" => {
                            let (branches, default) = check_int_branches(
                                context,
                                surface_head.span(),
                                surface_branches,
                                expected_ty,
                                report,
                            );
                            core::Term::IntElim(*span, Arc::new(head), branches, default)
                        }
                        _ => error(report),
                    }
                }
                core::Value::Error => core::Term::Error(*span),
                _ => error(report),
            }
        }
        (surface_term, expected_ty) => {
            let (core_term, synth_ty) = synth_term(context, surface_term, report);

            if core::semantics::equal(&synth_ty, expected_ty) {
                core_term
            } else {
                report(diagnostics::type_mismatch(
                    Severity::Error,
                    context.file_id,
                    surface_term.span(),
                    expected_ty,
                    &synth_ty,
                ));
                core::Term::Error(surface_term.span())
            }
        }
    }
}

/// Synthesize the type of a surface term, and elaborate it into the core syntax.
pub fn synth_term(
    context: &TermContext<'_>,
    surface_term: &surface::Term,
    report: &mut dyn FnMut(Diagnostic),
) -> (core::Term, core::Value) {
    use crate::core::Universe::{Format, Kind, Type};

    match surface_term {
        surface::Term::Paren(_, surface_term) => synth_term(context, surface_term, report),
        surface::Term::Ann(surface_term, surface_ty) => {
            let core_ty = elaborate_universe(context, surface_ty, report);
            let ty = core::semantics::eval(&core_ty);
            let core_term = check_term(context, surface_term, &ty, report);
            (core::Term::Ann(Arc::new(core_term), Arc::new(core_ty)), ty)
        }
        surface::Term::Name(span, name) => match context.items.get(name.as_str()) {
            Some((_, ty)) => (
                core::Term::Item(*span, core::Label(name.to_string())),
                ty.clone(),
            ),
            None => match name.as_str() {
                "Kind" => {
                    report(diagnostics::kind_has_no_type(
                        Severity::Error,
                        context.file_id,
                        *span,
                    ));
                    (core::Term::Error(*span), core::Value::Error)
                }
                "Type" => (
                    core::Term::Universe(*span, Type),
                    core::Value::Universe(Kind),
                ),
                "Format" => (
                    core::Term::Universe(*span, Format),
                    core::Value::Universe(Kind),
                ),
                "U8" | "U16Le" | "U16Be" | "U32Le" | "U32Be" | "U64Le" | "U64Be" | "S8"
                | "S16Le" | "S16Be" | "S32Le" | "S32Be" | "S64Le" | "S64Be" | "F32Le" | "F32Be"
                | "F64Le" | "F64Be" => (
                    core::Term::Item(*span, core::Label(name.to_string())),
                    core::Value::Universe(Format),
                ),
                "Bool" | "Int" | "F32" | "F64" => (
                    core::Term::Item(*span, core::Label(name.to_string())),
                    core::Value::Universe(Type),
                ),
                "true" => (
                    core::Term::Constant(*span, core::Constant::Bool(true)),
                    core::Value::Neutral(
                        core::Head::Item(core::Label("Bool".to_owned())),
                        Vec::new(),
                    ),
                ),
                "false" => (
                    core::Term::Constant(*span, core::Constant::Bool(false)),
                    core::Value::Neutral(
                        core::Head::Item(core::Label("Bool".to_owned())),
                        Vec::new(),
                    ),
                ),
                _ => {
                    report(diagnostics::error::var_name_not_found(
                        context.file_id,
                        name.as_str(),
                        *span,
                    ));

                    (core::Term::Error(*span), core::Value::Error)
                }
            },
        },
        surface::Term::NumberLiteral(span, _) => {
            report(diagnostics::error::ambiguous_numeric_literal(
                context.file_id,
                *span,
            ));

            (core::Term::Error(*span), core::Value::Error)
        }
        surface::Term::If(span, surface_head, surface_if_true, surface_if_false) => {
            let bool_ty =
                core::Value::Neutral(core::Head::Item(core::Label("Bool".to_owned())), Vec::new());
            let head = check_term(context, surface_head, &bool_ty, report);
            let (if_true, if_true_ty) = synth_term(context, surface_if_true, report);
            let (if_false, if_false_ty) = synth_term(context, surface_if_false, report);

            if core::semantics::equal(&if_true_ty, &if_false_ty) {
                (
                    core::Term::BoolElim(
                        *span,
                        Arc::new(head),
                        Arc::new(if_true),
                        Arc::new(if_false),
                    ),
                    if_true_ty,
                )
            } else {
                report(diagnostics::type_mismatch(
                    Severity::Error,
                    context.file_id,
                    surface_if_false.span(),
                    &if_true_ty,
                    &if_false_ty,
                ));
                (core::Term::Error(*span), core::Value::Error)
            }
        }
        surface::Term::Match(span, _, _) => {
            report(diagnostics::ambiguous_match_expression(
                Severity::Error,
                context.file_id,
                *span,
            ));
            (core::Term::Error(*span), core::Value::Error)
        }
        surface::Term::Error(span) => (core::Term::Error(*span), core::Value::Error),
    }
}

#[allow(unused_variables)]
fn check_bool_branches(
    context: &TermContext<'_>,
    surface_branches: &[(surface::Pattern, surface::Term)],
    expected_ty: &core::Value,
    report: &mut dyn FnMut(Diagnostic),
) -> (Arc<core::Term>, Arc<core::Term>) {
    unimplemented!("boolean eliminators")
}

fn check_int_branches(
    context: &TermContext<'_>,
    span: Span,
    surface_branches: &[(surface::Pattern, surface::Term)],
    expected_ty: &core::Value,
    report: &mut dyn FnMut(Diagnostic),
) -> (BTreeMap<BigInt, Arc<core::Term>>, Arc<core::Term>) {
    use std::collections::btree_map::Entry;

    let mut branches = BTreeMap::new();
    let mut default = None;

    for (pattern, surface_term) in surface_branches {
        match pattern {
            surface::Pattern::NumberLiteral(span, literal) => {
                let core_term = check_term(context, surface_term, expected_ty, report);
                if let Some(value) = literal.parse_big_int(context.file_id, report) {
                    match &default {
                        None => match branches.entry(value) {
                            Entry::Occupied(_) => report(
                                diagnostics::warning::unreachable_pattern(context.file_id, *span),
                            ),
                            Entry::Vacant(entry) => drop(entry.insert(Arc::new(core_term))),
                        },
                        Some(_) => report(diagnostics::warning::unreachable_pattern(
                            context.file_id,
                            *span,
                        )),
                    }
                }
            }
            surface::Pattern::Name(span, _name) => {
                // TODO: check if name is bound
                // - if so compare for equality
                // - otherwise bind local variable
                let core_term = check_term(context, surface_term, expected_ty, report);
                match &default {
                    None => default = Some(Arc::new(core_term)),
                    Some(_) => report(diagnostics::warning::unreachable_pattern(
                        context.file_id,
                        *span,
                    )),
                }
            }
        }
    }

    let default = default.unwrap_or_else(|| {
        report(diagnostics::error::no_default_pattern(
            context.file_id,
            span,
        ));
        Arc::new(core::Term::Error(Span::initial()))
    });

    (branches, default)
}
