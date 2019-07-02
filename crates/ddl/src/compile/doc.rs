use codespan_reporting::diagnostic::Diagnostic;
use std::io;
use std::io::prelude::*;

use crate::core;

pub fn compile_module(
    writer: &mut impl Write,
    module: &core::Module,
) -> io::Result<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    let pkg_name = env!("CARGO_PKG_NAME");
    let pkg_version = env!("CARGO_PKG_VERSION");

    writeln!(writer, "<!--")?;
    writeln!(
        writer,
        "  This file is automatically @generated by {} {}",
        pkg_name, pkg_version,
    )?;
    writeln!(writer, "  It is not intended for manual editing.")?;
    writeln!(writer, "-->")?;

    for item in &module.items {
        match item {
            core::Item::Struct(struct_ty) => {
                writeln!(writer)?;
                compile_struct_item(writer, struct_ty, &mut diagnostics)?;
            }
        }
    }

    Ok(diagnostics)
}

fn compile_struct_item(
    writer: &mut impl Write,
    struct_ty: &core::StructType,
    diagnostics: &mut Vec<Diagnostic>,
) -> io::Result<()> {
    writeln!(writer, "## {}", struct_ty.name)?;

    if !struct_ty.doc.is_empty() {
        writeln!(writer)?;
        // TODO: Bump inner heading levels
        writeln!(writer, "{}", struct_ty.doc)?;
    }

    if !struct_ty.fields.is_empty() {
        writeln!(writer)?;
        writeln!(writer, "### Fields")?;
        writeln!(writer)?;

        if struct_ty.fields.iter().all(|field| field.doc.is_empty()) {
            writeln!(writer, "| Name | Type |")?;
            writeln!(writer, "| ---- | ---- |")?;

            for field in &struct_ty.fields {
                let ty = compile_ty(&field.term, diagnostics);
                writeln!(writer, "| {} | {} |", field.name, ty)?;
            }
        } else {
            writeln!(writer, "| Name | Type | Description |")?;
            writeln!(writer, "| ---- | ---- | ------------|")?;

            for field in &struct_ty.fields {
                let desc = compile_field_description(&field.doc);
                let ty = compile_ty(&field.term, diagnostics);
                writeln!(writer, "| {} | {} | {} |", field.name, ty, desc)?;
            }

            // TODO: output long-form field docs
        }
    }

    Ok(())
}

fn compile_field_description(doc: &str) -> String {
    let mut lines = doc.lines();
    match lines.next() {
        None => "".to_owned(),
        Some(first_line) => match lines.next() {
            None => first_line.trim_end_matches('.').to_owned(),
            // TODO: link ellipsis to long-form field docs
            Some(_) => format!("{}...", first_line.trim_end_matches('.')),
        },
    }
}

fn compile_ty(term: &core::Term, _diagnostics: &mut Vec<Diagnostic>) -> String {
    match term {
        core::Term::U8(_) => "U8".to_owned(),
        core::Term::U16Le(_) => "U16Le".to_owned(),
        core::Term::U16Be(_) => "U16Be".to_owned(),
        core::Term::U32Le(_) => "U32Le".to_owned(),
        core::Term::U32Be(_) => "U32Be".to_owned(),
        core::Term::U64Le(_) => "U64Le".to_owned(),
        core::Term::U64Be(_) => "U64Be".to_owned(),
        core::Term::S8(_) => "S8".to_owned(),
        core::Term::S16Le(_) => "S16Le".to_owned(),
        core::Term::S16Be(_) => "S16Be".to_owned(),
        core::Term::S32Le(_) => "S32Le".to_owned(),
        core::Term::S32Be(_) => "S32Be".to_owned(),
        core::Term::S64Le(_) => "S64Le".to_owned(),
        core::Term::S64Be(_) => "S64Be".to_owned(),
        core::Term::F32Le(_) => "F32Le".to_owned(),
        core::Term::F32Be(_) => "F32Be".to_owned(),
        core::Term::F64Le(_) => "F64Le".to_owned(),
        core::Term::F64Be(_) => "F64Be".to_owned(),
        core::Term::Error(_) => "**invalid data description**".to_owned(),
    }
}
