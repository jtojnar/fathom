use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::io::prelude::*;

use crate::lang::surface::{
    Constant, ItemData, Module, Pattern, PatternData, StructType, Term, TermData,
};
use crate::pass::surface_to_pretty::Prec;

#[allow(clippy::write_literal)]
pub fn from_module(writer: &mut impl Write, module: &Module) -> io::Result<()> {
    let mut context = Context {
        items: HashMap::new(),
    };

    write!(
        writer,
        r##"<!--
  This file is automatically @generated by {pkg_name} {pkg_version}
  It is not intended for manual editing.
-->

<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta http-equiv="X-UA-Compatible" content="ie=edge">
    <title>{module_name}</title>
    <style>
{minireset}

{style}
    </style>
  </head>
  <body>
    <section class="module">
"##,
        pkg_name = env!("CARGO_PKG_NAME"),
        pkg_version = env!("CARGO_PKG_VERSION"),
        module_name = "", // TODO: module name
        minireset = include_str!("./surface_to_doc/minireset.min.css").trim(),
        style = include_str!("./surface_to_doc/style.css").trim(),
    )?;

    if !module.doc.is_empty() {
        writeln!(writer, r##"      <section class="doc">"##)?;
        from_doc_lines(writer, "        ", &module.doc)?;
        writeln!(writer, r##"      </section>"##)?;
    }

    writeln!(writer, r##"      <dl class="items">"##)?;

    for item in &module.items {
        let (name, item) = match &item.data {
            ItemData::Constant(constant) => context.from_constant(writer, constant)?,
            ItemData::StructType(struct_type) => context.from_struct_type(writer, struct_type)?,
        };

        context.items.insert(name, item);
    }

    write!(
        writer,
        r##"      </dl>
    </section>
  </body>
</html>
"##
    )?;

    Ok(())
}

struct Context {
    items: HashMap<String, ItemMeta>,
}

struct ItemMeta {
    id: String,
}

impl Context {
    fn from_constant(
        &self,
        writer: &mut impl Write,
        constant: &Constant,
    ) -> io::Result<(String, ItemMeta)> {
        let id = format!("items[{}]", constant.name.data);

        writeln!(
            writer,
            r##"        <dt id="{id}" class="item constant">"##,
            id = id,
        )?;
        match &constant.type_ {
            None => writeln!(
                writer,
                r##"          <a href="#{id}">{name}</a>"##,
                id = id,
                name = constant.name.data,
            )?,
            Some(r#type) => writeln!(
                writer,
                r##"          const <a href="#{id}">{name}</a> : {type_}"##,
                id = id,
                name = constant.name.data,
                type_ = self.from_term_prec(r#type, Prec::Term),
            )?,
        }
        write!(
            writer,
            r##"        </dt>
        <dd class="item constant">
"##
        )?;

        if !constant.doc.is_empty() {
            writeln!(writer, r##"          <section class="doc">"##)?;
            from_doc_lines(writer, "            ", &constant.doc)?;
            writeln!(writer, r##"          </section>"##)?;
        }

        let term = self.from_term_prec(&constant.term, Prec::Term);

        write!(
            writer,
            r##"          <section class="term">
            {}
          </section>
        </dd>
"##,
            term
        )?;

        Ok((constant.name.data.clone(), ItemMeta { id }))
    }

    fn from_struct_type(
        &self,
        writer: &mut impl Write,
        struct_type: &StructType,
    ) -> io::Result<(String, ItemMeta)> {
        let id = format!("items[{}]", struct_type.name.data);

        writeln!(
            writer,
            r##"        <dt id="{id}" class="item struct">"##,
            id = id
        )?;
        match &struct_type.type_ {
            None => writeln!(
                writer,
                r##"          struct <a href="#{id}">{name}</a>"##,
                id = id,
                name = struct_type.name.data,
            )?,
            Some(r#type) => writeln!(
                writer,
                r##"          struct <a href="#{id}">{name}</a> : {type_}"##,
                id = id,
                name = struct_type.name.data,
                type_ = self.from_term_prec(&r#type, Prec::Term),
            )?,
        }

        writeln!(writer, r##"        </dt>"##)?;
        writeln!(writer, r##"        <dd class="item struct">"##)?;

        if !struct_type.doc.is_empty() {
            writeln!(writer, r##"          <section class="doc">"##)?;
            from_doc_lines(writer, "            ", &struct_type.doc)?;
            writeln!(writer, r##"          </section>"##)?;
        }

        if !struct_type.fields.is_empty() {
            writeln!(writer, r##"          <dl class="fields">"##)?;
            for field in &struct_type.fields {
                let field_id = format!("{}.fields[{}]", id, field.label.data);
                let r#type = self.from_term_prec(&field.term, Prec::Term);

                write!(
                    writer,
                    r##"            <dt id="{id}" class="field">
              <a href="#{id}">{name}</a> : {type_}
            </dt>
            <dd class="field">
              <section class="doc">
"##,
                    id = field_id,
                    name = field.label.data,
                    type_ = r#type,
                )?;
                from_doc_lines(writer, "                ", &field.doc)?;
                write!(
                    writer,
                    r##"              </section>
            </dd>
"##
                )?;
            }
            writeln!(writer, r##"          </dl>"##)?;
        }

        writeln!(writer, r##"        </dd>"##)?;

        Ok((struct_type.name.data.clone(), ItemMeta { id }))
    }

    fn from_term_prec<'term>(&self, term: &'term Term, prec: Prec) -> Cow<'term, str> {
        use itertools::Itertools;

        match &term.data {
            TermData::Name(name) => {
                let id = match self.items.get(name) {
                    Some(item) => item.id.as_str(),
                    None => "",
                };

                format!(r##"<var><a href="#{}">{}</a></var>"##, id, name).into()
            }

            TermData::KindType => "Kind".into(),
            TermData::TypeType => "Type".into(),

            TermData::Ann(term, r#type) => format!(
                "{lparen}{term} : {type}{rparen}",
                lparen = if prec > Prec::Term { "(" } else { "" },
                rparen = if prec > Prec::Term { ")" } else { "" },
                term = self.from_term_prec(term, Prec::Arrow),
                type = self.from_term_prec(r#type, Prec::Term),
            )
            .into(),

            TermData::FunctionType(param_type, body_type) => format!(
                "{lparen}{param_type} &rarr; {body_type}{rparen}",
                lparen = if prec > Prec::Arrow { "(" } else { "" },
                rparen = if prec > Prec::Arrow { ")" } else { "" },
                param_type = self.from_term_prec(param_type, Prec::App),
                body_type = self.from_term_prec(body_type, Prec::Arrow),
            )
            .into(),
            TermData::FunctionElim(head, arguments) => format!(
                // TODO: multiline formatting!
                "{lparen}{head} {arguments}{rparen}",
                lparen = if prec > Prec::App { "(" } else { "" },
                rparen = if prec > Prec::App { ")" } else { "" },
                head = self.from_term_prec(head, Prec::Atomic),
                arguments = arguments
                    .iter()
                    .map(|argument| self.from_term_prec(argument, Prec::Atomic))
                    .format(" "),
            )
            .into(),

            TermData::NumberLiteral(literal) => format!("{}", literal).into(),
            TermData::If(head, if_true, if_false) => format!(
                // TODO: multiline formatting!
                "if {head} {{ {if_true} }} else {{ {if_false} }}",
                head = self.from_term_prec(head, Prec::Term),
                if_true = self.from_term_prec(if_true, Prec::Term),
                if_false = self.from_term_prec(if_false, Prec::Term),
            )
            .into(),
            TermData::Match(head, branches) => format!(
                // TODO: multiline formatting!
                "match {head} {{ {branches} }}",
                head = self.from_term_prec(head, Prec::Term),
                branches = branches
                    .iter()
                    .map(|(pattern, term)| format!(
                        "{pattern} &rArr; {term}",
                        pattern = self.from_pattern(pattern),
                        term = self.from_term_prec(term, Prec::Term),
                    ))
                    .format(", "),
            )
            .into(),

            TermData::FormatType => "Format".into(),

            TermData::Repr => "repr".into(),

            TermData::Error => r##"<strong>(invalid data description)</strong>"##.into(),
        }
    }

    fn from_pattern<'term>(&self, pattern: &'term Pattern) -> Cow<'term, str> {
        match &pattern.data {
            PatternData::Name(name) => format!(r##"<a href="#">{}</a>"##, name).into(), // TODO: add local binding
            PatternData::NumberLiteral(literal) => format!("{}", literal).into(),
        }
    }
}

fn from_doc_lines(writer: &mut impl Write, prefix: &str, doc_lines: &[String]) -> io::Result<()> {
    // TODO: parse markdown

    for doc_line in doc_lines.iter() {
        let doc_line = match doc_line {
            line if line.starts_with(' ') => &line[" ".len()..],
            line => &line[..],
        };
        writeln!(writer, "{}{}", prefix, doc_line)?;
    }

    Ok(())
}
